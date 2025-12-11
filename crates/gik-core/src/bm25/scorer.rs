//! BM25 scoring algorithm.
//!
//! Implements the Okapi BM25 scoring function:
//!
//! ```text
//! score(D, Q) = Σ IDF(q_i) * (f(q_i, D) * (k1 + 1)) / (f(q_i, D) + k1 * (1 - b + b * |D| / avgdl))
//! ```
//!
//! Where:
//! - f(q_i, D) = frequency of query term q_i in document D
//! - |D| = document length (in tokens)
//! - avgdl = average document length in the corpus
//! - k1 = term frequency saturation parameter (default: 1.2)
//! - b = document length normalization parameter (default: 0.75)
//! - IDF = inverse document frequency

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// BM25 scoring parameters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Encode, Decode)]
pub struct Bm25Params {
    /// Term frequency saturation parameter.
    /// Higher values give more weight to term frequency.
    /// Default: 1.2
    pub k1: f32,

    /// Document length normalization parameter.
    /// 0 = no normalization, 1 = full normalization.
    /// Default: 0.75
    pub b: f32,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// Calculate the IDF (Inverse Document Frequency) for a term.
///
/// Uses the smoothed IDF formula:
/// ```text
/// IDF(t) = ln((N - df(t) + 0.5) / (df(t) + 0.5) + 1)
/// ```
///
/// Where:
/// - N = total number of documents
/// - df(t) = number of documents containing term t
///
/// This formula ensures IDF is always positive and handles edge cases gracefully.
#[inline]
pub fn idf(num_docs: usize, doc_freq: usize) -> f32 {
    let n = num_docs as f32;
    let df = doc_freq as f32;
    ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
}

/// Calculate the BM25 score contribution for a single term.
///
/// Returns the score contribution of a query term to a document's total score.
///
/// # Arguments
///
/// * `term_freq` - Frequency of the term in the document
/// * `doc_len` - Length of the document (in tokens)
/// * `avg_doc_len` - Average document length in the corpus
/// * `idf_value` - Pre-computed IDF value for the term
/// * `params` - BM25 parameters (k1, b)
#[inline]
pub fn bm25_term_score(
    term_freq: usize,
    doc_len: usize,
    avg_doc_len: f32,
    idf_value: f32,
    params: &Bm25Params,
) -> f32 {
    let tf = term_freq as f32;
    let dl = doc_len as f32;
    let k1 = params.k1;
    let b = params.b;

    // BM25 formula
    let numerator = tf * (k1 + 1.0);
    let denominator = tf + k1 * (1.0 - b + b * dl / avg_doc_len);

    idf_value * numerator / denominator
}

/// Calculate the complete BM25 score for a document given query terms.
///
/// # Arguments
///
/// * `query_terms` - Slice of (term, idf_value) pairs for query terms
/// * `doc_term_freqs` - Function that returns term frequency in the document
/// * `doc_len` - Length of the document (in tokens)
/// * `avg_doc_len` - Average document length in the corpus
/// * `params` - BM25 parameters
pub fn bm25_score<F>(
    query_terms: &[(&str, f32)], // (term, idf)
    doc_term_freqs: F,
    doc_len: usize,
    avg_doc_len: f32,
    params: &Bm25Params,
) -> f32
where
    F: Fn(&str) -> usize,
{
    query_terms
        .iter()
        .map(|(term, idf_value)| {
            let tf = doc_term_freqs(term);
            if tf == 0 {
                0.0
            } else {
                bm25_term_score(tf, doc_len, avg_doc_len, *idf_value, params)
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idf_common_term() {
        // Term appears in most documents -> low IDF
        let idf_val = idf(1000, 900);
        assert!(idf_val < 0.5);
        assert!(idf_val > 0.0); // Should still be positive
    }

    #[test]
    fn test_idf_rare_term() {
        // Term appears in few documents -> high IDF
        let idf_val = idf(1000, 10);
        assert!(idf_val > 3.0);
    }

    #[test]
    fn test_idf_unique_term() {
        // Term appears in only one document -> highest IDF
        let idf_val = idf(1000, 1);
        assert!(idf_val > 5.0);
    }

    #[test]
    fn test_idf_edge_case_zero_docs() {
        // Term appears in no documents
        let idf_val = idf(1000, 0);
        assert!(idf_val > 0.0);
        assert!(idf_val.is_finite());
    }

    #[test]
    fn test_idf_edge_case_all_docs() {
        // Term appears in all documents
        let idf_val = idf(1000, 1000);
        assert!(idf_val > 0.0); // Still positive due to smoothing
    }

    #[test]
    fn test_bm25_term_score_basic() {
        let params = Bm25Params::default();
        let idf_val = idf(100, 10);

        // High term frequency in average-length document
        let score = bm25_term_score(5, 100, 100.0, idf_val, &params);
        assert!(score > 0.0);
    }

    #[test]
    fn test_bm25_length_normalization() {
        let params = Bm25Params::default();
        let idf_val = idf(100, 10);

        // Same term frequency, different document lengths
        let score_short = bm25_term_score(3, 50, 100.0, idf_val, &params);
        let score_long = bm25_term_score(3, 200, 100.0, idf_val, &params);

        // Shorter document should score higher (term is more significant)
        assert!(score_short > score_long);
    }

    #[test]
    fn test_bm25_tf_saturation() {
        let params = Bm25Params::default();
        let idf_val = idf(100, 10);

        // Increasing term frequency
        let score_1 = bm25_term_score(1, 100, 100.0, idf_val, &params);
        let score_5 = bm25_term_score(5, 100, 100.0, idf_val, &params);
        let score_10 = bm25_term_score(10, 100, 100.0, idf_val, &params);
        let score_100 = bm25_term_score(100, 100, 100.0, idf_val, &params);

        // Scores should increase but saturate
        assert!(score_5 > score_1);
        assert!(score_10 > score_5);
        assert!(score_100 > score_10);

        // But the increase should slow down (saturation)
        let increase_1_5 = score_5 - score_1;
        let increase_10_100 = score_100 - score_10;
        // The increase from 10 to 100 should be less than linear
        assert!(increase_10_100 / 90.0 < increase_1_5 / 4.0);
    }

    #[test]
    fn test_bm25_score_multi_term() {
        let params = Bm25Params::default();

        // Query: "rust programming"
        let query_terms = vec![("rust", idf(100, 5)), ("program", idf(100, 20))];

        // Document term frequencies
        let doc_tf = |term: &str| match term {
            "rust" => 3,
            "program" => 2,
            _ => 0,
        };

        let score = bm25_score(&query_terms, doc_tf, 100, 100.0, &params);
        assert!(score > 0.0);
    }

    #[test]
    fn test_bm25_score_no_match() {
        let params = Bm25Params::default();

        let query_terms = vec![("rust", idf(100, 5))];

        // Document has no matching terms
        let doc_tf = |_: &str| 0;

        let score = bm25_score(&query_terms, doc_tf, 100, 100.0, &params);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_params_effect() {
        let idf_val = idf(100, 10);

        // Default params
        let default_params = Bm25Params::default();
        let score_default = bm25_term_score(3, 200, 100.0, idf_val, &default_params);

        // No length normalization (b=0)
        let no_norm_params = Bm25Params { k1: 1.2, b: 0.0 };
        let score_no_norm = bm25_term_score(3, 200, 100.0, idf_val, &no_norm_params);

        // Without length normalization, long documents aren't penalized
        assert!(score_no_norm > score_default);
    }
}
