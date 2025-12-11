//! Utility functions for GIK
//!
//! This crate provides utility functions like URL fetching with headless Chrome
//! and HTML to Markdown conversion. It isolates heavy dependencies (headless_chrome)
//! to avoid slowing down the main build.

use thiserror::Error;

pub mod url;

#[derive(Debug, Error)]
pub enum UtilsError {
    #[error("Failed to fetch URL: {0}")]
    FetchError(String),
    
    #[error("Failed to extract content: {0}")]
    ExtractionError(String),
    
    #[error("Content is empty")]
    EmptyContent,
}

#[cfg(test)]
mod url_tests {
    use super::url::fetch_url_as_markdown;
    
    #[test]
    #[ignore] // Requires network
    fn test_nextjs_docs_extraction() {
        let url = "https://nextjs.org/docs/app/getting-started/upgrading";
        let md = fetch_url_as_markdown(url).expect("Should fetch");
        
        println!("=== FIRST 1000 CHARS ===");
        println!("{}", &md[..md.len().min(1000)]);
        println!("\n=== TOTAL LENGTH ===");
        println!("{} chars", md.len());
        
        // The content should contain "Upgrading" and "next upgrade"
        assert!(md.contains("Upgrading"), "Should contain 'Upgrading'");
        assert!(md.contains("upgrade"), "Should contain 'upgrade' command");
        
        // The content should NOT start with menu items  
        assert!(!md.starts_with("Menu"), "Should not start with Menu navigation");
    }
}
