//! LanceDB vector index backend.
//!
//! This backend provides production-ready vector search using LanceDB,
//! with support for ANN (Approximate Nearest Neighbor) search and
//! rich metadata filtering.

use super::super::config::{VectorIndexConfig, LANCEDB_TABLE_NAME};
use super::super::metadata::VectorSearchFilter;
use super::super::traits::{
    VectorId, VectorIndexBackend, VectorInsert, VectorMetric, VectorSearchResult,
};
use crate::error::{DbError, DbResult};
use arrow_array::{
    Array, ArrayRef, FixedSizeListArray, Float32Array, Int64Array, RecordBatch,
    RecordBatchIterator, StringArray,
};
use arrow_buffer::OffsetBuffer;
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lance_arrow::FixedSizeListArrayExt;
use lancedb::{
    connect,
    query::{ExecutableQuery, QueryBase},
    Connection, Table,
};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::runtime::Runtime;
use tracing::{debug, trace};

/// LanceDB vector index backend.
pub struct LanceDbVectorIndex {
    /// Path to the index directory.
    #[allow(dead_code)]
    path: PathBuf,

    /// Vector dimension.
    dimension: usize,

    /// Distance metric.
    metric: VectorMetric,

    /// LanceDB connection.
    connection: Connection,

    /// LanceDB table (lazily initialized).
    table: RwLock<Option<Table>>,

    /// Tokio runtime for async operations.
    runtime: Runtime,
}

impl LanceDbVectorIndex {
    /// Open or create a LanceDB vector index.
    pub fn open(config: &VectorIndexConfig) -> DbResult<Self> {
        debug!("Opening LanceDbVectorIndex at {:?}", config.path);

        // Create tokio runtime
        let runtime = Runtime::new()
            .map_err(|e| DbError::internal(format!("Failed to create runtime: {}", e)))?;

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

        let index = Self {
            path: config.path.clone(),
            dimension: config.dimension,
            metric: config.metric,
            connection,
            table: RwLock::new(None),
            runtime,
        };

        // Try to open existing table
        index.ensure_table()?;

        Ok(index)
    }

    /// Ensure the table exists, creating it if necessary.
    fn ensure_table(&self) -> DbResult<()> {
        let mut table_guard = self
            .table
            .write()
            .map_err(|e| DbError::internal(format!("Failed to acquire table lock: {}", e)))?;

        if table_guard.is_some() {
            return Ok(());
        }

        // Check if table exists
        let table_names = self
            .runtime
            .block_on(async { self.connection.table_names().execute().await })
            .map_err(|e| DbError::LanceDb {
                message: format!("Failed to list tables: {}", e),
            })?;

        let table = if table_names.contains(&LANCEDB_TABLE_NAME.to_string()) {
            debug!("Opening existing table '{}'", LANCEDB_TABLE_NAME);
            self.runtime
                .block_on(async {
                    self.connection
                        .open_table(LANCEDB_TABLE_NAME)
                        .execute()
                        .await
                })
                .map_err(|e| DbError::LanceDb {
                    message: format!("Failed to open table: {}", e),
                })?
        } else {
            debug!("Creating new table '{}'", LANCEDB_TABLE_NAME);
            // Create empty table with schema
            let schema = self.create_schema();
            let batch = self.create_empty_batch(&schema)?;
            let batches = RecordBatchIterator::new(vec![Ok(batch)], Arc::new(schema));

            self.runtime
                .block_on(async {
                    self.connection
                        .create_table(LANCEDB_TABLE_NAME, Box::new(batches))
                        .execute()
                        .await
                })
                .map_err(|e| DbError::LanceDb {
                    message: format!("Failed to create table: {}", e),
                })?
        };

        *table_guard = Some(table);
        Ok(())
    }

    /// Get the table, initializing if needed.
    fn get_table(&self) -> DbResult<Table> {
        self.ensure_table()?;

        let guard = self
            .table
            .read()
            .map_err(|e| DbError::internal(format!("Failed to acquire table lock: {}", e)))?;

        guard
            .clone()
            .ok_or_else(|| DbError::internal("Table not initialized"))
    }

    /// Create the Arrow schema for vectors.
    fn create_schema(&self) -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    self.dimension as i32,
                ),
                false,
            ),
            Field::new("payload", DataType::Utf8, true),
            Field::new("base", DataType::Utf8, false),
            Field::new("branch", DataType::Utf8, true),
            Field::new("source_type", DataType::Utf8, false),
            Field::new("path", DataType::Utf8, true),
            Field::new(
                "tags",
                DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
                false,
            ),
            Field::new("revision_id", DataType::Utf8, true),
            Field::new("created_at", DataType::Utf8, true),
            Field::new("updated_at", DataType::Utf8, true),
        ])
    }

    /// Create an empty RecordBatch with the schema.
    fn create_empty_batch(&self, schema: &Schema) -> DbResult<RecordBatch> {
        let ids: ArrayRef = Arc::new(Int64Array::from(Vec::<i64>::new()));

        // Create empty fixed-size list for vectors
        let values = Float32Array::from(Vec::<f32>::new());
        let vector_array =
            FixedSizeListArray::try_new_from_values(values, self.dimension as i32)
                .map_err(|e| DbError::internal(format!("Failed to create vector array: {}", e)))?;
        let vectors: ArrayRef = Arc::new(vector_array);

        let payloads: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let bases: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let branches: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let source_types: ArrayRef = Arc::new(StringArray::from(Vec::<&str>::new()));
        let paths: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));

        // Empty tags list
        let tags_values = StringArray::from(Vec::<&str>::new());
        let tags_offsets = OffsetBuffer::new(vec![0i32].into());
        let tags_array = arrow_array::ListArray::new(
            Arc::new(Field::new("item", DataType::Utf8, true)),
            tags_offsets,
            Arc::new(tags_values),
            None,
        );
        let tags: ArrayRef = Arc::new(tags_array);

        let revision_ids: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let created_ats: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));
        let updated_ats: ArrayRef = Arc::new(StringArray::from(Vec::<Option<&str>>::new()));

        RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![
                ids,
                vectors,
                payloads,
                bases,
                branches,
                source_types,
                paths,
                tags,
                revision_ids,
                created_ats,
                updated_ats,
            ],
        )
        .map_err(|e| DbError::internal(format!("Failed to create batch: {}", e)))
    }

    /// Convert VectorInsert items to a RecordBatch.
    fn inserts_to_batch(&self, inserts: &[VectorInsert]) -> DbResult<RecordBatch> {
        let schema = self.create_schema();

        let ids: ArrayRef = Arc::new(Int64Array::from(
            inserts
                .iter()
                .map(|i| i.id.value() as i64)
                .collect::<Vec<_>>(),
        ));

        // Create vectors as FixedSizeList
        let flat_vectors: Vec<f32> = inserts.iter().flat_map(|i| i.vector.clone()).collect();
        let values = Float32Array::from(flat_vectors);
        let vector_array =
            FixedSizeListArray::try_new_from_values(values, self.dimension as i32)
                .map_err(|e| DbError::internal(format!("Failed to create vector array: {}", e)))?;
        let vectors: ArrayRef = Arc::new(vector_array);

        let payloads: ArrayRef = Arc::new(StringArray::from(
            inserts
                .iter()
                .map(|i| serde_json::to_string(&i.payload).ok())
                .collect::<Vec<_>>(),
        ));

        let bases: ArrayRef = Arc::new(StringArray::from(
            inserts.iter().map(|i| i.base.as_str()).collect::<Vec<_>>(),
        ));

        let branches: ArrayRef = Arc::new(StringArray::from(
            inserts
                .iter()
                .map(|i| i.branch.as_deref())
                .collect::<Vec<_>>(),
        ));

        let source_types: ArrayRef = Arc::new(StringArray::from(
            inserts
                .iter()
                .map(|i| i.source_type.as_str())
                .collect::<Vec<_>>(),
        ));

        let paths: ArrayRef = Arc::new(StringArray::from(
            inserts
                .iter()
                .map(|i| i.path.as_deref())
                .collect::<Vec<_>>(),
        ));

        // Build tags list array
        let tags = self.build_tags_array(inserts)?;

        let revision_ids: ArrayRef = Arc::new(StringArray::from(
            inserts
                .iter()
                .map(|i| i.revision_id.as_deref())
                .collect::<Vec<_>>(),
        ));

        let now = chrono::Utc::now().to_rfc3339();
        let timestamps: Vec<Option<&str>> = inserts.iter().map(|_| Some(now.as_str())).collect();
        let created_ats: ArrayRef = Arc::new(StringArray::from(timestamps.clone()));
        let updated_ats: ArrayRef = Arc::new(StringArray::from(timestamps));

        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                ids,
                vectors,
                payloads,
                bases,
                branches,
                source_types,
                paths,
                tags,
                revision_ids,
                created_ats,
                updated_ats,
            ],
        )
        .map_err(|e| DbError::internal(format!("Failed to create batch: {}", e)))
    }

    /// Build a ListArray for tags.
    fn build_tags_array(&self, inserts: &[VectorInsert]) -> DbResult<ArrayRef> {
        let mut all_tags: Vec<&str> = Vec::new();
        let mut offsets: Vec<i32> = vec![0];

        for insert in inserts {
            for tag in &insert.tags {
                all_tags.push(tag.as_str());
            }
            offsets.push(all_tags.len() as i32);
        }

        let tags_values = StringArray::from(all_tags);
        let tags_offsets = OffsetBuffer::new(offsets.into());

        let tags_array = arrow_array::ListArray::new(
            Arc::new(Field::new("item", DataType::Utf8, true)),
            tags_offsets,
            Arc::new(tags_values),
            None,
        );

        Ok(Arc::new(tags_array))
    }

    /// Convert distance metric to LanceDB string.
    #[allow(dead_code)]
    fn metric_to_lance(&self) -> &'static str {
        match self.metric {
            VectorMetric::Cosine => "cosine",
            VectorMetric::Dot => "dot",
            VectorMetric::L2 => "l2",
        }
    }
}

impl VectorIndexBackend for LanceDbVectorIndex {
    fn query(
        &self,
        embedding: &[f32],
        limit: usize,
        filter: Option<&VectorSearchFilter>,
    ) -> DbResult<Vec<VectorSearchResult>> {
        trace!("Querying LanceDbVectorIndex, limit={}", limit);

        let table = self.get_table()?;

        self.runtime.block_on(async {
            // Build query
            let mut query =
                table
                    .vector_search(embedding.to_vec())
                    .map_err(|e| DbError::LanceDb {
                        message: format!("Failed to create query: {}", e),
                    })?;

            // Apply filter if provided
            if let Some(f) = filter {
                if let Some(filter_str) = f.to_lance_filter() {
                    debug!("Applying filter: {}", filter_str);
                    query = query.only_if(filter_str);
                }
            }

            // Set limit and metric
            query = query
                .limit(limit)
                .distance_type(lancedb::DistanceType::Cosine);

            // Execute query
            let results = query.execute().await.map_err(|e| DbError::LanceDb {
                message: format!("Query failed: {}", e),
            })?;

            // Collect results
            let batches: Vec<RecordBatch> =
                results.try_collect().await.map_err(|e| DbError::LanceDb {
                    message: format!("Failed to collect results: {}", e),
                })?;

            // Convert to VectorSearchResult
            let mut search_results = Vec::new();

            for batch in batches {
                let ids = batch
                    .column_by_name("id")
                    .and_then(|c| c.as_any().downcast_ref::<Int64Array>());

                let payloads = batch
                    .column_by_name("payload")
                    .and_then(|c| c.as_any().downcast_ref::<StringArray>());

                let distances = batch
                    .column_by_name("_distance")
                    .and_then(|c| c.as_any().downcast_ref::<Float32Array>());

                if let (Some(ids), Some(payloads), Some(distances)) = (ids, payloads, distances) {
                    for i in 0..batch.num_rows() {
                        let id = ids.value(i);
                        let payload_str = payloads.value(i);
                        let payload: serde_json::Value =
                            serde_json::from_str(payload_str).unwrap_or(serde_json::json!({}));

                        // Convert distance to similarity score
                        // For cosine: similarity = 1 - distance
                        let distance = distances.value(i);
                        let score = 1.0 - distance;

                        search_results.push(VectorSearchResult::new(
                            VectorId::from(id),
                            score,
                            payload,
                        ));
                    }
                }
            }

            Ok(search_results)
        })
    }

    fn upsert(&self, vectors: &[VectorInsert]) -> DbResult<()> {
        if vectors.is_empty() {
            return Ok(());
        }

        debug!("Upserting {} vectors", vectors.len());

        // Validate dimensions
        for insert in vectors {
            if insert.vector.len() != self.dimension {
                return Err(DbError::DimensionMismatch {
                    expected: self.dimension,
                    actual: insert.vector.len(),
                });
            }
        }

        let table = self.get_table()?;

        // First, delete existing vectors with these IDs
        let id_list = vectors
            .iter()
            .map(|v| v.id.value().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let delete_filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            // Try to delete, ignore if no rows match
            if let Err(e) = table.delete(&delete_filter).await {
                debug!("Delete before upsert returned error (may be ok): {}", e);
            }

            // Insert new vectors
            let batch = self.inserts_to_batch(vectors)?;
            let schema = batch.schema();
            let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);

            table
                .add(Box::new(batches))
                .execute()
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Insert failed: {}", e),
                })?;

            Ok(())
        })
    }

    fn delete(&self, ids: &[VectorId]) -> DbResult<()> {
        if ids.is_empty() {
            return Ok(());
        }

        debug!("Deleting {} vectors", ids.len());

        let table = self.get_table()?;

        let id_list = ids
            .iter()
            .map(|id| id.value().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let delete_filter = format!("id IN ({})", id_list);

        self.runtime.block_on(async {
            table
                .delete(&delete_filter)
                .await
                .map_err(|e| DbError::LanceDb {
                    message: format!("Delete failed: {}", e),
                })?;
            Ok(())
        })
    }

    fn flush(&self) -> DbResult<()> {
        // LanceDB writes are durable by default
        Ok(())
    }

    fn len(&self) -> DbResult<usize> {
        let table = self.get_table()?;

        self.runtime.block_on(async {
            let count = table.count_rows(None).await.map_err(|e| DbError::LanceDb {
                message: format!("Count failed: {}", e),
            })?;
            Ok(count)
        })
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn metric(&self) -> VectorMetric {
        self.metric
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_to_lance() {
        let filter = VectorSearchFilter::new()
            .with_base("code")
            .with_branch("main");

        let lance_filter = filter.to_lance_filter().unwrap();
        assert!(lance_filter.contains("base = 'code'"));
        assert!(lance_filter.contains("branch = 'main'"));
    }
}
