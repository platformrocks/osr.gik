//! Integration tests for KG extraction from existing bases.
//!
//! These tests validate the end-to-end KG extraction workflow:
//! - KG nodes are created from code/docs sources
//! - KG import edges are created from detected imports
//! - KG is synchronized during commit flow
//! - stats.json is consistent with nodes/edges
//!
//! # Phase 9.2 Scope
//!
//! This phase implements file-level KG extraction:
//! - Nodes of kind "file" for code sources
//! - Nodes of kind "doc" for docs sources
//! - Edges of kind "imports" for file→file import relationships
//!
//! Phase 9.2.1 added symbol extraction but these tests focus on file-level
//! extraction only (using config.without_symbols()).

use std::fs;
use tempfile::TempDir;

use gik_core::kg::{
    kg_exists, read_all_edges, read_all_nodes, read_stats, sync_branch_kg, DefaultKgExtractor,
    KgExtractionConfig, KgExtractor, KG_VERSION,
};
use gik_core::workspace::Workspace;

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a temporary workspace with initialized GIK structure.
fn create_test_workspace() -> (TempDir, Workspace) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Create the .guided/knowledge directory structure (simulating init_workspace)
    let knowledge_root = temp_dir.path().join(".guided/knowledge");
    fs::create_dir_all(&knowledge_root).expect("Failed to create knowledge root");

    // Create branch directory
    fs::create_dir_all(knowledge_root.join("main")).expect("Failed to create main branch");

    let workspace = Workspace::from_root(temp_dir.path()).expect("Failed to create workspace");

    (temp_dir, workspace)
}

/// Create a workspace with a code base containing source files.
fn create_workspace_with_code_base() -> (TempDir, Workspace) {
    let (temp_dir, workspace) = create_test_workspace();

    // Create code base directory
    let code_base_dir = workspace.knowledge_root().join("main/bases/code");
    fs::create_dir_all(&code_base_dir).expect("Failed to create code base");

    // Create sources.jsonl with TypeScript files that have imports
    let sources_content = r#"{"id":"chunk-001","base":"code","branch":"main","filePath":"src/index.ts","startLine":1,"endLine":20,"text":"import { helper, format } from './utils';\nimport { Config } from './config';\n\nexport function main() {\n  const cfg = new Config();\n  console.log(helper(cfg.value));\n}","vectorId":1,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-001"}
{"id":"chunk-002","base":"code","branch":"main","filePath":"src/utils.ts","startLine":1,"endLine":15,"text":"export function helper(x: number): number {\n  return x * 2;\n}\n\nexport function format(s: string): string {\n  return s.trim();\n}","vectorId":2,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-002"}
{"id":"chunk-003","base":"code","branch":"main","filePath":"src/config.ts","startLine":1,"endLine":10,"text":"export class Config {\n  value: number = 42;\n}","vectorId":3,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-003"}
{"id":"chunk-004","base":"code","branch":"main","filePath":"src/api/handler.ts","startLine":1,"endLine":15,"text":"import { helper } from '../utils';\n\nexport function handleRequest(req: Request) {\n  return helper(req.body.value);\n}","vectorId":4,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-004"}"#;

    fs::write(code_base_dir.join("sources.jsonl"), sources_content)
        .expect("Failed to write sources");

    (temp_dir, workspace)
}

/// Create a workspace with both code and docs bases.
fn create_workspace_with_code_and_docs() -> (TempDir, Workspace) {
    let (temp_dir, workspace) = create_workspace_with_code_base();

    // Create docs base directory
    let docs_base_dir = workspace.knowledge_root().join("main/bases/docs");
    fs::create_dir_all(&docs_base_dir).expect("Failed to create docs base");

    // Create sources.jsonl with markdown documentation
    let line1 = r#"{"id":"chunk-d01","base":"docs","branch":"main","filePath":"docs/README.md","startLine":1,"endLine":30,"text":"Project Documentation","vectorId":101,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-d01"}"#;
    let line2 = r#"{"id":"chunk-d02","base":"docs","branch":"main","filePath":"docs/api.md","startLine":1,"endLine":20,"text":"API Reference","vectorId":102,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-d02"}"#;
    let docs_content = format!("{}\n{}", line1, line2);

    fs::write(docs_base_dir.join("sources.jsonl"), docs_content)
        .expect("Failed to write docs sources");

    (temp_dir, workspace)
}

/// Helper to sync KG without symbol extraction (for these legacy tests).
fn sync_without_symbols(workspace: &Workspace, branch: &str) -> gik_core::kg::KgSyncResult {
    let cfg = KgExtractionConfig::default()
        .without_symbols()
        .without_endpoints();
    let extractor = DefaultKgExtractor::new();
    sync_branch_kg(workspace, branch, &extractor, &cfg).unwrap()
}

// ============================================================================
// Basic Extraction Tests
// ============================================================================

#[test]
fn test_kg_extraction_creates_file_nodes() {
    let (_temp_dir, workspace) = create_workspace_with_code_base();

    let result = sync_without_symbols(&workspace, "main");

    // Should have created 4 file nodes
    assert_eq!(result.nodes_written, 4);
    assert!(kg_exists(&workspace, "main"));

    let nodes = read_all_nodes(&workspace, "main").unwrap();
    assert_eq!(nodes.len(), 4);

    // Verify all nodes are of kind "file"
    for node in &nodes {
        assert_eq!(node.kind, "file");
        assert!(node.id.starts_with("file:"));
    }

    // Verify expected file paths
    let paths: Vec<_> = nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(paths.contains(&"file:src/index.ts"));
    assert!(paths.contains(&"file:src/utils.ts"));
    assert!(paths.contains(&"file:src/config.ts"));
    assert!(paths.contains(&"file:src/api/handler.ts"));
}

#[test]
fn test_kg_extraction_creates_import_edges() {
    let (_temp_dir, workspace) = create_workspace_with_code_base();

    let result = sync_without_symbols(&workspace, "main");

    // Should have import edges
    assert!(result.edges_written > 0);

    let edges = read_all_edges(&workspace, "main").unwrap();

    // All edges should be of kind "imports"
    for edge in &edges {
        assert_eq!(edge.kind, "imports");
    }

    // index.ts should import utils.ts and config.ts
    let index_imports: Vec<_> = edges
        .iter()
        .filter(|e| e.from == "file:src/index.ts")
        .collect();

    // Should have at least the utils import
    let imports_utils = index_imports.iter().any(|e| e.to == "file:src/utils.ts");
    assert!(imports_utils, "index.ts should import utils.ts");

    let imports_config = index_imports.iter().any(|e| e.to == "file:src/config.ts");
    assert!(imports_config, "index.ts should import config.ts");
}

#[test]
fn test_kg_extraction_creates_doc_nodes() {
    let (_temp_dir, workspace) = create_workspace_with_code_and_docs();

    let result = sync_without_symbols(&workspace, "main");

    // Should have file nodes (4) + doc nodes (2) = 6
    assert_eq!(result.nodes_written, 6);

    let nodes = read_all_nodes(&workspace, "main").unwrap();

    // Filter doc nodes
    let doc_nodes: Vec<_> = nodes.iter().filter(|n| n.kind == "doc").collect();
    assert_eq!(doc_nodes.len(), 2);

    // Verify doc node IDs
    let doc_ids: Vec<_> = doc_nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(doc_ids.contains(&"doc:docs/README.md"));
    assert!(doc_ids.contains(&"doc:docs/api.md"));
}

#[test]
fn test_kg_extraction_stats_consistent() {
    let (_temp_dir, workspace) = create_workspace_with_code_and_docs();

    sync_without_symbols(&workspace, "main");

    let stats = read_stats(&workspace, "main").unwrap();
    let nodes = read_all_nodes(&workspace, "main").unwrap();
    let edges = read_all_edges(&workspace, "main").unwrap();

    // Stats should match actual counts
    assert_eq!(stats.node_count, nodes.len() as u64);
    assert_eq!(stats.edge_count, edges.len() as u64);
    assert_eq!(stats.version, KG_VERSION);
}

// ============================================================================
// Node Props Tests
// ============================================================================

#[test]
fn test_kg_node_props_contain_base_and_path() {
    let (_temp_dir, workspace) = create_workspace_with_code_and_docs();

    sync_without_symbols(&workspace, "main");

    let nodes = read_all_nodes(&workspace, "main").unwrap();

    for node in &nodes {
        // All nodes should have props.base and props.path
        assert!(
            node.props.get("base").is_some(),
            "Node {} missing base prop",
            node.id
        );
        assert!(
            node.props.get("path").is_some(),
            "Node {} missing path prop",
            node.id
        );

        // Code files should have base = "code", docs should have base = "docs"
        let base = node.props.get("base").unwrap().as_str().unwrap();
        if node.kind == "file" {
            assert_eq!(base, "code");
        } else if node.kind == "doc" {
            assert_eq!(base, "docs");
        }
    }
}

#[test]
fn test_kg_edge_props_contain_raw_import() {
    let (_temp_dir, workspace) = create_workspace_with_code_base();

    sync_without_symbols(&workspace, "main");

    let edges = read_all_edges(&workspace, "main").unwrap();

    for edge in &edges {
        // All import edges should have props.rawImport
        assert!(
            edge.props.get("rawImport").is_some(),
            "Edge {} -> {} missing rawImport prop",
            edge.from,
            edge.to
        );
    }
}

// ============================================================================
// Full Rebuild Tests
// ============================================================================

#[test]
fn test_kg_sync_is_full_rebuild() {
    let (_temp_dir, workspace) = create_workspace_with_code_base();

    // First sync
    let result1 = sync_without_symbols(&workspace, "main");
    assert!(result1.full_rebuild);
    assert_eq!(result1.nodes_written, 4);

    // Second sync should still be full rebuild
    let result2 = sync_without_symbols(&workspace, "main");
    assert!(result2.full_rebuild);
    assert_eq!(result2.nodes_written, 4);

    // Verify we have exactly 4 nodes (not 8)
    let nodes = read_all_nodes(&workspace, "main").unwrap();
    assert_eq!(nodes.len(), 4);
}

// ============================================================================
// Empty Workspace Tests
// ============================================================================

#[test]
fn test_kg_extraction_empty_workspace() {
    let (_temp_dir, workspace) = create_test_workspace();

    let result = sync_without_symbols(&workspace, "main");

    // No bases = no nodes
    assert_eq!(result.nodes_written, 0);
    assert_eq!(result.edges_written, 0);

    // KG directory should not be created for empty extraction
    assert!(!kg_exists(&workspace, "main"));
}

// ============================================================================
// Extractor Config Tests
// ============================================================================

#[test]
fn test_kg_extraction_with_max_files_limit() {
    let (_temp_dir, workspace) = create_workspace_with_code_base();

    let extractor = DefaultKgExtractor::new();
    // Disable symbols/endpoints so we only count file nodes
    let cfg = KgExtractionConfig::new()
        .with_max_files(2)
        .without_symbols()
        .without_endpoints();

    let result = extractor
        .extract_for_branch(&workspace, "main", &cfg)
        .unwrap();

    // Should only have 2 file nodes due to limit
    assert_eq!(result.nodes.len(), 2);
}

#[test]
fn test_kg_extraction_without_docs() {
    let (_temp_dir, workspace) = create_workspace_with_code_and_docs();

    let extractor = DefaultKgExtractor::new();
    // Disable symbols/endpoints to only get file/doc nodes
    let cfg = KgExtractionConfig::new()
        .without_docs()
        .without_symbols()
        .without_endpoints();

    let result = extractor
        .extract_for_branch(&workspace, "main", &cfg)
        .unwrap();

    // Should only have file nodes, no doc nodes
    for node in &result.nodes {
        assert_eq!(node.kind, "file", "Should not have doc nodes");
    }
}

// ============================================================================
// Branch Isolation Tests
// ============================================================================

#[test]
fn test_kg_extraction_branch_isolation() {
    let (_temp_dir, workspace) = create_workspace_with_code_base();

    // Create another branch
    let knowledge_root = workspace.knowledge_root();
    fs::create_dir_all(knowledge_root.join("feature-x")).expect("Failed to create feature branch");

    // Create code base for feature branch with different files
    let feature_code_dir = knowledge_root.join("feature-x/bases/code");
    fs::create_dir_all(&feature_code_dir).expect("Failed to create feature code base");

    let feature_sources = r#"{"id":"chunk-f01","base":"code","branch":"feature-x","filePath":"src/new-feature.ts","startLine":1,"endLine":5,"text":"export function newFeature() { return 'new'; }","vectorId":201,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-f01","sourceId":"src-f01"}"#;

    fs::write(feature_code_dir.join("sources.jsonl"), feature_sources)
        .expect("Failed to write feature sources");

    // Sync both branches
    sync_without_symbols(&workspace, "main");
    sync_without_symbols(&workspace, "feature-x");

    // Main should have 4 nodes
    let main_nodes = read_all_nodes(&workspace, "main").unwrap();
    assert_eq!(main_nodes.len(), 4);

    // Feature should have 1 node
    let feature_nodes = read_all_nodes(&workspace, "feature-x").unwrap();
    assert_eq!(feature_nodes.len(), 1);
    assert_eq!(feature_nodes[0].id, "file:src/new-feature.ts");
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_kg_extraction_handles_external_imports() {
    let (_temp_dir, workspace) = create_test_workspace();

    // Create code base with external imports
    let code_base_dir = workspace.knowledge_root().join("main/bases/code");
    fs::create_dir_all(&code_base_dir).expect("Failed to create code base");

    // File with both internal and external imports
    let sources = r#"{"id":"chunk-001","base":"code","branch":"main","filePath":"src/app.ts","startLine":1,"endLine":10,"text":"import React from 'react';\nimport express from 'express';\nimport { helper } from './utils';\n\nconsole.log('app');","vectorId":1,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-001"}
{"id":"chunk-002","base":"code","branch":"main","filePath":"src/utils.ts","startLine":1,"endLine":5,"text":"export function helper() { return 42; }","vectorId":2,"indexedAt":"2025-01-01T00:00:00Z","revisionId":"rev-001","sourceId":"src-002"}"#;

    fs::write(code_base_dir.join("sources.jsonl"), sources).expect("Failed to write sources");

    sync_without_symbols(&workspace, "main");

    let edges = read_all_edges(&workspace, "main").unwrap();

    // Should only have edge for internal import (./utils), not external (react, express)
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].from, "file:src/app.ts");
    assert_eq!(edges[0].to, "file:src/utils.ts");
}

#[test]
fn test_kg_extraction_handles_nested_imports() {
    let (_temp_dir, workspace) = create_workspace_with_code_base();

    sync_without_symbols(&workspace, "main");

    let edges = read_all_edges(&workspace, "main").unwrap();

    // handler.ts imports ../utils which should resolve to src/utils.ts
    let handler_imports: Vec<_> = edges
        .iter()
        .filter(|e| e.from == "file:src/api/handler.ts")
        .collect();

    assert!(
        !handler_imports.is_empty(),
        "handler.ts should have imports"
    );
    assert!(
        handler_imports.iter().any(|e| e.to == "file:src/utils.ts"),
        "handler.ts should import utils.ts via ../utils"
    );
}
