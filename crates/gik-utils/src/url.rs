//! URL fetching utilities using headless Chrome

use crate::UtilsError;
use std::time::Duration;

/// Fetch content from a URL using headless Chrome and convert to Markdown.
///
/// This function:
/// 1. Launches a headless Chrome browser
/// 2. Navigates to the URL and waits for JavaScript rendering
/// 3. Extracts the fully-rendered HTML from main content area
/// 4. Converts HTML to clean Markdown using htmd
///
/// This approach works well for JavaScript-heavy SPAs (React, Next.js, etc.)
/// where content is dynamically rendered and not present in initial HTML.
pub fn fetch_url_as_markdown(url: &str) -> Result<String, UtilsError> {
    use headless_chrome::{Browser, LaunchOptions};
    
    // Launch headless Chrome
    let browser = Browser::new(LaunchOptions {
        headless: true,
        ..Default::default()
    })
    .map_err(|e| UtilsError::FetchError(format!("Failed to launch browser: {}", e)))?;
    
    // Navigate to URL
    let tab = browser
        .new_tab()
        .map_err(|e| UtilsError::FetchError(format!("Failed to create tab: {}", e)))?;
    
    tab.navigate_to(url)
        .map_err(|e| UtilsError::FetchError(format!("Failed to navigate: {}", e)))?;
    
    // Wait for page to fully load
    tab.wait_until_navigated()
        .map_err(|e| UtilsError::FetchError(format!("Failed to wait for navigation: {}", e)))?;
    
    // Shorter wait for dynamic content (React hydration, etc.)
    // Most SPAs are ready within 1 second
    std::thread::sleep(Duration::from_millis(800));
    
    // First, try to remove common navigation elements to get cleaner content
    // This helps with sites like Next.js docs that have large sidebars
    let remove_selectors = [
        "nav",
        "aside",
        "header",
        "footer",
        "[role='navigation']",
        ".sidebar",
        ".nav",
        ".menu",
        ".toc",
        ".table-of-contents",
    ];
    
    for selector in remove_selectors {
        // Try to find and remove each navigation element
        let js_remove = format!(
            r#"document.querySelectorAll('{}').forEach(el => el.remove())"#,
            selector
        );
        let _ = tab.evaluate(&js_remove, false);
    }
    
    // Try to extract main content area using common selectors
    // More specific selectors first, then broader ones
    let selectors = vec![
        // Site-specific selectors (Next.js docs, etc.)
        "[data-docs-container]",
        "[data-docs-content]",
        // Documentation-specific selectors
        ".docs-content",
        ".markdown-body",
        ".prose",
        ".documentation",
        ".article-content",
        // Generic content selectors
        "article",
        "main article",
        "main .content",
        "[role='main'] article",
        "main",
        "[role='main']",
        "#content",
        ".content",
        "body", // Fallback to body if no main content found
    ];
    
    let mut content_html = String::new();
    for selector in selectors {
        // Use find_element (non-waiting) instead of wait_for_element for speed
        if let Ok(element) = tab.find_element(selector) {
            if let Ok(html) = element.get_content() {
                // Skip if content is too small (probably wrong element)
                if html.len() > 500 {
                    content_html = html;
                    break;
                }
            }
        }
    }
    
    // If still empty, fallback to full page
    if content_html.is_empty() {
        content_html = tab
            .get_content()
            .map_err(|e| UtilsError::FetchError(format!("Failed to get content: {}", e)))?;
    }
    
    // Convert HTML to Markdown
    let markdown = htmd::convert(&content_html)
        .map_err(|e| UtilsError::ExtractionError(format!("Failed to convert HTML: {}", e)))?;
    
    if markdown.trim().is_empty() {
        return Err(UtilsError::EmptyContent);
    }
    
    Ok(markdown)
}
