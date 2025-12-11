//! KG storage backends.
//!
//! This module provides the backend implementations for KG storage.

#[cfg(feature = "lancedb")]
pub mod lancedb;

#[cfg(feature = "lancedb")]
pub use lancedb::LanceDbKgStore;

use crate::error::DbResult;

use super::traits::{KgStoreBackend, KgStoreConfig};

/// Open a KG store with the given configuration.
///
/// This is the main factory function for creating KG store backends.
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
pub fn open_kg_store(config: &KgStoreConfig) -> DbResult<Box<dyn KgStoreBackend>> {
    let store = LanceDbKgStore::open(config)?;
    Ok(Box::new(store))
}
