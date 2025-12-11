//! BM25 Inverted Index.
//!
//! Provides an inverted index optimized for BM25 scoring:
//! - Term → document postings with term frequencies
//! - Pre-computed document lengths and IDF values
//! - Fast query-time scoring

use std::collections::HashMap;

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use super::scorer::{bm25_term_score, idf, Bm25Params};
use super::tokenizer::{Tokenizer, TokenizerConfig};
use super::{Bm25Config, Bm25SearchResult};

/// Statistics for a single document in the index.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct DocumentStats {
    /// Number of tokens in the document.
    pub length: usize,
    /// Original document ID (chunk_id).
    pub doc_id: String,
}

/// Posting entry: document index and term frequency.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct Posting {
    /// Index into the documents array.
    pub doc_idx: usize,
    /// Term frequency in this document.
    pub term_freq: usize,
}

/// BM25 Inverted Index.
///
/// Stores:
/// - Vocabulary: term → (term_id, document frequency, postings)
/// - Documents: array of document stats
/// - Pre-computed average document length
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct Bm25Index {
    /// BM25 parameters.
    params: Bm25Params,
    /// Tokenizer configuration (for query tokenization).
    tokenizer_config: TokenizerConfig,
    /// Term → postings list.
    /// Each entry: (document_frequency, postings)
    inverted_index: HashMap<String, (usize, Vec<Posting>)>,
    /// Document statistics indexed by internal doc_idx.
    documents: Vec<DocumentStats>,
    /// Average document length.
    avg_doc_len: f32,
    /// Total number of tokens in the corpus.
    total_tokens: usize,
}

impl Bm25Index {
    /// Create a new empty BM25 index.
    pub fn new(config: Bm25Config) -> Self {
        let tokenizer_config = TokenizerConfig {
            stemming: config.stemming,
            remove_stopwords: config.remove_stopwords,
            min_token_length: config.min_token_length,
        };

        Self {
            params: Bm25Params {
                k1: config.k1,
                b: config.b,
            },
            tokenizer_config,
            inverted_index: HashMap::new(),
            documents: Vec::new(),
            avg_doc_len: 0.0,
            total_tokens: 0,
        }
    }

    /// Add a document to the index.
    ///
    /// # Arguments
    ///
    /// * `doc_id` - Unique document identifier (e.g., chunk_id)
    /// * `text` - Document text content
    ///
    /// # Returns
    ///
    /// The internal document index assigned to this document.
    pub fn add_document(&mut self, doc_id: String, text: &str) -> usize {
        let tokenizer = Tokenizer::new(self.tokenizer_config.clone());
        let tokens = tokenizer.tokenize(text);
        let doc_len = tokens.len();

        // Compute term frequencies for this document
        let mut term_freqs: HashMap<String, usize> = HashMap::new();
        for token in tokens {
            *term_freqs.entry(token).or_insert(0) += 1;
        }

        // Add document to documents array
        let doc_idx = self.documents.len();
        self.documents.push(DocumentStats {
            length: doc_len,
            doc_id,
        });

        // Update inverted index
        for (term, tf) in term_freqs {
            let entry = self.inverted_index.entry(term).or_insert((0, Vec::new()));
            entry.0 += 1; // Increment document frequency
            entry.1.push(Posting {
                doc_idx,
                term_freq: tf,
            });
        }

        // Update corpus statistics
        self.total_tokens += doc_len;
        self.avg_doc_len = self.total_tokens as f32 / self.documents.len() as f32;

        doc_idx
    }

    /// Build the index from an iterator of (doc_id, text) pairs.
    ///
    /// This is more efficient than calling `add_document` repeatedly
    /// as it batches the updates.
    pub fn build_from_iter<I, S1, S2>(&mut self, documents: I)
    where
        I: Iterator<Item = (S1, S2)>,
        S1: Into<String>,
        S2: AsRef<str>,
    {
        for (doc_id, text) in documents {
            self.add_document(doc_id.into(), text.as_ref());
        }
    }

    /// Search the index for documents matching the query.
    ///
    /// # Arguments
    ///
    /// * `query` - Query text
    /// * `top_k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score (descending).
    pub fn search(&self, query: &str, top_k: usize) -> Vec<Bm25SearchResult> {
        if self.documents.is_empty() {
            return Vec::new();
        }

        let tokenizer = Tokenizer::new(self.tokenizer_config.clone());
        let query_tokens = tokenizer.tokenize(query);

        if query_tokens.is_empty() {
            return Vec::new();
        }

        // Collect query terms with their IDF values
        let num_docs = self.documents.len();
        let query_terms: Vec<(&str, f32)> = query_tokens
            .iter()
            .filter_map(|term| {
                self.inverted_index.get(term).map(|(df, _)| {
                    let idf_val = idf(num_docs, *df);
                    (term.as_str(), idf_val)
                })
            })
            .collect();

        if query_terms.is_empty() {
            return Vec::new();
        }

        // Score all documents that contain at least one query term
        let mut scores: HashMap<usize, f32> = HashMap::new();

        for (term, idf_val) in &query_terms {
            if let Some((_, postings)) = self.inverted_index.get(*term) {
                for posting in postings {
                    let doc_stats = &self.documents[posting.doc_idx];
                    let term_score = bm25_term_score(
                        posting.term_freq,
                        doc_stats.length,
                        self.avg_doc_len,
                        *idf_val,
                        &self.params,
                    );
                    *scores.entry(posting.doc_idx).or_insert(0.0) += term_score;
                }
            }
        }

        // Sort by score and take top_k
        let mut scored_docs: Vec<(usize, f32)> = scores.into_iter().collect();
        scored_docs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored_docs
            .into_iter()
            .take(top_k)
            .enumerate()
            .map(|(rank, (doc_idx, score))| Bm25SearchResult {
                doc_id: self.documents[doc_idx].doc_id.clone(),
                score,
                rank: rank + 1, // 1-indexed
            })
            .collect()
    }

    /// Get the number of documents in the index.
    pub fn num_documents(&self) -> usize {
        self.documents.len()
    }

    /// Get the number of unique terms in the vocabulary.
    pub fn vocabulary_size(&self) -> usize {
        self.inverted_index.len()
    }

    /// Get the average document length.
    pub fn avg_doc_length(&self) -> f32 {
        self.avg_doc_len
    }

    /// Get document frequency for a term.
    pub fn document_frequency(&self, term: &str) -> usize {
        self.inverted_index
            .get(term)
            .map(|(df, _)| *df)
            .unwrap_or(0)
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// Get index statistics for debugging/logging.
    pub fn stats(&self) -> Bm25IndexStats {
        Bm25IndexStats {
            num_documents: self.documents.len(),
            vocabulary_size: self.inverted_index.len(),
            total_tokens: self.total_tokens,
            avg_doc_length: self.avg_doc_len,
        }
    }
}

/// Statistics about the BM25 index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25IndexStats {
    /// Number of documents indexed.
    pub num_documents: usize,
    /// Number of unique terms in vocabulary.
    pub vocabulary_size: usize,
    /// Total tokens across all documents.
    pub total_tokens: usize,
    /// Average document length.
    pub avg_doc_length: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_index() -> Bm25Index {
        let mut index = Bm25Index::new(Bm25Config::default());

        // Add some test documents
        index.add_document(
            "doc1".to_string(),
            "The quick brown fox jumps over the lazy dog",
        );
        index.add_document(
            "doc2".to_string(),
            "Rust programming language is fast and safe",
        );
        index.add_document(
            "doc3".to_string(),
            "The fox is quick and cunning in the forest",
        );
        index.add_document(
            "doc4".to_string(),
            "Rust language features include ownership and borrowing",
        );

        index
    }

    #[test]
    fn test_add_document() {
        let mut index = Bm25Index::new(Bm25Config::default());
        let doc_idx = index.add_document("doc1".to_string(), "hello world");

        assert_eq!(doc_idx, 0);
        assert_eq!(index.num_documents(), 1);
        assert!(index.vocabulary_size() > 0);
    }

    #[test]
    fn test_search_basic() {
        let index = create_test_index();

        // Search for "fox"
        let results = index.search("fox", 10);

        assert!(!results.is_empty());
        // doc1 and doc3 contain "fox"
        let doc_ids: Vec<_> = results.iter().map(|r| r.doc_id.as_str()).collect();
        assert!(doc_ids.contains(&"doc1") || doc_ids.contains(&"doc3"));
    }

    #[test]
    fn test_search_rust() {
        let index = create_test_index();

        // Search for "Rust programming"
        let results = index.search("Rust programming", 10);

        assert!(!results.is_empty());
        // doc2 and doc4 are about Rust
        let top_result = &results[0];
        assert!(top_result.doc_id == "doc2" || top_result.doc_id == "doc4");
    }

    #[test]
    fn test_search_no_match() {
        let index = create_test_index();

        // Search for non-existent term
        let results = index.search("nonexistentterm12345", 10);

        assert!(results.is_empty());
    }

    #[test]
    fn test_ranking_order() {
        let mut index = Bm25Index::new(Bm25Config::default());

        // Document with more occurrences of "rust" should rank higher
        index.add_document("many_rust".to_string(), "rust rust rust rust programming");
        index.add_document("one_rust".to_string(), "rust programming");
        index.add_document("no_rust".to_string(), "python programming");

        let results = index.search("rust", 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].doc_id, "many_rust");
        assert_eq!(results[1].doc_id, "one_rust");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_build_from_iter() {
        let mut index = Bm25Index::new(Bm25Config::default());

        let docs = vec![
            ("doc1", "hello world"),
            ("doc2", "world peace"),
            ("doc3", "peace and quiet"),
        ];

        index.build_from_iter(docs.into_iter());

        assert_eq!(index.num_documents(), 3);
    }

    #[test]
    fn test_stats() {
        let index = create_test_index();
        let stats = index.stats();

        assert_eq!(stats.num_documents, 4);
        assert!(stats.vocabulary_size > 0);
        assert!(stats.total_tokens > 0);
        assert!(stats.avg_doc_length > 0.0);
    }

    #[test]
    fn test_document_frequency() {
        let index = create_test_index();

        // "rust" should appear in 2 documents (after stemming)
        let df = index.document_frequency("rust");
        assert!(df >= 2);

        // Non-existent term
        let df_none = index.document_frequency("nonexistent");
        assert_eq!(df_none, 0);
    }

    #[test]
    fn test_top_k_limit() {
        let mut index = Bm25Index::new(Bm25Config::default());

        // Add many documents with "test"
        for i in 0..100 {
            index.add_document(format!("doc{}", i), "test document content");
        }

        let results = index.search("test", 5);
        assert_eq!(results.len(), 5);

        let results_all = index.search("test", 1000);
        assert_eq!(results_all.len(), 100);
    }

    #[test]
    fn test_rank_values() {
        let index = create_test_index();
        let results = index.search("fox", 10);

        // Ranks should be 1-indexed and sequential
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result.rank, i + 1);
        }
    }
}
