//! Vector index module for gik-db.
//!
//! This module provides vector storage backends for GIK's similarity search.
//!
//! ## Available Backends
//!
//! - `lancedb` (default): Production-ready LanceDB with ANN search
//! - `simple`: File-based backend for testing/small indexes
//!
//! ## Usage
//!
//! ```ignore
//! use gik_db::vector::{VectorIndexConfig, open_vector_index};
//!
//! let config = VectorIndexConfig::new(1024, "/path/to/index");
//! let index = open_vector_index(&config)?;
//!
//! // Insert vectors
//! index.upsert(&inserts)?;
//!
//! // Query similar vectors
//! let results = index.query(&embedding, 10, None)?;
//! ```

mod backend;
mod config;
mod metadata;
mod traits;

// Re-export main types
pub use config::{
    check_index_compatibility, load_index_meta, write_index_meta, VectorIndexCompatibility,
    VectorIndexConfig, VectorIndexMeta, DEFAULT_BACKEND, INDEX_META_FILENAME, LANCEDB_TABLE_NAME,
};
pub use metadata::{
    VectorMetadata, VectorSearchFilter, SOURCE_TYPE_ARCHIVE, SOURCE_TYPE_FILE, SOURCE_TYPE_MEMORY,
    SOURCE_TYPE_URL,
};
pub use traits::{VectorId, VectorIndexBackend, VectorInsert, VectorMetric, VectorSearchResult};

// Re-export backend factory function
pub use backend::open_vector_index;

// Re-export backends
#[cfg(feature = "lancedb")]
pub use backend::LanceDbVectorIndex;

#[cfg(feature = "simple")]
pub use backend::SimpleFileVectorIndex;
