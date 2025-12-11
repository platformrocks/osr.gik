//! Release module for GIK.
//!
//! This module provides functionality for generating release changelogs from
//! the timeline. It reads Commit revisions, parses conventional commit messages,
//! groups them by type, and renders a `CHANGELOG.md` file.
//!
//! **Key design decisions:**
//! - Release is **read-only**: it does NOT append a `RevisionOperation::Release` to the timeline.
//! - CHANGELOG.md is **fully regenerated** each time (no incremental merging).
//! - Commit messages follow **Conventional Commits** format for grouping.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::errors::GikError;
use crate::timeline::{read_timeline, Revision, RevisionId, RevisionOperation};
use crate::workspace::{BranchName, Workspace};

// ============================================================================
// Release Types
// ============================================================================

/// Specifies the revision range for gathering release entries.
#[derive(Debug, Clone, Default)]
pub struct ReleaseRange {
    /// Starting revision (exclusive). If None, starts from the beginning.
    pub from: Option<RevisionId>,
    /// Ending revision (inclusive). If None, ends at HEAD.
    pub to: Option<RevisionId>,
}

/// Mode for writing the changelog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReleaseMode {
    /// Replace the entire CHANGELOG.md (default).
    /// Regenerates from the GIK timeline each time.
    #[default]
    Replace,
    /// Append a new version section to existing CHANGELOG.md.
    /// Preserves the header and existing version sections.
    Append,
}

/// The kind of a release entry, parsed from Conventional Commits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseEntryKind {
    /// New feature (feat:)
    Feat,
    /// Bug fix (fix:)
    Fix,
    /// Documentation changes (docs:)
    Docs,
    /// Code style changes (style:)
    Style,
    /// Code refactoring (refactor:)
    Refactor,
    /// Performance improvements (perf:)
    Perf,
    /// Tests (test:)
    Test,
    /// Build system changes (build:)
    Build,
    /// CI configuration (ci:)
    Ci,
    /// Chores (chore:)
    Chore,
    /// Reverts (revert:)
    Revert,
    /// Other/unknown type
    Other,
}

impl ReleaseEntryKind {
    /// Parse the kind from a conventional commit prefix.
    pub fn from_prefix(prefix: &str) -> Self {
        match prefix.to_lowercase().as_str() {
            "feat" | "feature" => Self::Feat,
            "fix" | "bugfix" => Self::Fix,
            "docs" | "doc" => Self::Docs,
            "style" => Self::Style,
            "refactor" => Self::Refactor,
            "perf" | "performance" => Self::Perf,
            "test" | "tests" => Self::Test,
            "build" => Self::Build,
            "ci" => Self::Ci,
            "chore" => Self::Chore,
            "revert" => Self::Revert,
            _ => Self::Other,
        }
    }

    /// Get the display label for this kind.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Feat => "Features",
            Self::Fix => "Bug Fixes",
            Self::Docs => "Documentation",
            Self::Style => "Styles",
            Self::Refactor => "Code Refactoring",
            Self::Perf => "Performance Improvements",
            Self::Test => "Tests",
            Self::Build => "Build System",
            Self::Ci => "Continuous Integration",
            Self::Chore => "Chores",
            Self::Revert => "Reverts",
            Self::Other => "Other Changes",
        }
    }

    /// Get the sort order for grouping (lower = earlier in changelog).
    pub fn sort_order(&self) -> u8 {
        match self {
            Self::Feat => 0,
            Self::Fix => 1,
            Self::Perf => 2,
            Self::Refactor => 3,
            Self::Docs => 4,
            Self::Style => 5,
            Self::Test => 6,
            Self::Build => 7,
            Self::Ci => 8,
            Self::Chore => 9,
            Self::Revert => 10,
            Self::Other => 11,
        }
    }
}

/// A single entry in the release changelog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseEntry {
    /// The kind of change (feat, fix, etc.).
    pub kind: ReleaseEntryKind,
    /// The scope (e.g., "cli", "core") if present.
    pub scope: Option<String>,
    /// The description text.
    pub description: String,
    /// Whether this is a breaking change.
    pub breaking: bool,
    /// The revision ID this entry came from.
    pub revision_id: RevisionId,
    /// The timestamp of the revision.
    pub timestamp: DateTime<Utc>,
    /// Number of sources indexed in this commit.
    pub source_count: usize,
    /// Bases touched by this commit.
    pub bases: Vec<String>,
}

/// A memory operation entry for the release changelog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReleaseEntry {
    /// The type of memory operation.
    pub operation: MemoryOperationType,
    /// The description/message for this operation.
    pub description: String,
    /// The revision ID this entry came from.
    pub revision_id: RevisionId,
    /// The timestamp of the revision.
    pub timestamp: DateTime<Utc>,
    /// Number of entries affected (ingested or pruned).
    pub count: usize,
    /// For pruning: number archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_count: Option<usize>,
    /// For pruning: number deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_count: Option<usize>,
}

/// Type of memory operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryOperationType {
    /// Memory entries were ingested.
    Ingest,
    /// Memory entries were pruned.
    Prune,
}

impl std::fmt::Display for MemoryOperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ingest => write!(f, "ingest"),
            Self::Prune => write!(f, "prune"),
        }
    }
}

/// Summary of a release, containing grouped entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseSummary {
    /// The branch this release covers.
    pub branch: String,
    /// The starting revision (exclusive), or None if from beginning.
    pub from_revision: Option<RevisionId>,
    /// The ending revision (inclusive), or None if to HEAD.
    pub to_revision: Option<RevisionId>,
    /// Total number of entries.
    pub total_entries: usize,
    /// Entries grouped by kind.
    pub groups: Vec<ReleaseGroup>,
    /// Memory operations (ingests and prunes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_entries: Vec<MemoryReleaseEntry>,
    /// Whether this was a dry run.
    pub dry_run: bool,
}

/// A group of entries of the same kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroup {
    /// The kind of entries in this group.
    pub kind: ReleaseEntryKind,
    /// The display label for this group.
    pub label: String,
    /// Entries in this group.
    pub entries: Vec<ReleaseEntry>,
}

/// Options for the release command.
#[derive(Debug, Clone, Default)]
pub struct ReleaseOptions {
    /// Optional release tag (e.g., "v1.0.0"). Used as heading in CHANGELOG.
    pub tag: Option<String>,
    /// Branch to generate release for (defaults to current branch).
    pub branch: Option<String>,
    /// Revision range to include.
    pub range: ReleaseRange,
    /// Dry run: report what would be written without actually writing.
    pub dry_run: bool,
    /// Mode for writing changelog (replace or append).
    pub mode: ReleaseMode,
}

/// Result of the release command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseResult {
    /// The tag that was used for the release heading.
    pub tag: String,
    /// Path to the generated CHANGELOG.md (if not dry run).
    pub changelog_path: Option<String>,
    /// The release summary.
    pub summary: ReleaseSummary,
}

// ============================================================================
// Conventional Commit Parsing
// ============================================================================

/// Parsed components of a conventional commit message.
#[derive(Debug, Clone)]
struct ConventionalCommit {
    kind: ReleaseEntryKind,
    scope: Option<String>,
    description: String,
    breaking: bool,
}

/// Parse a commit message following Conventional Commits format.
///
/// Format: `<type>[optional scope][!]: <description>`
///
/// Examples:
/// - `feat: add new feature`
/// - `fix(cli): resolve parsing error`
/// - `feat!: breaking change`
/// - `feat(core)!: breaking change with scope`
fn parse_conventional_commit(message: &str) -> ConventionalCommit {
    // Default result for non-conventional messages
    let default = ConventionalCommit {
        kind: ReleaseEntryKind::Other,
        scope: None,
        description: message.to_string(),
        breaking: false,
    };

    // Try to match conventional commit pattern
    // Pattern: type(scope)?: description  OR  type(scope)?!: description
    let message = message.trim();

    // Find the colon separator
    let colon_pos = match message.find(':') {
        Some(pos) => pos,
        None => return default,
    };

    let prefix = &message[..colon_pos];
    let description = message[colon_pos + 1..].trim();

    if description.is_empty() {
        return default;
    }

    // Check for breaking change marker
    let (prefix, breaking) = if let Some(stripped) = prefix.strip_suffix('!') {
        (stripped, true)
    } else {
        (prefix, false)
    };

    // Check for scope in parentheses
    let (type_part, scope) = if let Some(paren_start) = prefix.find('(') {
        if let Some(paren_end) = prefix.find(')') {
            let type_part = &prefix[..paren_start];
            let scope = &prefix[paren_start + 1..paren_end];
            (type_part, Some(scope.to_string()))
        } else {
            (prefix, None)
        }
    } else {
        (prefix, None)
    };

    let kind = ReleaseEntryKind::from_prefix(type_part);

    ConventionalCommit {
        kind,
        scope,
        description: description.to_string(),
        breaking,
    }
}

// ============================================================================
// Release Functions
// ============================================================================

/// Gather release entries from the timeline within the specified range.
///
/// This reads all Commit revisions in the range and creates ReleaseEntry
/// objects from them.
pub fn gather_release_entries(
    workspace: &Workspace,
    branch: &BranchName,
    range: &ReleaseRange,
) -> Result<Vec<ReleaseEntry>, GikError> {
    let timeline_path = workspace.timeline_path(branch.as_str());
    let revisions = read_timeline(&timeline_path)?;

    // Build revision lookup map
    let revision_map: HashMap<&RevisionId, &Revision> =
        revisions.iter().map(|r| (&r.id, r)).collect();

    // Determine the range bounds
    let from_id = range.from.as_ref();
    let to_id = range.to.as_ref();

    // Walk from HEAD backwards, collecting commits in range
    let mut entries = Vec::new();
    let mut include = to_id.is_none(); // If no to_id, start including immediately

    for revision in revisions.iter().rev() {
        // Check if we've reached the 'to' boundary
        if let Some(to) = to_id {
            if &revision.id == to {
                include = true;
            }
        }

        if !include {
            continue;
        }

        // Check if we've passed the 'from' boundary
        if let Some(from) = from_id {
            if &revision.id == from {
                break; // Stop before including this revision
            }
        }

        // Extract commit operations
        for op in &revision.operations {
            if let RevisionOperation::Commit {
                bases,
                source_count,
            } = op
            {
                let parsed = parse_conventional_commit(&revision.message);

                entries.push(ReleaseEntry {
                    kind: parsed.kind,
                    scope: parsed.scope,
                    description: parsed.description,
                    breaking: parsed.breaking,
                    revision_id: revision.id.clone(),
                    timestamp: revision.timestamp,
                    source_count: *source_count,
                    bases: bases.clone(),
                });
            }
        }
    }

    // Reverse to get chronological order (oldest first)
    entries.reverse();

    // Drop revision_map to satisfy borrow checker
    drop(revision_map);

    Ok(entries)
}

/// Gather memory operation entries from the timeline within the specified range.
///
/// This reads all MemoryIngest and MemoryPrune revisions in the range and
/// creates MemoryReleaseEntry objects from them.
pub fn gather_memory_entries(
    workspace: &Workspace,
    branch: &BranchName,
    range: &ReleaseRange,
) -> Result<Vec<MemoryReleaseEntry>, GikError> {
    let timeline_path = workspace.timeline_path(branch.as_str());
    let revisions = read_timeline(&timeline_path)?;

    // Determine the range bounds
    let from_id = range.from.as_ref();
    let to_id = range.to.as_ref();

    // Walk from HEAD backwards, collecting memory operations in range
    let mut entries = Vec::new();
    let mut include = to_id.is_none(); // If no to_id, start including immediately

    for revision in revisions.iter().rev() {
        // Check if we've reached the 'to' boundary
        if let Some(to) = to_id {
            if &revision.id == to {
                include = true;
            }
        }

        if !include {
            continue;
        }

        // Check if we've passed the 'from' boundary
        if let Some(from) = from_id {
            if &revision.id == from {
                break; // Stop before including this revision
            }
        }

        // Extract memory operations
        for op in &revision.operations {
            match op {
                RevisionOperation::MemoryIngest { count } => {
                    entries.push(MemoryReleaseEntry {
                        operation: MemoryOperationType::Ingest,
                        description: revision.message.clone(),
                        revision_id: revision.id.clone(),
                        timestamp: revision.timestamp,
                        count: *count,
                        archived_count: None,
                        deleted_count: None,
                    });
                }
                RevisionOperation::MemoryPrune {
                    count,
                    archived_count,
                    deleted_count,
                } => {
                    entries.push(MemoryReleaseEntry {
                        operation: MemoryOperationType::Prune,
                        description: revision.message.clone(),
                        revision_id: revision.id.clone(),
                        timestamp: revision.timestamp,
                        count: *count,
                        archived_count: Some(*archived_count),
                        deleted_count: Some(*deleted_count),
                    });
                }
                _ => {}
            }
        }
    }

    // Reverse to get chronological order (oldest first)
    entries.reverse();

    Ok(entries)
}

/// Group release entries by kind for rendering.
pub fn group_entries_by_kind(entries: Vec<ReleaseEntry>) -> Vec<ReleaseGroup> {
    let mut groups: HashMap<ReleaseEntryKind, Vec<ReleaseEntry>> = HashMap::new();

    for entry in entries {
        groups.entry(entry.kind.clone()).or_default().push(entry);
    }

    // Convert to sorted groups
    let mut result: Vec<ReleaseGroup> = groups
        .into_iter()
        .map(|(kind, entries)| ReleaseGroup {
            label: kind.label().to_string(),
            kind,
            entries,
        })
        .collect();

    // Sort groups by kind's sort order
    result.sort_by_key(|g| g.kind.sort_order());

    result
}

/// Render a CHANGELOG.md from the release summary.
pub fn render_changelog_markdown(summary: &ReleaseSummary, tag: &str) -> String {
    let mut out = String::new();

    // Header
    out.push_str("# Changelog\n\n");
    out.push_str("All notable changes to this project will be documented in this file.\n\n");
    out.push_str(
        "This changelog is auto-generated by GIK from commit history.\n\n",
    );

    // Release heading
    let heading = if tag.is_empty() {
        "Unreleased".to_string()
    } else {
        tag.to_string()
    };
    out.push_str(&format!("## {}\n\n", heading));

    if summary.groups.is_empty() && summary.memory_entries.is_empty() {
        out.push_str("No changes recorded.\n\n");
        return out;
    }

    // Render each group
    for group in &summary.groups {
        out.push_str(&format!("### {}\n\n", group.label));

        for entry in &group.entries {
            let scope_part = entry
                .scope
                .as_ref()
                .map(|s| format!("**{}:** ", s))
                .unwrap_or_default();
            let breaking_part = if entry.breaking { "**BREAKING:** " } else { "" };
            let rev_short = &entry.revision_id.as_str()[..8.min(entry.revision_id.as_str().len())];

            out.push_str(&format!(
                "- {}{}{} ({})\n",
                breaking_part, scope_part, entry.description, rev_short
            ));
        }

        out.push('\n');
    }

    // Render memory section if there are any memory operations
    if !summary.memory_entries.is_empty() {
        out.push_str("### Memory\n\n");

        for entry in &summary.memory_entries {
            let rev_short = &entry.revision_id.as_str()[..8.min(entry.revision_id.as_str().len())];

            match entry.operation {
                MemoryOperationType::Ingest => {
                    out.push_str(&format!(
                        "- **ingest:** {} ({} entries) ({})\n",
                        entry.description, entry.count, rev_short
                    ));
                }
                MemoryOperationType::Prune => {
                    let archived = entry.archived_count.unwrap_or(0);
                    let deleted = entry.deleted_count.unwrap_or(0);
                    out.push_str(&format!(
                        "- **prune:** {} ({} entries: {} archived, {} deleted) ({})\n",
                        entry.description, entry.count, archived, deleted, rev_short
                    ));
                }
            }
        }

        out.push('\n');
    }

    out
}

/// Run the release pipeline.
///
/// This is the main entry point for generating a release changelog:
/// 1. Gather entries from the timeline in the specified range
/// 2. Group entries by conventional commit type
/// 3. Gather memory operation entries
/// 4. Render and write CHANGELOG.md (unless dry_run)
///
/// **Note:** This does NOT mutate the timeline. No `RevisionOperation::Release`
/// is appended.
pub fn run_release(
    workspace: &Workspace,
    branch: &BranchName,
    opts: &ReleaseOptions,
) -> Result<ReleaseResult, GikError> {
    tracing::info!(
        branch = %branch,
        tag = ?opts.tag,
        dry_run = opts.dry_run,
        "Running release pipeline"
    );

    // Gather commit entries
    let entries = gather_release_entries(workspace, branch, &opts.range)?;
    tracing::debug!(count = entries.len(), "Gathered release entries");

    // Gather memory entries
    let memory_entries = gather_memory_entries(workspace, branch, &opts.range)?;
    tracing::debug!(count = memory_entries.len(), "Gathered memory entries");

    // Group by kind
    let groups = group_entries_by_kind(entries);
    let total_entries: usize = groups.iter().map(|g| g.entries.len()).sum();

    // Build summary
    let summary = ReleaseSummary {
        branch: branch.as_str().to_string(),
        from_revision: opts.range.from.clone(),
        to_revision: opts.range.to.clone(),
        total_entries,
        groups,
        memory_entries,
        dry_run: opts.dry_run,
    };

    // Determine tag
    let tag = opts.tag.clone().unwrap_or_else(|| "Unreleased".to_string());

    // Write to file (unless dry run)
    let changelog_path = if opts.dry_run {
        tracing::info!("Dry run - not writing CHANGELOG.md");
        None
    } else {
        let path = workspace.root().join("CHANGELOG.md");
        match opts.mode {
            ReleaseMode::Replace => {
                let markdown = render_changelog_markdown(&summary, &tag);
                write_changelog(&path, &markdown)?;
                tracing::info!(path = %path.display(), "Wrote CHANGELOG.md (replace mode)");
            }
            ReleaseMode::Append => {
                let section = render_version_section(&summary, &tag);
                append_changelog(&path, &section, &tag)?;
                tracing::info!(path = %path.display(), "Updated CHANGELOG.md (append mode)");
            }
        }
        Some(path.to_string_lossy().to_string())
    };

    Ok(ReleaseResult {
        tag,
        changelog_path,
        summary,
    })
}

/// Write the changelog content to a file (full replacement).
fn write_changelog(path: &Path, content: &str) -> Result<(), GikError> {
    fs::write(path, content).map_err(GikError::Io)
}

/// Render just the version section (without header) for append mode.
fn render_version_section(summary: &ReleaseSummary, tag: &str) -> String {
    let mut out = String::new();

    // Version heading
    let heading = if tag.is_empty() {
        "Unreleased".to_string()
    } else {
        tag.to_string()
    };
    out.push_str(&format!("## {}\n\n", heading));

    if summary.groups.is_empty() && summary.memory_entries.is_empty() {
        out.push_str("No changes recorded.\n\n");
        return out;
    }

    // Render each group
    for group in &summary.groups {
        out.push_str(&format!("### {}\n\n", group.label));

        for entry in &group.entries {
            let scope_part = entry
                .scope
                .as_ref()
                .map(|s| format!("**{}:** ", s))
                .unwrap_or_default();
            let breaking_part = if entry.breaking { "**BREAKING:** " } else { "" };
            let rev_short = &entry.revision_id.as_str()[..8.min(entry.revision_id.as_str().len())];

            out.push_str(&format!(
                "- {}{}{} ({})\n",
                breaking_part, scope_part, entry.description, rev_short
            ));
        }

        out.push('\n');
    }

    // Render memory section if there are any memory operations
    if !summary.memory_entries.is_empty() {
        out.push_str("### Memory\n\n");

        for entry in &summary.memory_entries {
            let rev_short = &entry.revision_id.as_str()[..8.min(entry.revision_id.as_str().len())];

            match entry.operation {
                MemoryOperationType::Ingest => {
                    out.push_str(&format!(
                        "- **ingest:** {} ({} entries) ({})\n",
                        entry.description, entry.count, rev_short
                    ));
                }
                MemoryOperationType::Prune => {
                    let archived = entry.archived_count.unwrap_or(0);
                    let deleted = entry.deleted_count.unwrap_or(0);
                    out.push_str(&format!(
                        "- **prune:** {} ({} entries: {} archived, {} deleted) ({})\n",
                        entry.description, entry.count, archived, deleted, rev_short
                    ));
                }
            }
        }

        out.push('\n');
    }

    out
}

/// Append a new version section to existing CHANGELOG.md.
///
/// This function:
/// 1. Reads existing CHANGELOG.md (or creates with header if missing)
/// 2. Checks for duplicate tag section
/// 3. Inserts new section after the header, before existing versions
fn append_changelog(path: &Path, new_section: &str, tag: &str) -> Result<(), GikError> {
    let section_marker = format!("## {}", tag);

    // Read existing content or create default header
    let existing = if path.exists() {
        fs::read_to_string(path).map_err(GikError::Io)?
    } else {
        // Create with standard header
        String::from(
            "# Changelog\n\n\
             All notable changes to this project will be documented in this file.\n\n\
             This changelog is auto-generated by GIK from commit history.\n\n",
        )
    };

    // Check for duplicate section
    if existing.contains(&section_marker) {
        return Err(GikError::InvalidArgument(format!(
            "Section '{}' already exists in CHANGELOG.md. Use a different --tag or remove the existing section.",
            section_marker
        )));
    }

    // Find insertion point: after header, before first ## section
    let insertion_point = find_first_version_section(&existing);

    // Build new content
    let new_content = if let Some(pos) = insertion_point {
        format!(
            "{}{}{}\n",
            &existing[..pos],
            new_section,
            &existing[pos..]
        )
    } else {
        // No existing sections, append at end
        format!("{}{}\n", existing, new_section)
    };

    fs::write(path, new_content).map_err(GikError::Io)
}

/// Find the byte position of the first `## ` section heading in content.
fn find_first_version_section(content: &str) -> Option<usize> {
    // Look for lines starting with "## " (version heading)
    for (i, line) in content.lines().enumerate() {
        if line.starts_with("## ") {
            // Calculate byte offset
            let offset: usize = content
                .lines()
                .take(i)
                .map(|l| l.len() + 1) // +1 for newline
                .sum();
            return Some(offset);
        }
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_conventional_commit_simple() {
        let parsed = parse_conventional_commit("feat: add new feature");
        assert_eq!(parsed.kind, ReleaseEntryKind::Feat);
        assert_eq!(parsed.scope, None);
        assert_eq!(parsed.description, "add new feature");
        assert!(!parsed.breaking);
    }

    #[test]
    fn test_parse_conventional_commit_with_scope() {
        let parsed = parse_conventional_commit("fix(cli): resolve parsing error");
        assert_eq!(parsed.kind, ReleaseEntryKind::Fix);
        assert_eq!(parsed.scope, Some("cli".to_string()));
        assert_eq!(parsed.description, "resolve parsing error");
        assert!(!parsed.breaking);
    }

    #[test]
    fn test_parse_conventional_commit_breaking() {
        let parsed = parse_conventional_commit("feat!: breaking change");
        assert_eq!(parsed.kind, ReleaseEntryKind::Feat);
        assert_eq!(parsed.scope, None);
        assert_eq!(parsed.description, "breaking change");
        assert!(parsed.breaking);
    }

    #[test]
    fn test_parse_conventional_commit_breaking_with_scope() {
        let parsed = parse_conventional_commit("feat(core)!: breaking change with scope");
        assert_eq!(parsed.kind, ReleaseEntryKind::Feat);
        assert_eq!(parsed.scope, Some("core".to_string()));
        assert_eq!(parsed.description, "breaking change with scope");
        assert!(parsed.breaking);
    }

    #[test]
    fn test_parse_conventional_commit_non_conventional() {
        let parsed = parse_conventional_commit("Random commit message");
        assert_eq!(parsed.kind, ReleaseEntryKind::Other);
        assert_eq!(parsed.scope, None);
        assert_eq!(parsed.description, "Random commit message");
        assert!(!parsed.breaking);
    }

    #[test]
    fn test_parse_conventional_commit_all_types() {
        let types = vec![
            ("feat:", ReleaseEntryKind::Feat),
            ("fix:", ReleaseEntryKind::Fix),
            ("docs:", ReleaseEntryKind::Docs),
            ("style:", ReleaseEntryKind::Style),
            ("refactor:", ReleaseEntryKind::Refactor),
            ("perf:", ReleaseEntryKind::Perf),
            ("test:", ReleaseEntryKind::Test),
            ("build:", ReleaseEntryKind::Build),
            ("ci:", ReleaseEntryKind::Ci),
            ("chore:", ReleaseEntryKind::Chore),
            ("revert:", ReleaseEntryKind::Revert),
        ];

        for (prefix, expected_kind) in types {
            let msg = format!("{} test description", prefix);
            let parsed = parse_conventional_commit(&msg);
            assert_eq!(parsed.kind, expected_kind, "Failed for prefix: {}", prefix);
        }
    }

    #[test]
    fn test_release_entry_kind_sort_order() {
        // Features should come first, Other should come last
        assert!(ReleaseEntryKind::Feat.sort_order() < ReleaseEntryKind::Fix.sort_order());
        assert!(ReleaseEntryKind::Fix.sort_order() < ReleaseEntryKind::Chore.sort_order());
        assert!(ReleaseEntryKind::Chore.sort_order() < ReleaseEntryKind::Other.sort_order());
    }

    #[test]
    fn test_group_entries_by_kind() {
        use chrono::Utc;

        let entries = vec![
            ReleaseEntry {
                kind: ReleaseEntryKind::Feat,
                scope: None,
                description: "feature 1".to_string(),
                breaking: false,
                revision_id: RevisionId::new("rev1"),
                timestamp: Utc::now(),
                source_count: 1,
                bases: vec!["code".to_string()],
            },
            ReleaseEntry {
                kind: ReleaseEntryKind::Fix,
                scope: None,
                description: "fix 1".to_string(),
                breaking: false,
                revision_id: RevisionId::new("rev2"),
                timestamp: Utc::now(),
                source_count: 1,
                bases: vec!["code".to_string()],
            },
            ReleaseEntry {
                kind: ReleaseEntryKind::Feat,
                scope: Some("cli".to_string()),
                description: "feature 2".to_string(),
                breaking: false,
                revision_id: RevisionId::new("rev3"),
                timestamp: Utc::now(),
                source_count: 2,
                bases: vec!["code".to_string()],
            },
        ];

        let groups = group_entries_by_kind(entries);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].kind, ReleaseEntryKind::Feat);
        assert_eq!(groups[0].entries.len(), 2);
        assert_eq!(groups[1].kind, ReleaseEntryKind::Fix);
        assert_eq!(groups[1].entries.len(), 1);
    }

    #[test]
    fn test_render_changelog_markdown() {
        use chrono::Utc;

        let summary = ReleaseSummary {
            branch: "main".to_string(),
            from_revision: None,
            to_revision: None,
            total_entries: 2,
            groups: vec![
                ReleaseGroup {
                    kind: ReleaseEntryKind::Feat,
                    label: "Features".to_string(),
                    entries: vec![ReleaseEntry {
                        kind: ReleaseEntryKind::Feat,
                        scope: Some("cli".to_string()),
                        description: "add release command".to_string(),
                        breaking: false,
                        revision_id: RevisionId::new("abc12345"),
                        timestamp: Utc::now(),
                        source_count: 1,
                        bases: vec!["code".to_string()],
                    }],
                },
                ReleaseGroup {
                    kind: ReleaseEntryKind::Fix,
                    label: "Bug Fixes".to_string(),
                    entries: vec![ReleaseEntry {
                        kind: ReleaseEntryKind::Fix,
                        scope: None,
                        description: "resolve crash".to_string(),
                        breaking: true,
                        revision_id: RevisionId::new("def67890"),
                        timestamp: Utc::now(),
                        source_count: 1,
                        bases: vec!["code".to_string()],
                    }],
                },
            ],
            memory_entries: vec![],
            dry_run: false,
        };

        let md = render_changelog_markdown(&summary, "v1.0.0");

        assert!(md.contains("# Changelog"));
        assert!(md.contains("## v1.0.0"));
        assert!(md.contains("### Features"));
        assert!(md.contains("**cli:** add release command"));
        assert!(md.contains("### Bug Fixes"));
        assert!(md.contains("**BREAKING:** resolve crash"));
    }

    #[test]
    fn test_render_changelog_with_memory_section() {
        use chrono::Utc;

        let summary = ReleaseSummary {
            branch: "main".to_string(),
            from_revision: None,
            to_revision: None,
            total_entries: 0,
            groups: vec![],
            memory_entries: vec![
                MemoryReleaseEntry {
                    operation: MemoryOperationType::Ingest,
                    description: "Add architecture decisions".to_string(),
                    revision_id: RevisionId::new("mem12345"),
                    timestamp: Utc::now(),
                    count: 5,
                    archived_count: None,
                    deleted_count: None,
                },
                MemoryReleaseEntry {
                    operation: MemoryOperationType::Prune,
                    description: "Prune old entries".to_string(),
                    revision_id: RevisionId::new("prune789"),
                    timestamp: Utc::now(),
                    count: 10,
                    archived_count: Some(7),
                    deleted_count: Some(3),
                },
            ],
            dry_run: false,
        };

        let md = render_changelog_markdown(&summary, "v2.0.0");

        assert!(md.contains("# Changelog"));
        assert!(md.contains("## v2.0.0"));
        assert!(md.contains("### Memory"));
        assert!(md.contains("**ingest:** Add architecture decisions (5 entries)"));
        assert!(md.contains("**prune:** Prune old entries (10 entries: 7 archived, 3 deleted)"));
    }
}
