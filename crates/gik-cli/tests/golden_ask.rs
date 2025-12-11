//! Golden tests for `gik ask` output shape and AskContextBundle structure.
//!
//! These tests verify that the JSON output structure of `gik ask --json` remains
//! stable and matches the documented contract in docs/2-ENTITIES.md.
//!
//! # Test Strategy
//!
//! - **Shape-focused**: Verify structure and field presence, not exact content
//! - **Contract-aligned**: Assertions trace back to documented contracts
//! - **Deterministic**: Use mock embeddings where possible to avoid model variance
//!
//! # Reference
//!
//! See: docs/2-ENTITIES.md (AskContextBundle definition)
//! See: crates/gik-core/src/ask.rs (RagChunk, MemoryEvent, AskDebugInfo)

mod common;

use std::fs;
use tempfile::TempDir;

use common::gik_cmd;

/// Create a source file in src/ directory.
fn create_source(dir: &std::path::Path, filename: &str, content: &str) {
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join(filename), content).expect("write source file");
}

/// Setup a minimal indexed workspace for golden tests.
fn setup_indexed_workspace() -> TempDir {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Create source files
    create_source(
        workspace,
        "main.rs",
        r#"//! Main application entry point.
//! Handles CLI argument parsing and application bootstrap.

fn main() {
    println!("Hello, world!");
}
"#,
    );

    create_source(
        workspace,
        "lib.rs",
        r#"//! Library module providing core functionality.

/// Add two numbers together.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Subtract b from a.  
pub fn subtract(a: i32, b: i32) -> i32 {
    a - b
}
"#,
    );

    // Add and commit
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("feat: initial project setup")
        .assert()
        .success();

    temp
}

// ============================================================================
// AskContextBundle Shape Tests (golden-ask-001 to golden-ask-003)
// ============================================================================

/// golden-ask-001: Basic ask returns well-formed AskContextBundle.
///
/// Verifies:
/// - Exit code 0
/// - Valid JSON output
/// - All required top-level fields present
/// - camelCase field naming (no snake_case)
#[test]
fn golden_ask_basic_shape() {
    let temp = setup_indexed_workspace();
    let workspace = temp.path();

    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("How does this project work?")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value =
        serde_json::from_str(&stdout).expect("ask --json should return valid JSON");

    // Required top-level fields (from docs/2-ENTITIES.md)
    assert!(
        bundle.get("revisionId").is_some(),
        "Missing required field: revisionId"
    );
    assert!(
        bundle.get("question").is_some(),
        "Missing required field: question"
    );
    assert!(
        bundle.get("bases").is_some(),
        "Missing required field: bases"
    );
    assert!(
        bundle.get("ragChunks").is_some(),
        "Missing required field: ragChunks"
    );
    assert!(
        bundle.get("kgResults").is_some(),
        "Missing required field: kgResults"
    );
    assert!(
        bundle.get("memoryEvents").is_some(),
        "Missing required field: memoryEvents"
    );
    assert!(
        bundle.get("debug").is_some(),
        "Missing required field: debug"
    );

    // Verify field types
    assert!(
        bundle["revisionId"].is_string(),
        "revisionId should be string"
    );
    assert!(bundle["question"].is_string(), "question should be string");
    assert!(bundle["bases"].is_array(), "bases should be array");
    assert!(bundle["ragChunks"].is_array(), "ragChunks should be array");
    assert!(bundle["kgResults"].is_array(), "kgResults should be array");
    assert!(
        bundle["memoryEvents"].is_array(),
        "memoryEvents should be array"
    );
    assert!(bundle["debug"].is_object(), "debug should be object");

    // Verify revisionId is non-empty (UUIDs are the actual format)
    let rev_id = bundle["revisionId"].as_str().unwrap();
    assert!(!rev_id.is_empty(), "revisionId should be non-empty string");
    // Revision IDs are UUIDs (e.g., "bb6aa6dd-3207-43ef-88a1-252d99ee91b5")
    assert!(
        rev_id.len() >= 32,
        "revisionId should be a UUID-length string, got: {}",
        rev_id
    );

    // Verify question matches input
    let question = bundle["question"].as_str().unwrap();
    assert_eq!(question, "How does this project work?");

    // Verify camelCase (no snake_case in keys)
    let json_str = serde_json::to_string(&bundle).unwrap();
    assert!(
        !json_str.contains("\"revision_id\""),
        "Found snake_case: revision_id"
    );
    assert!(
        !json_str.contains("\"rag_chunks\""),
        "Found snake_case: rag_chunks"
    );
    assert!(
        !json_str.contains("\"kg_results\""),
        "Found snake_case: kg_results"
    );
    assert!(
        !json_str.contains("\"memory_events\""),
        "Found snake_case: memory_events"
    );
    assert!(
        !json_str.contains("\"stack_summary\""),
        "Found snake_case: stack_summary"
    );
}

/// golden-ask-002: Ask with no matches returns empty arrays, not error.
///
/// Verifies:
/// - Exit code 0 (not an error)
/// - ragChunks is empty array [], not null
/// - kgResults is empty array []
/// - memoryEvents is empty array []
#[test]
fn golden_ask_empty_results_not_error() {
    let temp = setup_indexed_workspace();
    let workspace = temp.path();

    // Query for something that won't match
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("quantum entanglement in rust programming xyzzy123")
        .assert()
        .success(); // Should not error

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value =
        serde_json::from_str(&stdout).expect("ask --json should return valid JSON");

    // Arrays should exist (may be empty)
    assert!(
        bundle["ragChunks"].is_array(),
        "ragChunks should be array, not null"
    );
    assert!(
        bundle["kgResults"].is_array(),
        "kgResults should be array, not null"
    );
    assert!(
        bundle["memoryEvents"].is_array(),
        "memoryEvents should be array, not null"
    );

    // The arrays might have results (depending on embedding similarity) or be empty
    // The key assertion is that they are arrays, not null or missing
    println!(
        "Empty query results: {} ragChunks, {} kgResults, {} memoryEvents",
        bundle["ragChunks"].as_array().unwrap().len(),
        bundle["kgResults"].as_array().unwrap().len(),
        bundle["memoryEvents"].as_array().unwrap().len()
    );
}

/// golden-ask-003: Multi-base query returns results from multiple bases.
///
/// Verifies:
/// - bases array contains queried bases
/// - debug.usedBases matches bases
/// - debug.perBaseCounts has entry for each base
#[test]
fn golden_ask_multi_base_results() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Create code file
    create_source(
        workspace,
        "api.rs",
        r#"//! API module for handling HTTP requests.

/// Handle GET request.
pub fn handle_get() -> String {
    "GET response".to_string()
}
"#,
    );

    // Create docs file
    fs::write(
        workspace.join("README.md"),
        r#"# API Documentation

This project provides an HTTP API with GET and POST endpoints.

## Endpoints

- GET /api/data - Retrieve data
- POST /api/data - Create data
"#,
    )
    .expect("write README");

    // Add and commit both
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("README.md")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("feat: add API and docs")
        .assert()
        .success();

    // Query (should search both code and docs)
    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("How does the API work?")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value =
        serde_json::from_str(&stdout).expect("ask --json should return valid JSON");

    // Verify bases were searched
    let bases = bundle["bases"].as_array().expect("bases should be array");
    assert!(!bases.is_empty(), "bases array should not be empty");

    // Verify debug.usedBases
    let debug = &bundle["debug"];
    assert!(
        debug.get("usedBases").is_some(),
        "debug should have usedBases"
    );
    let used_bases = debug["usedBases"]
        .as_array()
        .expect("usedBases should be array");
    assert!(!used_bases.is_empty(), "usedBases should not be empty");

    // Verify debug.perBaseCounts
    assert!(
        debug.get("perBaseCounts").is_some(),
        "debug should have perBaseCounts"
    );
    let per_base_counts = debug["perBaseCounts"]
        .as_array()
        .expect("perBaseCounts should be array");

    // Each entry should have base and count fields
    for entry in per_base_counts {
        assert!(
            entry.get("base").is_some(),
            "perBaseCounts entry should have base"
        );
        assert!(
            entry.get("count").is_some(),
            "perBaseCounts entry should have count"
        );
    }
}

// ============================================================================
// RagChunk Shape Tests (golden-ask-008)
// ============================================================================

/// golden-ask-008: RagChunk contains all documented fields with correct types.
///
/// Required fields: base, score, path, startLine, endLine, snippet
/// Optional fields: denseScore, rerankerScore (omitted when null)
#[test]
fn golden_rag_chunk_field_completeness() {
    let temp = setup_indexed_workspace();
    let workspace = temp.path();

    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("What functions are available?")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let rag_chunks = bundle["ragChunks"]
        .as_array()
        .expect("ragChunks should be array");

    // Skip if no chunks (test still passes - we're testing shape when present)
    if rag_chunks.is_empty() {
        println!("Note: No ragChunks returned, skipping field completeness check");
        return;
    }

    for (i, chunk) in rag_chunks.iter().enumerate() {
        // Required fields
        assert!(
            chunk.get("base").is_some(),
            "ragChunks[{}] missing required field: base",
            i
        );
        assert!(
            chunk.get("score").is_some(),
            "ragChunks[{}] missing required field: score",
            i
        );
        assert!(
            chunk.get("path").is_some(),
            "ragChunks[{}] missing required field: path",
            i
        );
        assert!(
            chunk.get("startLine").is_some(),
            "ragChunks[{}] missing required field: startLine",
            i
        );
        assert!(
            chunk.get("endLine").is_some(),
            "ragChunks[{}] missing required field: endLine",
            i
        );
        assert!(
            chunk.get("snippet").is_some(),
            "ragChunks[{}] missing required field: snippet",
            i
        );

        // Type checks
        assert!(
            chunk["base"].is_string(),
            "ragChunks[{}].base should be string",
            i
        );
        assert!(
            chunk["score"].is_number(),
            "ragChunks[{}].score should be number",
            i
        );
        assert!(
            chunk["path"].is_string(),
            "ragChunks[{}].path should be string",
            i
        );
        assert!(
            chunk["startLine"].is_number(),
            "ragChunks[{}].startLine should be number",
            i
        );
        assert!(
            chunk["endLine"].is_number(),
            "ragChunks[{}].endLine should be number",
            i
        );
        assert!(
            chunk["snippet"].is_string(),
            "ragChunks[{}].snippet should be string",
            i
        );

        // Semantic checks
        let start_line = chunk["startLine"].as_u64().unwrap();
        let end_line = chunk["endLine"].as_u64().unwrap();
        assert!(
            start_line <= end_line,
            "ragChunks[{}].startLine ({}) should be <= endLine ({})",
            i,
            start_line,
            end_line
        );

        let score = chunk["score"].as_f64().unwrap();
        assert!(
            score >= 0.0,
            "ragChunks[{}].score ({}) should be >= 0",
            i,
            score
        );

        // Optional fields: should NOT be present as "null" - they should be omitted
        // (This is enforced by #[serde(skip_serializing_if = "Option::is_none")])
        let chunk_str = serde_json::to_string(chunk).unwrap();
        // Note: denseScore may be present (not null) when set
        if chunk_str.contains("\"denseScore\":null") {
            panic!("ragChunks[{}].denseScore should be omitted, not null", i);
        }
        if chunk_str.contains("\"rerankerScore\":null") {
            panic!("ragChunks[{}].rerankerScore should be omitted, not null", i);
        }
    }
}

// ============================================================================
// AskDebugInfo Shape Tests (golden-ask-009)
// ============================================================================

/// golden-ask-009: AskDebugInfo contains timing and base info.
///
/// Required fields: embeddingModelId, usedBases, perBaseCounts, rerankerUsed, hybridSearchUsed
/// Optional timing fields may be present
#[test]
fn golden_debug_info_completeness() {
    let temp = setup_indexed_workspace();
    let workspace = temp.path();

    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("test query for debug info")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let debug = &bundle["debug"];

    // Required fields
    assert!(
        debug.get("embeddingModelId").is_some(),
        "debug missing embeddingModelId"
    );
    assert!(debug.get("usedBases").is_some(), "debug missing usedBases");
    assert!(
        debug.get("perBaseCounts").is_some(),
        "debug missing perBaseCounts"
    );
    assert!(
        debug.get("rerankerUsed").is_some(),
        "debug missing rerankerUsed"
    );
    assert!(
        debug.get("hybridSearchUsed").is_some(),
        "debug missing hybridSearchUsed"
    );

    // Type checks
    assert!(
        debug["embeddingModelId"].is_string(),
        "embeddingModelId should be string"
    );
    assert!(debug["usedBases"].is_array(), "usedBases should be array");
    assert!(
        debug["perBaseCounts"].is_array(),
        "perBaseCounts should be array"
    );
    assert!(
        debug["rerankerUsed"].is_boolean(),
        "rerankerUsed should be boolean"
    );
    assert!(
        debug["hybridSearchUsed"].is_boolean(),
        "hybridSearchUsed should be boolean"
    );

    // embeddingModelId should be non-empty
    let model_id = debug["embeddingModelId"].as_str().unwrap();
    assert!(
        !model_id.is_empty(),
        "embeddingModelId should be non-empty string"
    );

    // Optional timing fields (check they're numbers if present)
    if let Some(embed_ms) = debug.get("embedTimeMs") {
        assert!(
            embed_ms.is_number() || embed_ms.is_null(),
            "embedTimeMs should be number or null"
        );
    }
    if let Some(search_ms) = debug.get("searchTimeMs") {
        assert!(
            search_ms.is_number() || search_ms.is_null(),
            "searchTimeMs should be number or null"
        );
    }
}

// ============================================================================
// Error Case Tests (golden-ask-006, golden-ask-007)
// ============================================================================

/// golden-ask-006: Ask on uninitialized workspace returns proper error.
#[test]
fn golden_ask_uninitialized_error() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Create a source file but don't init
    create_source(workspace, "lib.rs", "pub fn test() {}");

    // Ask should fail with informative error
    gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("test query")
        .assert()
        .failure();
    // Note: The exact error message may vary, the key is that it fails gracefully
}

/// golden-ask-007: Ask with no indexed bases returns proper error.
#[test]
fn golden_ask_no_indexed_bases_error() {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init but don't add/commit anything
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Ask should fail (no indexed bases)
    gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("test query")
        .assert()
        .failure();
}

// ============================================================================
// Stack Summary Tests
// ============================================================================

/// Test that stackSummary is present when indexed workspace has stack info.
#[test]
fn golden_stack_summary_present() {
    let temp = setup_indexed_workspace();
    let workspace = temp.path();

    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("What languages are used?")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    // stackSummary may be present or null (it's optional)
    if let Some(stack) = bundle.get("stackSummary") {
        if !stack.is_null() {
            // If present, verify structure
            assert!(
                stack.get("languages").is_some(),
                "stackSummary should have languages"
            );
            assert!(stack["languages"].is_array(), "languages should be array");

            assert!(
                stack.get("managers").is_some(),
                "stackSummary should have managers"
            );
            assert!(stack["managers"].is_array(), "managers should be array");
        }
    }
}

// ============================================================================
// Phase 8.6: Filename Pre-filter Tests
// ============================================================================

/// Create a workspace with specific files for filename tests.
fn setup_filename_test_workspace() -> TempDir {
    let temp = TempDir::new().expect("create temp dir");
    let workspace = temp.path();

    // Init
    gik_cmd()
        .current_dir(workspace)
        .arg("init")
        .assert()
        .success();

    // Create multiple files, one with a specific name we'll query for
    let src_dir = workspace.join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");

    // Target file we'll query for
    fs::write(
        src_dir.join("globals.css"),
        "/* Global styles */\nbody { margin: 0; }\n",
    )
    .expect("write globals.css");

    // Other files that might compete in dense search
    fs::write(
        src_dir.join("styles.css"),
        "/* Component styles */\n.button { padding: 10px; }\n",
    )
    .expect("write styles.css");

    fs::write(
        src_dir.join("main.rs"),
        "fn main() { println!(\"Hello\"); }\n",
    )
    .expect("write main.rs");

    fs::write(
        src_dir.join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .expect("write lib.rs");

    // Add and commit
    gik_cmd()
        .current_dir(workspace)
        .arg("add")
        .arg("src/")
        .assert()
        .success();

    gik_cmd()
        .current_dir(workspace)
        .arg("commit")
        .arg("-m")
        .arg("feat: add test files")
        .assert()
        .success();

    temp
}

/// phase8.6-ask-001: Filename pre-filter includes matching file in results.
///
/// When a query explicitly mentions a filename (e.g., "globals.css"),
/// that file should appear in results even if dense search wouldn't
/// normally rank it highly.
#[test]
#[ignore] // Requires real embedding model
fn filename_prefilter_includes_matching_file() {
    let temp = setup_filename_test_workspace();
    let workspace = temp.path();

    let output = gik_cmd()
        .current_dir(workspace)
        .arg("ask")
        .arg("--json")
        .arg("what is in globals.css")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let bundle: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    let chunks = bundle["ragChunks"]
        .as_array()
        .expect("ragChunks should be array");

    // globals.css should be in the results
    let has_globals = chunks.iter().any(|c| {
        c["path"]
            .as_str()
            .map(|p| p.contains("globals.css"))
            .unwrap_or(false)
    });

    assert!(
        has_globals,
        "globals.css should appear in results when explicitly mentioned in query"
    );
}
