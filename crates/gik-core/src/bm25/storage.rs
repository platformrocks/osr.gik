//! BM25 index serialization and storage.
//!
//! Uses bincode v2 for efficient binary serialization of the BM25 index.
//! Storage layout:
//!
//! ```text
//! .guided/knowledge/<branch>/bases/<base>/bm25/
//! ├── index.bin         # Serialized Bm25Index
//! └── meta.json         # Index metadata (stats, config hash)
//! ```

use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use bincode::config;
use serde::{Deserialize, Serialize};

use super::index::{Bm25Index, Bm25IndexStats};
use crate::errors::GikError;

/// Directory name for BM25 index storage.
pub const BM25_DIR_NAME: &str = "bm25";

/// Filename for the serialized index.
const INDEX_FILENAME: &str = "index.bin";

/// Filename for index metadata.
const META_FILENAME: &str = "meta.json";

/// BM25 index metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25IndexMeta {
    /// Index version for compatibility checks.
    pub version: u32,
    /// Statistics about the index.
    pub stats: Bm25IndexStats,
    /// Timestamp when the index was built (Unix epoch seconds).
    pub built_at: u64,
}

impl Bm25IndexMeta {
    /// Current index version.
    pub const CURRENT_VERSION: u32 = 1;

    /// Create new metadata for an index.
    pub fn new(stats: Bm25IndexStats) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            stats,
            built_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }
}

/// Get the BM25 index directory path for a base.
pub fn bm25_dir_for_base(base_root: &Path) -> PathBuf {
    base_root.join(BM25_DIR_NAME)
}

/// Get the index file path.
pub fn index_path(bm25_dir: &Path) -> PathBuf {
    bm25_dir.join(INDEX_FILENAME)
}

/// Get the metadata file path.
pub fn meta_path(bm25_dir: &Path) -> PathBuf {
    bm25_dir.join(META_FILENAME)
}

/// Save a BM25 index to disk.
///
/// Creates the directory structure if it doesn't exist.
///
/// # Arguments
///
/// * `index` - The BM25 index to save
/// * `base_root` - Path to the base directory (e.g., `.guided/knowledge/main/bases/code`)
///
/// # Errors
///
/// Returns an error if:
/// - Directory creation fails
/// - Serialization fails
/// - File write fails
pub fn save_bm25_index(index: &Bm25Index, base_root: &Path) -> Result<(), GikError> {
    let bm25_dir = bm25_dir_for_base(base_root);

    // Create directory if needed
    fs::create_dir_all(&bm25_dir).map_err(|e| GikError::BaseStoreIo {
        path: bm25_dir.clone(),
        message: format!("Failed to create BM25 directory: {}", e),
    })?;

    // Serialize index with bincode
    let index_file = index_path(&bm25_dir);
    let file = fs::File::create(&index_file).map_err(|e| GikError::BaseStoreIo {
        path: index_file.clone(),
        message: format!("Failed to create BM25 index file: {}", e),
    })?;
    let mut writer = BufWriter::new(file);

    bincode::encode_into_std_write(index, &mut writer, config::standard()).map_err(|e| GikError::BaseStoreParse {
        path: index_file.clone(),
        message: format!("Failed to serialize BM25 index: {}", e),
    })?;

    // Save metadata
    let meta = Bm25IndexMeta::new(index.stats());
    let meta_file = meta_path(&bm25_dir);
    let meta_json = serde_json::to_string_pretty(&meta).map_err(|e| GikError::BaseStoreParse {
        path: meta_file.clone(),
        message: format!("Failed to serialize BM25 metadata: {}", e),
    })?;
    fs::write(&meta_file, meta_json).map_err(|e| GikError::BaseStoreIo {
        path: meta_file.clone(),
        message: format!("Failed to write BM25 metadata: {}", e),
    })?;

    tracing::debug!(
        "Saved BM25 index to {}: {} docs, {} terms",
        bm25_dir.display(),
        index.num_documents(),
        index.vocabulary_size()
    );

    Ok(())
}

/// Load a BM25 index from disk.
///
/// # Arguments
///
/// * `base_root` - Path to the base directory
///
/// # Returns
///
/// The loaded BM25 index, or None if no index exists.
///
/// # Errors
///
/// Returns an error if the index exists but cannot be loaded.
pub fn load_bm25_index(base_root: &Path) -> Result<Option<Bm25Index>, GikError> {
    let bm25_dir = bm25_dir_for_base(base_root);
    let index_file = index_path(&bm25_dir);

    if !index_file.exists() {
        tracing::debug!("No BM25 index found at {}", index_file.display());
        return Ok(None);
    }

    // Check metadata version first
    let meta_file = meta_path(&bm25_dir);
    if meta_file.exists() {
        let meta_content = fs::read_to_string(&meta_file).map_err(|e| GikError::BaseStoreIo {
            path: meta_file.clone(),
            message: format!("Failed to read BM25 metadata: {}", e),
        })?;
        let meta: Bm25IndexMeta =
            serde_json::from_str(&meta_content).map_err(|e| GikError::BaseStoreParse {
                path: meta_file.clone(),
                message: format!("Failed to parse BM25 metadata: {}", e),
            })?;

        if meta.version != Bm25IndexMeta::CURRENT_VERSION {
            tracing::warn!(
                "BM25 index version mismatch: found {}, expected {}. Index will be rebuilt.",
                meta.version,
                Bm25IndexMeta::CURRENT_VERSION
            );
            return Ok(None);
        }
    }

    // Load index
    let file = fs::File::open(&index_file).map_err(|e| GikError::BaseStoreIo {
        path: index_file.clone(),
        message: format!("Failed to open BM25 index: {}", e),
    })?;
    let mut reader = BufReader::new(file);

    let index: Bm25Index =
        bincode::decode_from_std_read(&mut reader, config::standard()).map_err(|e| GikError::BaseStoreParse {
            path: index_file.clone(),
            message: format!("Failed to deserialize BM25 index: {}", e),
        })?;

    tracing::debug!(
        "Loaded BM25 index from {}: {} docs, {} terms",
        bm25_dir.display(),
        index.num_documents(),
        index.vocabulary_size()
    );

    Ok(Some(index))
}

/// Check if a BM25 index exists for a base.
#[allow(dead_code)]
pub fn bm25_index_exists(base_root: &Path) -> bool {
    let index_file = index_path(&bm25_dir_for_base(base_root));
    index_file.exists()
}

/// Load BM25 index metadata without loading the full index.
#[allow(dead_code)]
pub fn load_bm25_meta(base_root: &Path) -> Result<Option<Bm25IndexMeta>, GikError> {
    let meta_file = meta_path(&bm25_dir_for_base(base_root));

    if !meta_file.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&meta_file).map_err(|e| GikError::BaseStoreIo {
        path: meta_file.clone(),
        message: format!("Failed to read BM25 metadata: {}", e),
    })?;

    let meta: Bm25IndexMeta =
        serde_json::from_str(&content).map_err(|e| GikError::BaseStoreParse {
            path: meta_file.clone(),
            message: format!("Failed to parse BM25 metadata: {}", e),
        })?;

    Ok(Some(meta))
}

/// Delete BM25 index for a base.
#[allow(dead_code)]
pub fn delete_bm25_index(base_root: &Path) -> Result<(), GikError> {
    let bm25_dir = bm25_dir_for_base(base_root);

    if bm25_dir.exists() {
        fs::remove_dir_all(&bm25_dir).map_err(|e| GikError::BaseStoreIo {
            path: bm25_dir.clone(),
            message: format!("Failed to delete BM25 directory: {}", e),
        })?;
        tracing::debug!("Deleted BM25 index at {}", bm25_dir.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bm25::Bm25Config;
    use tempfile::TempDir;

    fn create_test_index() -> Bm25Index {
        let mut index = Bm25Index::new(Bm25Config::default());
        index.add_document("doc1".to_string(), "hello world");
        index.add_document("doc2".to_string(), "rust programming");
        index.add_document("doc3".to_string(), "hello rust");
        index
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let base_root = temp_dir.path();

        let original_index = create_test_index();

        // Save
        save_bm25_index(&original_index, base_root).unwrap();

        // Check files exist
        assert!(bm25_index_exists(base_root));

        // Load
        let loaded_index = load_bm25_index(base_root).unwrap().unwrap();

        // Verify
        assert_eq!(loaded_index.num_documents(), original_index.num_documents());
        assert_eq!(
            loaded_index.vocabulary_size(),
            original_index.vocabulary_size()
        );
    }

    #[test]
    fn test_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let result = load_bm25_index(temp_dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_metadata() {
        let temp_dir = TempDir::new().unwrap();
        let base_root = temp_dir.path();

        let index = create_test_index();
        save_bm25_index(&index, base_root).unwrap();

        let meta = load_bm25_meta(base_root).unwrap().unwrap();

        assert_eq!(meta.version, Bm25IndexMeta::CURRENT_VERSION);
        assert_eq!(meta.stats.num_documents, 3);
        assert!(meta.built_at > 0);
    }

    #[test]
    fn test_delete_index() {
        let temp_dir = TempDir::new().unwrap();
        let base_root = temp_dir.path();

        let index = create_test_index();
        save_bm25_index(&index, base_root).unwrap();

        assert!(bm25_index_exists(base_root));

        delete_bm25_index(base_root).unwrap();

        assert!(!bm25_index_exists(base_root));
    }

    #[test]
    fn test_search_after_reload() {
        let temp_dir = TempDir::new().unwrap();
        let base_root = temp_dir.path();

        let original_index = create_test_index();
        save_bm25_index(&original_index, base_root).unwrap();

        let loaded_index = load_bm25_index(base_root).unwrap().unwrap();

        // Search should work on loaded index
        let results = loaded_index.search("rust", 10);
        assert!(!results.is_empty());

        let doc_ids: Vec<_> = results.iter().map(|r| r.doc_id.as_str()).collect();
        assert!(doc_ids.contains(&"doc2") || doc_ids.contains(&"doc3"));
    }
}
