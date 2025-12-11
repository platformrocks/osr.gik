//! Knowledge Graph (KG) storage module for gik-db.
//!
//! This module provides the storage backend for the Knowledge Graph:
//! - [`KgNode`] - Nodes representing entities (files, modules, symbols, concepts)
//! - [`KgEdge`] - Edges representing relationships between nodes
//! - [`KgStats`] - Aggregate statistics for the graph
//! - [`KgStoreBackend`] - Trait for KG storage implementations
//!
//! ## Backends
//!
//! - **LanceDB** (default, feature-gated): Production-ready with efficient queries
//! - **JSONL** (simple, feature-gated): File-based storage for testing/small graphs
//!
//! ## Architecture
//!
//! The KG store is designed to be used by gik-core through trait objects:
//!
//! ```text
//! gik-core (KG sync/query)
//!     ↓
//! gik-db::kg::KgStoreBackend trait
//!     ↓
//! LanceDbKgStore (production) or JsonlKgStore (testing)
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use gik_db::kg::{KgNode, KgEdge, KgStoreConfig, open_kg_store};
//!
//! // Open or create a KG store
//! let config = KgStoreConfig::new("/path/to/kg");
//! let store = open_kg_store(&config)?;
//!
//! // Add nodes and edges
//! let node = KgNode::new("file:src/main.rs", "file", "src/main.rs");
//! store.upsert_nodes(&[node])?;
//!
//! let edge = KgEdge::new("file:src/main.rs", "file:src/lib.rs", "imports");
//! store.upsert_edges(&[edge])?;
//!
//! // Query
//! let nodes = store.get_nodes_by_kind("file")?;
//! let edges = store.get_edges_from("file:src/main.rs")?;
//! ```

pub mod entities;
pub mod traits;

#[cfg(feature = "lancedb")]
pub mod backend;

// Re-export core types
pub use entities::{KgEdge, KgNode, KgStats, KG_VERSION};
pub use traits::{KgStoreBackend, KgStoreConfig};

#[cfg(feature = "lancedb")]
pub use backend::open_kg_store;

use crate::error::DbResult;

/// Create a KG store with the given configuration.
///
/// This is the main factory function for creating KG store backends.
/// It automatically selects the appropriate backend based on configuration
/// and available features.
///
/// # Arguments
///
/// * `config` - KG store configuration
///
/// # Returns
///
/// A boxed `KgStoreBackend` implementation.
///
/// # Errors
///
/// Returns an error if store creation fails.
#[cfg(feature = "lancedb")]
pub fn create_kg_store(config: &KgStoreConfig) -> DbResult<Box<dyn KgStoreBackend>> {
    backend::open_kg_store(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entities_reexported() {
        // Verify entities are accessible through the module
        let node = KgNode::new("test:id", "test", "Test Node");
        assert_eq!(node.id, "test:id");

        let edge = KgEdge::new("from:id", "to:id", "relates");
        assert_eq!(edge.kind, "relates");

        let stats = KgStats::new(10, 20);
        assert_eq!(stats.node_count, 10);
    }
}
