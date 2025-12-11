//! LanceDB backend for Knowledge Graph storage.
//!
//! This backend provides efficient storage and querying for KG nodes and edges
//! using LanceDB's columnar storage format.

use crate::error::{DbError, DbResult};
use crate::kg::entities::{KgEdge, KgNode, KgStats};
use crate::kg::traits::{KgStoreBackend, KgStoreConfig};

use arrow_array::Array;
use arrow_array::{ArrayRef, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{connect, Connection, Table};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::runtime::Runtime;
use tracing::{debug, trace};

// ============================================================================
// Constants
// ============================================================================

/// LanceDB table name for nodes.
const NODES_TABLE_NAME: &str = "kg_nodes";

/// LanceDB table name for edges.
const EDGES_TABLE_NAME: &str = "kg_edges";

// ============================================================================
// LanceDbKgStore
// ============================================================================

/// LanceDB-based KG storage backend.
pub struct LanceDbKgStore {
    /// Path to the store directory.
    #[allow(dead_code)]
    path: PathBuf,

    /// LanceDB connection.
    connection: Connection,

    /// Nodes table (lazily initialized).
    nodes_table: RwLock<Option<Table>>,

    /// Edges table (lazily initialized).
    edges_table: RwLock<Option<Table>>,

    /// Tokio runtime for async operations.
    runtime: Runtime,
}

impl LanceDbKgStore {
    /// Open or create a LanceDB KG store.
    pub fn open(config: &KgStoreConfig) -> DbResult<Self> {
        debug!("Opening LanceDbKgStore at {:?}", config.path);

        // Ensure directory exists
        if !config.path.exists() {
            std::fs::create_dir_all(&config.path)?;
        }

        // Create tokio runtime
        let runtime = Runtime::new()
            .map_err(|e| DbError::internal(format!("Failed to create tokio runtime: {}", e)))?;

        // Connect to LanceDB
        let connection = runtime
            .block_on(async {
                connect(config.path.to_string_lossy().as_ref())
                    .execute()
                    .await
            })
            .map_err(|e| DbError::LanceDb {
                message: format!("Failed to connect: {}", e),
            })?;

        let store = Self {
            path: config.path.clone(),
            connection,
            nodes_table: RwLock::new(None),
            edges_table: RwLock::new(None),
            runtime,
        };

        // Ensure tables exist
        store.ensure_tables()?;

        Ok(store)
    }

    /// Ensure both nodes and edges tables exist.
    fn ensure_tables(&self) -> DbResult<()> {
        self.ensure_nodes_table()?;
        self.ensure_edges_table()?;
        Ok(())
    }

    /// Ensure the nodes table exists.
    fn ensure_nodes_table(&self) -> DbResult<()> {
        let mut table_guard = self
            .nodes_table
            .write()
            .map_err(|e| DbError::internal(format!("Failed to acquire nodes table lock: {}", e)))?;

        if table_guard.is_some() {
            return Ok(());
        }

        let table_names = self
            .runtime
            .block_on(async { self.connection.table_names().execute().await })
            .map_err(|e| DbError::LanceDb {
                message: format!("Failed to list tables: {}", e),
            })?;

        let table = if table_names.contains(&NODES_TABLE_NAME.to_string()) {
            debug!("Opening existing nodes table '{}'", NODES_TABLE_NAME);
            self.runtime
                .block_on(async { self.connection.open_table(NODES_TABLE_NAME).execute().await })
                .map_err(|e| DbError::LanceDb {
                    message: format!("Failed to open nodes table: {}", e),
                })?
        } else {
            debug!("Creating new nodes table '{}'", NODES_TABLE_NAME);
            let schema = Self::nodes_schema();
            let batch = Self::empty_nodes_batch(&schema)?;
            let batches = RecordBatchIterator::new(vec![Ok(batch)], Arc::new(schema));

            self.runtime
                .block_on(async {
                    self.connection
                        .create_table(NODES_TABLE_NAME, Box::new(batches))
                        .execute()
                        .await
                })
                .map_err(|e| DbError::LanceDb {
                    message: format!("Failed to create nodes table: {}", e),
                })?
        };

        *table_guard = Some(table);
        Ok(())
    }

    /// Ensure the edges table exists.
    fn ensure_edges_table(&self) -> DbResult<()> {
        let mut table_guard = self
            .edges_table
            .write()
            .map_err(|e| DbError::internal(format!("Failed to acquire edges table lock: {}", e)))?;

        if table_guard.is_some() {
            return Ok(());
        }

        let table_names = self
            .runtime
            .block_on(async { self.connection.table_names().execute().await })
            .map_err(|e| DbError::LanceDb {
                message: format!("Failed to list tables: {}", e),
            })?;

        let table = if table_names.contains(&EDGES_TABLE_NAME.to_string()) {
            debug!("Opening existing edges table '{}'", EDGES_TABLE_NAME);
            self.runtime
                .block_on(async { self.connection.open_table(EDGES_TABLE_NAME).execute().await })
                .map_err(|e| DbError::LanceDb {
                    message: format!("Failed to open edges table: {}", e),
                })?
        } else {
            debug!("Creating new edges table '{}'", EDGES_TABLE_NAME);
            let schema = Self::edges_schema();
            let batch = Self::empty_edges_batch(&schema)?;
            let batches = RecordBatchIterator::new(vec![Ok(batch)], Arc::new(schema));

            self.runtime
                .block_on(async {
                    self.connection
                        .create_table(EDGES_TABLE_NAME, Box::new(batches))
                        .execute()
                        .await
                })
                .map_err(|e| DbError::LanceDb {
                    message: format!("Failed to create edges table: {}", e),
                })?
        };

        *table_guard = Some(table);
        Ok(())
    }

    /// Get the nodes table.
    fn get_nodes_table(&self) -> DbResult<Table> {
        self.ensure_nodes_table()?;

        let guard = self
            .nodes_table
            .read()
            .map_err(|e| DbError::internal(format!("Failed to acquire nodes table lock: {}", e)))?;

        guard
            .clone()
            .ok_or_else(|| DbError::internal("Nodes table not initialized"))
    }

    /// Get the edges table.
    fn get_edges_table(&self) -> DbResult<Table> {
        self.ensure_edges_table()?;

        let guard = self
            .edges_table
            .read()
            .map_err(|e| DbError::internal(format!("Failed to acquire edges table lock: {}", e)))?;

        guard
            .clone()
            .ok_or_else(|| DbError::internal("Edges table not initialized"))
    }

    /// Create the Arrow schema for nodes.
    fn nodes_schema() -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("kind", DataType::Utf8, false),
            Field::new("label", DataType::Utf8, false),
            Field::new("props", DataType::Utf8, true), // JSON string
            Field::new("branch", DataType::Utf8, true),
            Field::new("created_at", DataType::Utf8, false),
            Field::new("updated_at", DataType::Utf8, false),
        ])
    }

    /// Create the Arrow schema for edges.
    fn edges_schema() -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("from_node", DataType::Utf8, false), // 'from' is reserved
            Field::new("to_node", DataType::Utf8, false),   // 'to' is reserved
            Field::new("kind", DataType::Utf8, false),
            Field::new("props", DataType::Utf8, true), // JSON string
            Field::new("branch", DataType::Utf8, true),
            Field::new("created_at", DataType::Utf8, false),
            Field::new("updated_at", DataType::Utf8, false),
        ])
    }

    /// Create an empty nodes batch.
    fn empty_nodes_batch(schema: &Schema) -> DbResult<RecordBatch> {
        let ids: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let kinds: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let labels: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let props: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let branches: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let created_ats: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let updated_ats: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));

        RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![
                ids,
                kinds,
                labels,
                props,
                branches,
                created_ats,
                updated_ats,
            ],
        )
        .map_err(|e| DbError::internal(format!("Failed to create empty nodes batch: {}", e)))
    }

    /// Create an empty edges batch.
    fn empty_edges_batch(schema: &Schema) -> DbResult<RecordBatch> {
        let ids: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let froms: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let tos: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let kinds: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let props: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let branches: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let created_ats: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let updated_ats: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));

        RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![
                ids,
                froms,
                tos,
                kinds,
                props,
                branches,
                created_ats,
                updated_ats,
            ],
        )
        .map_err(|e| DbError::internal(format!("Failed to create empty edges batch: {}", e)))
    }

    /// Convert nodes to a RecordBatch.
    fn nodes_to_batch(&self, nodes: &[KgNode]) -> DbResult<RecordBatch> {
        let schema = Self::nodes_schema();

        let ids: ArrayRef = Arc::new(StringArray::from(
            nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        ));
        let kinds: ArrayRef = Arc::new(StringArray::from(
            nodes.iter().map(|n| n.kind.as_str()).collect::<Vec<_>>(),
        ));
        let labels: ArrayRef = Arc::new(StringArray::from(
            nodes.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(),
        ));
        let props: ArrayRef = Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|n| serde_json::to_string(&n.props).ok())
                .collect::<Vec<_>>(),
        ));
        let branches: ArrayRef = Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|n| n.branch.as_deref())
                .collect::<Vec<_>>(),
        ));
        let created_ats: ArrayRef = Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|n| n.created_at.to_rfc3339())
                .collect::<Vec<_>>(),
        ));
        let updated_ats: ArrayRef = Arc::new(StringArray::from(
            nodes
                .iter()
                .map(|n| n.updated_at.to_rfc3339())
                .collect::<Vec<_>>(),
        ));

        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                ids,
                kinds,
                labels,
                props,
                branches,
                created_ats,
                updated_ats,
            ],
        )
        .map_err(|e| DbError::internal(format!("Failed to create nodes batch: {}", e)))
    }

    /// Convert edges to a RecordBatch.
    fn edges_to_batch(&self, edges: &[KgEdge]) -> DbResult<RecordBatch> {
        let schema = Self::edges_schema();

        let ids: ArrayRef = Arc::new(StringArray::from(
            edges.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ));
        let froms: ArrayRef = Arc::new(StringArray::from(
            edges.iter().map(|e| e.from.as_str()).collect::<Vec<_>>(),
        ));
        let tos: ArrayRef = Arc::new(StringArray::from(
            edges.iter().map(|e| e.to.as_str()).collect::<Vec<_>>(),
        ));
        let kinds: ArrayRef = Arc::new(StringArray::from(
            edges.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        ));
        let props: ArrayRef = Arc::new(StringArray::from(
            edges
                .iter()
                .map(|e| serde_json::to_string(&e.props).ok())
                .collect::<Vec<_>>(),
        ));
        let branches: ArrayRef = Arc::new(StringArray::from(
            edges
                .iter()
                .map(|e| e.branch.as_deref())
                .collect::<Vec<_>>(),
        ));
        let created_ats: ArrayRef = Arc::new(StringArray::from(
            edges
                .iter()
                .map(|e| e.created_at.to_rfc3339())
                .collect::<Vec<_>>(),
        ));
        let updated_ats: ArrayRef = Arc::new(StringArray::from(
            edges
                .iter()
                .map(|e| e.updated_at.to_rfc3339())
                .collect::<Vec<_>>(),
        ));

        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                ids,
                froms,
                tos,
                kinds,
                props,
                branches,
                created_ats,
                updated_ats,
            ],
        )
        .map_err(|e| DbError::internal(format!("Failed to create edges batch: {}", e)))
    }

    /// Parse nodes from RecordBatches.
    fn parse_nodes_from_batches(&self, batches: Vec<RecordBatch>) -> DbResult<Vec<KgNode>> {
        let mut nodes = Vec::new();

        for batch in batches {
            let ids = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let kinds = batch
                .column_by_name("kind")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let labels = batch
                .column_by_name("label")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let props = batch
                .column_by_name("props")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let branches = batch
                .column_by_name("branch")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let created_ats = batch
                .column_by_name("created_at")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let updated_ats = batch
                .column_by_name("updated_at")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());

            if let (
                Some(ids),
                Some(kinds),
                Some(labels),
                Some(props),
                Some(branches),
                Some(created_ats),
                Some(updated_ats),
            ) = (
                ids,
                kinds,
                labels,
                props,
                branches,
                created_ats,
                updated_ats,
            ) {
                for i in 0..batch.num_rows() {
                    let id = ids.value(i).to_string();
                    let kind = kinds.value(i).to_string();
                    let label = labels.value(i).to_string();

                    let props_value: serde_json::Value = if props.is_null(i) {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(props.value(i)).unwrap_or(serde_json::json!({}))
                    };

                    let branch = if branches.is_null(i) {
                        None
                    } else {
                        Some(branches.value(i).to_string())
                    };

                    let created_at = chrono::DateTime::parse_from_rfc3339(created_ats.value(i))
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());

                    let updated_at = chrono::DateTime::parse_from_rfc3339(updated_ats.value(i))
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());

                    nodes.push(KgNode {
                        id,
                        kind,
                        label,
                        props: props_value,
                        branch,
                        created_at,
                        updated_at,
                    });
                }
            }
        }

        Ok(nodes)
    }

    /// Parse edges from RecordBatches.
    fn parse_edges_from_batches(&self, batches: Vec<RecordBatch>) -> DbResult<Vec<KgEdge>> {
        let mut edges = Vec::new();

        for batch in batches {
            let ids = batch
                .column_by_name("id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let froms = batch
                .column_by_name("from_node")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let tos = batch
                .column_by_name("to_node")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let kinds = batch
                .column_by_name("kind")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let props = batch
                .column_by_name("props")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let branches = batch
                .column_by_name("branch")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let created_ats = batch
                .column_by_name("created_at")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let updated_ats = batch
                .column_by_name("updated_at")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());

            if let (
                Some(ids),
                Some(froms),
                Some(tos),
                Some(kinds),
                Some(props),
                Some(branches),
                Some(created_ats),
                Some(updated_ats),
            ) = (
                ids,
                froms,
                tos,
                kinds,
                props,
                branches,
                created_ats,
                updated_ats,
            ) {
                for i in 0..batch.num_rows() {
                    let id = ids.value(i).to_string();
                    let from = froms.value(i).to_string();
                    let to = tos.value(i).to_string();
                    let kind = kinds.value(i).to_string();

                    let props_value: serde_json::Value = if props.is_null(i) {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(props.value(i)).unwrap_or(serde_json::json!({}))
                    };

                    let branch = if branches.is_null(i) {
                        None
                    } else {
                        Some(branches.value(i).to_string())
                    };

                    let created_at = chrono::DateTime::parse_from_rfc3339(created_ats.value(i))
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());

                    let updated_at = chrono::DateTime::parse_from_rfc3339(updated_ats.value(i))
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now());

                    edges.push(KgEdge {
                        id,
                        from,
                        to,
                        kind,
                        props: props_value,
                        branch,
                        created_at,
                        updated_at,
                    });
                }
            }
        }

        Ok(edges)
    }

    /// Escape a string for use in LanceDB SQL filter.
    fn escape_sql_string(s: &str) -> String {
        s.replace('\'', "''")
    }
}

impl KgStoreBackend for LanceDbKgStore {
    fn upsert_nodes(&self, nodes: &[KgNode]) -> DbResult<usize> {
        if nodes.is_empty() {
            return Ok(0);
        }

        trace!("Upserting {} nodes", nodes.len());

        let table = self.get_nodes_table()?;

        // Delete existing nodes with same IDs
        let id_list = nodes
            .iter()
            .map(|n| format!("'{}'", Self::escape_sql_string(&n.id)))
            .collect::<Vec<_>>()
            .join(", ");
        let delete_filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            // Try to delete, ignore if no rows match
            if let Err(e) = table.delete(&delete_filter).await {
                debug!("Delete before upsert returned error (may be ok): {}", e);
            }

            // Insert new nodes
            let batch = self.nodes_to_batch(nodes)?;
            let schema = batch.schema();
            let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);

            table
                .add(Box::new(batches))
                .execute()
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Insert nodes failed: {}", e),
                })?;

            Ok(nodes.len())
        })
    }

    fn get_all_nodes(&self) -> DbResult<Vec<KgNode>> {
        trace!("Getting all nodes");

        let table = self.get_nodes_table()?;

        self.runtime.block_on(async {
            let results = table
                .query()
                .execute()
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Query all nodes failed: {}", e),
                })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect node results: {}", e),
                })?;

            self.parse_nodes_from_batches(batches)
        })
    }

    fn get_nodes_by_ids(&self, ids: &[&str]) -> DbResult<Vec<KgNode>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        trace!("Getting nodes by {} IDs", ids.len());

        let table = self.get_nodes_table()?;

        let id_list = ids
            .iter()
            .map(|id| format!("'{}'", Self::escape_sql_string(id)))
            .collect::<Vec<_>>()
            .join(", ");
        let filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query nodes by IDs failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect node results: {}", e),
                })?;

            self.parse_nodes_from_batches(batches)
        })
    }

    fn get_nodes_by_kind(&self, kind: &str) -> DbResult<Vec<KgNode>> {
        trace!("Getting nodes by kind: {}", kind);

        let table = self.get_nodes_table()?;
        let filter = format!("kind = '{}'", Self::escape_sql_string(kind));

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query nodes by kind failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect node results: {}", e),
                })?;

            self.parse_nodes_from_batches(batches)
        })
    }

    fn get_nodes_by_branch(&self, branch: &str) -> DbResult<Vec<KgNode>> {
        trace!("Getting nodes by branch: {}", branch);

        let table = self.get_nodes_table()?;
        let filter = format!("branch = '{}'", Self::escape_sql_string(branch));

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query nodes by branch failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect node results: {}", e),
                })?;

            self.parse_nodes_from_batches(batches)
        })
    }

    fn delete_nodes(&self, ids: &[&str]) -> DbResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }

        trace!("Deleting {} nodes", ids.len());

        let table = self.get_nodes_table()?;

        let id_list = ids
            .iter()
            .map(|id| format!("'{}'", Self::escape_sql_string(id)))
            .collect::<Vec<_>>()
            .join(", ");
        let delete_filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            table
                .delete(&delete_filter)
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Delete nodes failed: {}", e),
                })?;
            Ok(ids.len())
        })
    }

    fn count_nodes(&self) -> DbResult<u64> {
        let table = self.get_nodes_table()?;

        self.runtime.block_on(async {
            let count = table.count_rows(None).await.map_err(|e| DbError::LanceDb {
                message: format!("Count nodes failed: {}", e),
            })?;
            Ok(count as u64)
        })
    }

    fn upsert_edges(&self, edges: &[KgEdge]) -> DbResult<usize> {
        if edges.is_empty() {
            return Ok(0);
        }

        trace!("Upserting {} edges", edges.len());

        let table = self.get_edges_table()?;

        // Delete existing edges with same IDs
        let id_list = edges
            .iter()
            .map(|e| format!("'{}'", Self::escape_sql_string(&e.id)))
            .collect::<Vec<_>>()
            .join(", ");
        let delete_filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            // Try to delete, ignore if no rows match
            if let Err(e) = table.delete(&delete_filter).await {
                debug!("Delete before upsert returned error (may be ok): {}", e);
            }

            // Insert new edges
            let batch = self.edges_to_batch(edges)?;
            let schema = batch.schema();
            let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);

            table
                .add(Box::new(batches))
                .execute()
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Insert edges failed: {}", e),
                })?;

            Ok(edges.len())
        })
    }

    fn get_all_edges(&self) -> DbResult<Vec<KgEdge>> {
        trace!("Getting all edges");

        let table = self.get_edges_table()?;

        self.runtime.block_on(async {
            let results = table
                .query()
                .execute()
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Query all edges failed: {}", e),
                })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect edge results: {}", e),
                })?;

            self.parse_edges_from_batches(batches)
        })
    }

    fn get_edges_by_ids(&self, ids: &[&str]) -> DbResult<Vec<KgEdge>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        trace!("Getting edges by {} IDs", ids.len());

        let table = self.get_edges_table()?;

        let id_list = ids
            .iter()
            .map(|id| format!("'{}'", Self::escape_sql_string(id)))
            .collect::<Vec<_>>()
            .join(", ");
        let filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query edges by IDs failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect edge results: {}", e),
                })?;

            self.parse_edges_from_batches(batches)
        })
    }

    fn get_edges_from(&self, node_id: &str) -> DbResult<Vec<KgEdge>> {
        trace!("Getting edges from node: {}", node_id);

        let table = self.get_edges_table()?;
        let filter = format!("from_node = '{}'", Self::escape_sql_string(node_id));

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query edges from node failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect edge results: {}", e),
                })?;

            self.parse_edges_from_batches(batches)
        })
    }

    fn get_edges_to(&self, node_id: &str) -> DbResult<Vec<KgEdge>> {
        trace!("Getting edges to node: {}", node_id);

        let table = self.get_edges_table()?;
        let filter = format!("to_node = '{}'", Self::escape_sql_string(node_id));

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query edges to node failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect edge results: {}", e),
                })?;

            self.parse_edges_from_batches(batches)
        })
    }

    fn get_edges_by_kind(&self, kind: &str) -> DbResult<Vec<KgEdge>> {
        trace!("Getting edges by kind: {}", kind);

        let table = self.get_edges_table()?;
        let filter = format!("kind = '{}'", Self::escape_sql_string(kind));

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query edges by kind failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect edge results: {}", e),
                })?;

            self.parse_edges_from_batches(batches)
        })
    }

    fn get_edges_by_branch(&self, branch: &str) -> DbResult<Vec<KgEdge>> {
        trace!("Getting edges by branch: {}", branch);

        let table = self.get_edges_table()?;
        let filter = format!("branch = '{}'", Self::escape_sql_string(branch));

        self.runtime.block_on(async {
            let results =
                table
                    .query()
                    .only_if(filter)
                    .execute()
                    .await
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Query edges by branch failed: {}", e),
                    })?;

            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect edge results: {}", e),
                })?;

            self.parse_edges_from_batches(batches)
        })
    }

    fn delete_edges(&self, ids: &[&str]) -> DbResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }

        trace!("Deleting {} edges", ids.len());

        let table = self.get_edges_table()?;

        let id_list = ids
            .iter()
            .map(|id| format!("'{}'", Self::escape_sql_string(id)))
            .collect::<Vec<_>>()
            .join(", ");
        let delete_filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            table
                .delete(&delete_filter)
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Delete edges failed: {}", e),
                })?;
            Ok(ids.len())
        })
    }

    fn count_edges(&self) -> DbResult<u64> {
        let table = self.get_edges_table()?;

        self.runtime.block_on(async {
            let count = table.count_rows(None).await.map_err(|e| DbError::LanceDb {
                message: format!("Count edges failed: {}", e),
            })?;
            Ok(count as u64)
        })
    }

    fn get_stats(&self) -> DbResult<KgStats> {
        let node_count = self.count_nodes()?;
        let edge_count = self.count_edges()?;

        Ok(KgStats::new(node_count, edge_count))
    }

    fn refresh_stats(&self) -> DbResult<KgStats> {
        // For LanceDB, get_stats already computes fresh counts
        self.get_stats()
    }

    fn clear(&self) -> DbResult<()> {
        debug!("Clearing all KG data");

        let nodes_table = self.get_nodes_table()?;
        let edges_table = self.get_edges_table()?;

        self.runtime.block_on(async {
            // Delete all nodes
            if let Err(e) = nodes_table.delete("id IS NOT NULL").await {
                debug!("Clear nodes returned error (may be ok): {}", e);
            }

            // Delete all edges
            if let Err(e) = edges_table.delete("id IS NOT NULL").await {
                debug!("Clear edges returned error (may be ok): {}", e);
            }

            Ok(())
        })
    }

    fn flush(&self) -> DbResult<()> {
        // LanceDB writes are durable by default
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kg::entities::KG_VERSION;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, LanceDbKgStore) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config = KgStoreConfig::new(temp_dir.path());
        let store = LanceDbKgStore::open(&config).expect("Failed to create store");
        (temp_dir, store)
    }

    #[test]
    fn test_open_store() {
        let (_temp_dir, store) = create_test_store();
        assert_eq!(store.count_nodes().unwrap(), 0);
        assert_eq!(store.count_edges().unwrap(), 0);
    }

    #[test]
    fn test_upsert_and_get_nodes() {
        let (_temp_dir, store) = create_test_store();

        let nodes = vec![
            KgNode::new("file:src/main.rs", "file", "main.rs")
                .with_props(json!({"language": "rust"})),
            KgNode::new("file:src/lib.rs", "file", "lib.rs"),
        ];

        let count = store.upsert_nodes(&nodes).unwrap();
        assert_eq!(count, 2);

        let all_nodes = store.get_all_nodes().unwrap();
        assert_eq!(all_nodes.len(), 2);

        let file_nodes = store.get_nodes_by_kind("file").unwrap();
        assert_eq!(file_nodes.len(), 2);
    }

    #[test]
    fn test_upsert_and_get_edges() {
        let (_temp_dir, store) = create_test_store();

        let edges = vec![
            KgEdge::new("file:main.rs", "file:lib.rs", "imports"),
            KgEdge::new("file:lib.rs", "file:util.rs", "imports"),
        ];

        let count = store.upsert_edges(&edges).unwrap();
        assert_eq!(count, 2);

        let all_edges = store.get_all_edges().unwrap();
        assert_eq!(all_edges.len(), 2);

        let imports = store.get_edges_by_kind("imports").unwrap();
        assert_eq!(imports.len(), 2);
    }

    #[test]
    fn test_get_edges_from_to() {
        let (_temp_dir, store) = create_test_store();

        let edges = vec![
            KgEdge::new("file:a.rs", "file:b.rs", "imports"),
            KgEdge::new("file:a.rs", "file:c.rs", "imports"),
            KgEdge::new("file:b.rs", "file:c.rs", "imports"),
        ];

        store.upsert_edges(&edges).unwrap();

        let from_a = store.get_edges_from("file:a.rs").unwrap();
        assert_eq!(from_a.len(), 2);

        let to_c = store.get_edges_to("file:c.rs").unwrap();
        assert_eq!(to_c.len(), 2);
    }

    #[test]
    fn test_delete_nodes() {
        let (_temp_dir, store) = create_test_store();

        let nodes = vec![
            KgNode::new("file:a.rs", "file", "a.rs"),
            KgNode::new("file:b.rs", "file", "b.rs"),
        ];

        store.upsert_nodes(&nodes).unwrap();
        assert_eq!(store.count_nodes().unwrap(), 2);

        store.delete_nodes(&["file:a.rs"]).unwrap();
        assert_eq!(store.count_nodes().unwrap(), 1);
    }

    #[test]
    fn test_get_stats() {
        let (_temp_dir, store) = create_test_store();

        let nodes = vec![
            KgNode::new("file:a.rs", "file", "a.rs"),
            KgNode::new("file:b.rs", "file", "b.rs"),
            KgNode::new("file:c.rs", "file", "c.rs"),
        ];
        let edges = vec![
            KgEdge::new("file:a.rs", "file:b.rs", "imports"),
            KgEdge::new("file:b.rs", "file:c.rs", "imports"),
        ];

        store.upsert_nodes(&nodes).unwrap();
        store.upsert_edges(&edges).unwrap();

        let stats = store.get_stats().unwrap();
        assert_eq!(stats.node_count, 3);
        assert_eq!(stats.edge_count, 2);
        assert_eq!(stats.version, KG_VERSION);
    }

    #[test]
    fn test_clear() {
        let (_temp_dir, store) = create_test_store();

        let nodes = vec![KgNode::new("file:a.rs", "file", "a.rs")];
        let edges = vec![KgEdge::new("file:a.rs", "file:b.rs", "imports")];

        store.upsert_nodes(&nodes).unwrap();
        store.upsert_edges(&edges).unwrap();

        assert_eq!(store.count_nodes().unwrap(), 1);
        assert_eq!(store.count_edges().unwrap(), 1);

        store.clear().unwrap();

        assert_eq!(store.count_nodes().unwrap(), 0);
        assert_eq!(store.count_edges().unwrap(), 0);
    }

    #[test]
    fn test_upsert_overwrites() {
        let (_temp_dir, store) = create_test_store();

        let node1 = KgNode::new("file:a.rs", "file", "original label");
        store.upsert_nodes(&[node1]).unwrap();

        let nodes = store.get_nodes_by_ids(&["file:a.rs"]).unwrap();
        assert_eq!(nodes[0].label, "original label");

        let node2 = KgNode::new("file:a.rs", "file", "updated label");
        store.upsert_nodes(&[node2]).unwrap();

        let nodes = store.get_nodes_by_ids(&["file:a.rs"]).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].label, "updated label");
    }
}
