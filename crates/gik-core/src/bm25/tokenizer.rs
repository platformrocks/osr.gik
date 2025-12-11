//! Unicode-aware tokenizer with stemming for BM25.
//!
//! Provides text preprocessing for BM25 indexing:
//! - Unicode normalization and segmentation
//! - Case folding (lowercasing)
//! - Porter stemming (English)
//! - Stop word removal
//! - Minimum token length filtering

use bincode::{Decode, Encode};
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use unicode_segmentation::UnicodeSegmentation;

/// Tokenizer configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct TokenizerConfig {
    /// Apply Porter stemming to tokens.
    pub stemming: bool,
    /// Remove common stop words.
    pub remove_stopwords: bool,
    /// Minimum token length to include.
    pub min_token_length: usize,
}

impl Default for TokenizerConfig {
    fn default() -> Self {
        Self {
            stemming: true,
            remove_stopwords: true,
            min_token_length: 2,
        }
    }
}

/// Unicode-aware tokenizer with optional stemming.
pub struct Tokenizer {
    config: TokenizerConfig,
    stemmer: Option<Stemmer>,
    stopwords: HashSet<&'static str>,
}

impl Tokenizer {
    /// Create a new tokenizer with the given configuration.
    pub fn new(config: TokenizerConfig) -> Self {
        let stemmer = if config.stemming {
            Some(Stemmer::create(Algorithm::English))
        } else {
            None
        };

        Self {
            config,
            stemmer,
            stopwords: Self::default_stopwords(),
        }
    }

    /// Tokenize text into a vector of processed tokens.
    ///
    /// Processing steps:
    /// 1. Unicode word segmentation
    /// 2. Lowercase normalization
    /// 3. Filter non-alphabetic tokens
    /// 4. Minimum length filtering
    /// 5. Stop word removal (if enabled)
    /// 6. Porter stemming (if enabled)
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        text.unicode_words()
            .filter_map(|word| self.process_token(word))
            .collect()
    }

    /// Tokenize and return term frequencies.
    pub fn tokenize_with_tf(&self, text: &str) -> Vec<(String, usize)> {
        use std::collections::HashMap;

        let mut tf: HashMap<String, usize> = HashMap::new();
        for token in self.tokenize(text) {
            *tf.entry(token).or_insert(0) += 1;
        }

        tf.into_iter().collect()
    }

    /// Process a single token through the pipeline.
    fn process_token(&self, word: &str) -> Option<String> {
        // Lowercase
        let lower = word.to_lowercase();

        // Filter non-alphabetic (keep alphanumeric for code identifiers)
        if !lower.chars().any(|c| c.is_alphabetic()) {
            return None;
        }

        // Minimum length
        if lower.len() < self.config.min_token_length {
            return None;
        }

        // Stop word removal
        if self.config.remove_stopwords && self.stopwords.contains(lower.as_str()) {
            return None;
        }

        // Stemming
        let token = if let Some(ref stemmer) = self.stemmer {
            stemmer.stem(&lower).to_string()
        } else {
            lower
        };

        // Filter again after stemming (some stems become too short)
        if token.len() < self.config.min_token_length {
            return None;
        }

        Some(token)
    }

    /// Default English stop words for code search.
    ///
    /// This is a curated list that includes common English words
    /// but excludes words that might be meaningful in code contexts
    /// (like "do", "return", "for", "if", etc.).
    fn default_stopwords() -> HashSet<&'static str> {
        [
            // Articles
            "a", "an", "the", // Prepositions
            "in", "on", "at", "to", "of", "with", "by", "from", "as", "into", "through", "during",
            "before", "after", "above", "below", "between", "under", "over", "out", "up", "down",
            "off", // Conjunctions
            "and", "or", "but", "nor", "so", "yet", // Pronouns
            "i", "you", "he", "she", "it", "we", "they", "me", "him", "her", "us", "them", "my",
            "your", "his", "its", "our", "their", "this", "that", "these", "those", "which", "who",
            "whom", "whose", "what", "where", "when", "how", "why",
            // Common verbs (but keep code-relevant ones)
            "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "having",
            "does", "did", "doing", "will", "would", "could", "should", "may", "might", "must",
            "shall", "can", "need", "dare", "ought", // Other common words
            "not", "no", "yes", "all", "any", "both", "each", "few", "more", "most", "other",
            "some", "such", "than", "too", "very", "just", "also", "only", "own", "same", "then",
            "there", "here", "now", "always", "never", "ever", // Question/relative
            "about", "whether",
        ]
        .into_iter()
        .collect()
    }

    /// Get the number of stop words.
    #[cfg(test)]
    pub fn stopword_count(&self) -> usize {
        self.stopwords.len()
    }
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new(TokenizerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokenization() {
        let tokenizer = Tokenizer::default();
        let tokens = tokenizer.tokenize("Hello World");

        assert_eq!(tokens.len(), 2);
        // With stemming: "hello" -> "hello", "world" -> "world"
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
    }

    #[test]
    fn test_stopword_removal() {
        let tokenizer = Tokenizer::default();
        let tokens = tokenizer.tokenize("the quick brown fox");

        // "the" should be removed
        assert!(!tokens.iter().any(|t| t == "the"));
        assert!(tokens.contains(&"quick".to_string()));
    }

    #[test]
    fn test_stemming() {
        let tokenizer = Tokenizer::default();
        let tokens = tokenizer.tokenize("running runs");

        // Both should stem to "run"
        for token in &tokens {
            assert_eq!(token, "run");
        }
    }

    #[test]
    fn test_no_stemming() {
        let config = TokenizerConfig {
            stemming: false,
            ..Default::default()
        };
        let tokenizer = Tokenizer::new(config);
        let tokens = tokenizer.tokenize("running runs runner");

        assert!(tokens.contains(&"running".to_string()));
        assert!(tokens.contains(&"runs".to_string()));
        assert!(tokens.contains(&"runner".to_string()));
    }

    #[test]
    fn test_code_identifiers() {
        let tokenizer = Tokenizer::default();
        let tokens = tokenizer.tokenize("function calculateTotal getUser");

        assert!(tokens.iter().any(|t| t == "function"));
        assert!(tokens.iter().any(|t| t == "calculatetot")); // stemmed
        assert!(tokens.iter().any(|t| t == "getus")); // stemmed
    }

    #[test]
    fn test_min_length_filtering() {
        let tokenizer = Tokenizer::default();
        let tokens = tokenizer.tokenize("a b c de foo bar");

        // "a", "b", "c" should be filtered (< 2 chars)
        assert!(!tokens.contains(&"a".to_string()));
        assert!(!tokens.contains(&"b".to_string()));
        assert!(!tokens.contains(&"c".to_string()));
        // "de", "foo", "bar" should pass
        assert!(tokens.contains(&"de".to_string()));
        assert!(tokens.contains(&"foo".to_string()));
        assert!(tokens.contains(&"bar".to_string()));
    }

    #[test]
    fn test_term_frequencies() {
        let tokenizer = Tokenizer::default();
        let tf = tokenizer.tokenize_with_tf("foo bar foo baz foo");

        let tf_map: std::collections::HashMap<_, _> = tf.into_iter().collect();
        assert_eq!(tf_map.get("foo"), Some(&3));
        assert_eq!(tf_map.get("bar"), Some(&1));
        assert_eq!(tf_map.get("baz"), Some(&1));
    }

    #[test]
    fn test_unicode_text() {
        let tokenizer = Tokenizer::default();
        let tokens = tokenizer.tokenize("café naïve résumé");

        // Should handle accented characters
        assert!(tokens.iter().any(|t| t.contains("caf")));
        assert!(tokens
            .iter()
            .any(|t| t.contains("naiv") || t.contains("naïv")));
    }

    #[test]
    fn test_mixed_content() {
        let tokenizer = Tokenizer::default();
        let tokens = tokenizer.tokenize("The function returns 42 items from database");

        // "the" removed (stopword)
        // numbers alone are filtered
        // "returns" -> "return" (stemmed)
        // "items" -> "item" (stemmed)
        // "database" -> "databas" (stemmed)
        assert!(!tokens.contains(&"the".to_string()));
        assert!(tokens.iter().any(|t| t == "function"));
        assert!(tokens.iter().any(|t| t == "return"));
        assert!(tokens.iter().any(|t| t == "item"));
        assert!(tokens.iter().any(|t| t == "databas"));
    }
}
