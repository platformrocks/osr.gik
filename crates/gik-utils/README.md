# gik-utils

> Utility functions for GIK—URL fetching and HTML parsing.

## Overview

`gik-utils` is a utility library crate that provides supporting functions for GIK. Currently, it focuses on URL fetching and HTML-to-Markdown conversion, isolating the heavy `headless_chrome` dependency from other crates. This crate is used by `gik-core` when indexing web content via `gik add --url`.

## Goals

- **Dependency isolation**: Keep headless Chrome and web-related dependencies out of `gik-core`
- **Simple API**: Provide straightforward functions for common utility operations
- **JS rendering support**: Handle JavaScript-rendered pages via headless browser

## Features

- Fetches web pages using headless Chrome for full JavaScript rendering
- Converts HTML content to clean Markdown format
- Handles navigation, waiting for page load, and content extraction

## Architecture

### Module Overview

```
src/
├── lib.rs              # UtilsError type, re-exports
└── url.rs              # fetch_url_as_markdown() implementation
```

### Key Types

| Type | Role |
|------|------|
| `UtilsError` | Error type for utility operations |

### Key Functions

**`fetch_url_as_markdown`**:
```rust
/// Fetches a URL using headless Chrome and converts the page content to Markdown.
///
/// This function:
/// 1. Launches a headless Chrome instance
/// 2. Navigates to the URL
/// 3. Waits for the page to load (including JS rendering)
/// 4. Extracts the HTML content
/// 5. Converts HTML to Markdown
pub fn fetch_url_as_markdown(url: &str) -> Result<String, UtilsError>
```

## Dependencies

### External

| Crate | Purpose |
|-------|---------|
| `headless_chrome` | Headless Chrome browser automation |
| `reqwest` | HTTP client (blocking) |
| `html2md` | HTML to Markdown conversion |
| `thiserror` | Error derive macro |

## Usage

### Fetching a URL

```rust
use gik_utils::fetch_url_as_markdown;

let markdown = fetch_url_as_markdown("https://example.com/docs")?;
println!("{}", markdown);
```

### In GIK Context

This crate is typically used by `gik-core` when processing URL sources:

```bash
# Add a web page to the knowledge base
gik add https://docs.example.com/guide

# The URL is fetched, converted to Markdown, and indexed
gik commit -m "Add documentation"
```

## Requirements

- **Chrome/Chromium**: headless_chrome requires Chrome or Chromium to be installed on the system
- The browser is launched in headless mode automatically

## Error Handling

```rust
#[derive(Error, Debug)]
pub enum UtilsError {
    #[error("Failed to launch browser: {0}")]
    BrowserLaunch(String),
    
    #[error("Failed to navigate to URL: {0}")]
    Navigation(String),
    
    #[error("Failed to extract content: {0}")]
    ContentExtraction(String),
}
```

## Testing

```bash
# Run tests (may require Chrome installed)
cargo test -p gik-utils
```

## Versioning

This crate follows the workspace version defined in the root `Cargo.toml`.
See [CHANGELOG.md](./CHANGELOG.md) for version history.

## Related Documentation

- [Crates Overview](../../.guided/architecture/crates-overview.md) — All crates in the workspace
- [Architecture Document](../../docs/5-ARCH.md) — Global architecture view
- [gik-core README](../gik-core/README.md) — How core uses this crate
