//! # gik-db
//!
//! Infrastructure layer for GIK - vector storage and KG persistence.
//!
//! This crate provides the "heavy" infrastructure implementations that are isolated
//! from the core domain logic in `gik-core`. By separating these concerns:
//!
//! - Changes to `gik-core` compile fast (no heavy DB deps)
//! - Vector storage backends can be swapped without changing domain logic
//! - Testing is easier with mock implementations
//!
//! ## Architecture
//!
//! ```text
//! gik-cli → gik-core → (traits)
//!              ↑
//!           gik-db (implements traits for vector/KG storage)
//!           gik-model (implements traits for embeddings/reranker)
//! ```
//!
//! ## Features
//!
//! - `lancedb` (default): LanceDB vector storage with ANN search
//! - `simple`: Simple file-based vector backend for testing
//!
//! ## Modules
//!
//! - `vector`: Vector index backends (LanceDB, SimpleFile)
//! - `kg`: Knowledge graph storage (LanceDB)
//!
//! ## Usage
//!
//! ```ignore
//! use gik_db::vector::{VectorIndexConfig, open_vector_index};
//!
//! // Create a vector index
//! let config = VectorIndexConfig::new(1024, "/path/to/index");
//! let index = open_vector_index(&config)?;
//!
//! // Insert vectors
//! index.upsert(&inserts)?;
//!
//! // Query similar vectors
//! let results = index.query(&embedding, 10, None)?;
//! ```

pub mod error;
pub mod kg;
pub mod vector;

pub use error::{DbError, DbResult};
