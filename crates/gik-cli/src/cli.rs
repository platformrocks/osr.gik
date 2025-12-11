//! CLI definition and command dispatch for GIK.
//!
//! This module defines the command-line interface using `clap` and provides
//! the `run()` function that dispatches commands to the engine.
//!
//! ## Configuration Precedence
//!
//! Configuration is resolved with the following precedence (highest to lowest):
//! 1. CLI flags (e.g., `--config`, `--verbose`)
//! 2. Environment variables (`GIK_CONFIG`, `GIK_VERBOSE`, `GIK_DEVICE`)
//! 3. Config file (`~/.gik/config.yaml` or path from `--config`/`GIK_CONFIG`)
//! 4. Built-in defaults

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::ui::{format, table, ColorMode, MessageType, Progress, ProgressMode, Style};

use gik_core::memory::{MemoryEntry, MemoryScope, MemorySource};
use gik_core::{
    AddOptions, CommitOptions, GikEngine, GikError, KgExportFormat, ReindexOptions, ReleaseMode,
    ReleaseOptions, ReleaseRange, RevisionId, ShowOptions, StatsQuery,
};

// ============================================================================
// CLI Definition
// ============================================================================

/// Version string including git commit hash
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")");

/// Guided Indexing Kernel – local-first knowledge engine
#[derive(Parser, Debug)]
#[command(name = "gik")]
#[command(author, version = VERSION, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Enable verbose output (debug logging)
    #[arg(short, long, global = true, env = "GIK_VERBOSE")]
    pub verbose: bool,

    /// Suppress progress and informational messages
    #[arg(short, long, global = true, env = "GIK_QUIET")]
    pub quiet: bool,

    /// Path to configuration file (default: ~/.gik/config.yaml)
    #[arg(long, global = true, env = "GIK_CONFIG")]
    pub config: Option<PathBuf>,

    /// Device preference for embedding inference (auto/gpu/cpu)
    #[arg(long, global = true, env = "GIK_DEVICE")]
    pub device: Option<String>,

    /// Color output mode: always, never, or auto (default: auto)
    #[arg(long, global = true, env = "GIK_COLOR", default_value = "auto")]
    pub color: String,

    #[command(subcommand)]
    pub command: Command,
}

/// Available commands
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Initialize GIK structures for the current workspace
    #[command(after_help = r#"EXAMPLES:
    # Initialize GIK in current directory
    gik init

    # Typical first-time workflow
    gik init && gik add . && gik commit -m "Initial indexing"
"#)]
    Init,

    /// Show current GIK status (bases, stats, staging)
    #[command(after_help = r#"EXAMPLES:
    # Show current status
    gik status

    # Get status as JSON for scripting
    gik status --json

    # Pipe to jq for specific fields
    gik status --json | jq '.head.revisionId'
"#)]
    Status {
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// List available knowledge bases for the current branch
    #[command(after_help = r#"EXAMPLES:
    # List all bases
    gik bases

    # Common bases: code, docs, memory
"#)]
    Bases,

    /// Stage sources and update stack (paths, URLs, archives)
    #[command(after_help = r#"EXAMPLES:
    # Stage current directory
    gik add .

    # Stage specific files or folders
    gik add src/ README.md

    # Stage documentation to docs base
    gik add docs/ --base docs

    # Stage from a URL
    gik add https://example.com/api-docs.html --base docs

    # Add a memory entry
    gik add --memory "We chose PostgreSQL for ACID guarantees"

    # Add a memory entry with scope and source
    gik add --memory "Feature X uses Redis caching" --scope branch --source decision
"#)]
    Add {
        /// Targets to stage (paths, URLs, or archive references). Defaults to "." if omitted.
        #[arg(default_value = ".", conflicts_with = "memory")]
        targets: Vec<String>,

        /// Target knowledge base (e.g., "code", "docs"). If omitted, inferred from source type.
        #[arg(long, conflicts_with = "memory")]
        base: Option<String>,

        /// Add a memory entry instead of staging files.
        #[arg(long, value_name = "TEXT")]
        memory: Option<String>,

        /// Memory scope: 'project' (default), 'branch', or 'global'.
        #[arg(long, default_value = "project", requires = "memory")]
        scope: String,

        /// Memory source type: 'manual_note' (default), 'decision', 'observation', 'external_reference', 'agent_generated', 'commit_context'.
        #[arg(long, default_value = "manual_note", requires = "memory")]
        source: String,
    },

    /// Remove files from the staging area
    #[command(name = "rm", after_help = r#"EXAMPLES:
    # Remove a specific file from staging
    gik rm src/main.rs

    # Remove multiple files from staging
    gik rm src/lib.rs README.md

    # Remove files in a directory from staging
    gik rm src/utils/helper.rs src/utils/parser.rs
"#)]
    Rm {
        /// Files to remove from staging (paths relative to workspace root).
        #[arg(required = true)]
        targets: Vec<String>,
    },

    /// Index staged sources/memory and create a new revision
    #[command(after_help = r#"EXAMPLES:
    # Commit staged sources with auto-generated message
    gik commit

    # Commit with a descriptive message
    gik commit -m "Index API documentation"

    # Typical workflow
    gik add . && gik commit -m "Initial indexing"
"#)]
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: Option<String>,
    },

    /// Show knowledge timeline (revisions) for the current branch
    #[command(after_help = r#"EXAMPLES:
    # Show revision timeline
    gik log

    # Show only commit operations
    gik log --op commit

    # Show ask query history
    gik log --kind ask

    # Filter by date range
    gik log --since 2025-01-01T00:00:00Z --limit 10

    # Output as JSON
    gik log --json
"#)]
    Log {
        /// Log kind: 'timeline' (default) or 'ask'
        #[arg(long, default_value = "timeline")]
        kind: String,

        /// Filter by timeline operation: init, commit, reindex, release (comma-separated)
        #[arg(long, value_delimiter = ',')]
        op: Option<Vec<String>>,

        /// Filter by base name (comma-separated)
        #[arg(long, value_delimiter = ',')]
        base: Option<Vec<String>>,

        /// Filter entries since this timestamp (RFC 3339, e.g., 2024-01-15T10:00:00Z)
        #[arg(long)]
        since: Option<String>,

        /// Filter entries until this timestamp (RFC 3339, e.g., 2024-01-15T10:00:00Z)
        #[arg(long)]
        until: Option<String>,

        /// Maximum number of entries to return
        #[arg(short = 'n', long)]
        limit: Option<usize>,

        /// Output in JSON format
        #[arg(long)]
        json: bool,

        /// Output in JSONL format (one JSON object per line)
        #[arg(long)]
        jsonl: bool,
    },

    /// Query knowledge (RAG/stack/memory/KG) and return context
    #[command(after_help = r#"EXAMPLES:
    # Ask a simple question
    gik ask "How does authentication work?"

    # Search only specific bases
    gik ask "API endpoints" --bases code,docs

    # Get more context with higher top_k (controls final output count)
    gik ask "error handling" --top-k 16

    # Filter low-confidence results
    gik ask "config" --min-score 0.5

    # Output as JSON for scripting
    gik ask "database schema" --json
"#)]
    Ask {
        /// The question to ask
        query: String,

        /// Restrict RAG to specific bases (comma-separated)
        #[arg(long, value_delimiter = ',')]
        bases: Option<Vec<String>>,

        /// Maximum chunks to return (default: 8). Overrides config's retrieval.reranker.finalK.
        #[arg(long, default_value = "8")]
        top_k: usize,

        /// Minimum relevance score (0.0-1.0) to include chunks
        #[arg(long)]
        min_score: Option<f32>,

        /// Output in JSON format
        #[arg(long)]
        json: bool,

        /// Pretty-print JSON output
        #[arg(long)]
        pretty: bool,
    },

    /// Show aggregated stats for all bases or a single base
    #[command(after_help = r#"EXAMPLES:
    # Show stats for all bases
    gik stats

    # Show stats for a specific base
    gik stats --base code

    # Output as JSON
    gik stats --json
"#)]
    Stats {
        /// Base to query (omit for all bases)
        #[arg(long)]
        base: Option<String>,

        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Rebuild embeddings and index for a specific base
    #[command(after_help = r#"EXAMPLES:
    # Reindex code base after model upgrade
    gik reindex --base code

    # Force reindex even if model hasn't changed
    gik reindex --base docs --force

    # Preview what would be reindexed
    gik reindex --base code --dry-run
"#)]
    Reindex {
        /// The base to reindex
        #[arg(long, required = true)]
        base: String,

        /// Force reindex even if model hasn't changed
        #[arg(long)]
        force: bool,

        /// Dry run: report what would change without writing
        #[arg(long)]
        dry_run: bool,

        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Generate CHANGELOG.md from commit history
    #[command(after_help = r#"EXAMPLES:
    # Generate changelog for unreleased changes
    gik release

    # Create a tagged release
    gik release --tag v1.0.0

    # Append to existing changelog
    gik release --tag v1.1.0 --append

    # Preview without writing
    gik release --dry-run

    # Generate for specific revision range
    gik release --from abc123 --to def456
"#)]
    Release {
        /// Release tag (e.g., v1.0.0). If not provided, uses "Unreleased"
        #[arg(long)]
        tag: Option<String>,

        /// Branch to generate changelog for (defaults to current branch)
        #[arg(short, long)]
        branch: Option<String>,

        /// Starting revision (exclusive). If not provided, starts from beginning
        #[arg(long)]
        from: Option<String>,

        /// Ending revision (inclusive). If not provided, ends at HEAD
        #[arg(long)]
        to: Option<String>,

        /// Append to existing CHANGELOG.md instead of replacing. Requires --tag.
        #[arg(long)]
        append: bool,

        /// Dry run: show what would be written without actually writing
        #[arg(long)]
        dry_run: bool,

        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Inspect a single knowledge revision (similar to `git show`)
    #[command(after_help = r#"EXAMPLES:
    # Show current HEAD revision
    gik show

    # Show a specific revision by ID
    gik show abc12345

    # Show previous revision
    gik show HEAD~1

    # Output as JSON
    gik show --json

    # Export KG as Mermaid diagram
    gik show --kg-mermaid

    # Export KG as DOT (Graphviz)
    gik show --kg-dot > graph.dot
"#)]
    Show {
        /// Revision reference (ID, prefix, HEAD, HEAD~N). Defaults to HEAD.
        #[arg(default_value = "HEAD")]
        revision: String,

        /// Branch to inspect (defaults to current branch)
        #[arg(short, long)]
        branch: Option<String>,

        /// Output in JSON format
        #[arg(long)]
        json: bool,

        /// Output KG subgraph in DOT format (Graphviz)
        #[arg(long)]
        kg_dot: bool,

        /// Output KG subgraph in Mermaid format
        #[arg(long)]
        kg_mermaid: bool,

        /// Maximum number of source paths to display
        #[arg(long, default_value = "20")]
        max_sources: usize,

        /// Maximum number of KG nodes to export
        #[arg(long, default_value = "50")]
        max_kg_nodes: usize,

        /// Maximum number of KG edges to export
        #[arg(long, default_value = "100")]
        max_kg_edges: usize,
    },

    /// Manage GIK configuration (validate, show resolved config)
    #[command(after_help = r#"EXAMPLES:
    # Validate configuration files
    gik config validate

    # Show resolved configuration (all sources merged)
    gik config show

    # Output as JSON
    gik config show --json
"#)]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

/// Config subcommands
#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Validate configuration files and report errors/warnings
    #[command(name = "check", after_help = r#"EXAMPLES:
    # Validate all config sources
    gik config check

    # Output as JSON
    gik config check --json
"#)]
    Check {
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },

    /// Show resolved configuration (merged from all sources)
    #[command(after_help = r#"EXAMPLES:
    # Show resolved config
    gik config show

    # Output as JSON
    gik config show --json
"#)]
    Show {
        /// Output in JSON format
        #[arg(long)]
        json: bool,
    },
}

// ============================================================================
// Run function
// ============================================================================

/// Run the CLI application.
///
/// Parses command-line arguments, creates a `GikEngine`, and dispatches
/// to the appropriate command handler.
///
/// Configuration precedence:
/// 1. CLI flags (`--config`, `--verbose`)
/// 2. Environment variables (`GIK_CONFIG`, `GIK_VERBOSE`)
/// 3. Config file (`~/.gik/config.yaml` or custom path)
/// 4. Built-in defaults
///
/// # Returns
///
/// Returns `ExitCode::SUCCESS` on success, or `ExitCode::FAILURE` on error.
pub fn run() -> ExitCode {
    let cli = Cli::parse();

    // Initialize tracing subscriber
    // - Always show warnings (for config issues, deprecations, etc.)
    // - Show debug info only when --verbose is set
    let log_level = if cli.verbose { "debug" } else { "warn" };
    let filter = format!("gik_core={},gik_cli={}", log_level, log_level);
    
    tracing_subscriber::fmt()
        .with_env_filter(&filter)
        .with_target(false)
        .init();

    // Create engine with configuration
    // Priority: --config flag > GIK_CONFIG env > ~/.gik/config.yaml
    let engine = match &cli.config {
        Some(config_path) => GikEngine::with_config(config_path),
        None => GikEngine::with_defaults(),
    };

    // Parse color mode from --color flag
    let color_mode = ColorMode::from_str(&cli.color).unwrap_or(ColorMode::Auto);
    let style = Style::new(color_mode);

    let mut engine = match engine {
        Ok(engine) => engine,
        Err(e) => {
            let hint = if let Some(path) = &cli.config {
                format!("Check your config at {}", path.display())
            } else {
                "Check your global config at ~/.gik/config.yaml".to_string()
            };
            eprintln!(
                "{}",
                style.error_with_context(
                    "Failed to initialize GIK engine",
                    Some(&e.to_string()),
                    Some(&hint),
                )
            );
            return ExitCode::FAILURE;
        }
    };

    // Apply device override if specified via --device or GIK_DEVICE
    if let Some(device_str) = &cli.device {
        match device_str.to_lowercase().as_str() {
            "auto" => engine.set_device(gik_core::config::DevicePreference::Auto),
            "gpu" => engine.set_device(gik_core::config::DevicePreference::Gpu),
            "cpu" => engine.set_device(gik_core::config::DevicePreference::Cpu),
            _ => {
                eprintln!(
                    "{}",
                    style.error_with_context(
                        &format!("Invalid device preference '{}'", device_str),
                        None,
                        Some("Valid options: auto, gpu, cpu"),
                    )
                );
                return ExitCode::FAILURE;
            }
        }
    }

    // Resolve workspace from current directory
    let workspace = match engine.resolve_workspace(Path::new(".")) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!(
                "{}",
                style.message(MessageType::Err, &format!("Failed to resolve workspace: {}", e))
            );
            return ExitCode::FAILURE;
        }
    };

    // Get current branch
    let branch = match engine.current_branch(&workspace) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "{}",
                style.message(MessageType::Err, &format!("Failed to determine branch: {}", e))
            );
            return ExitCode::FAILURE;
        }
    };

    // Dispatch to command handler (pass verbose flag for commands that need it)
    let result = match cli.command {
        Command::Init => handle_init(&style, &engine, &workspace),
        Command::Status { json } => handle_status(&style, &engine, &workspace, &branch, json),
        Command::Bases => handle_bases(&style, &engine, &workspace, &branch),
        Command::Add { targets, base, memory, scope, source } => {
            if let Some(text) = memory {
                handle_add_memory(&style, &engine, &workspace, &branch, text, scope, source)
            } else {
                handle_add(&style, &engine, &workspace, &branch, targets, base)
            }
        }
        Command::Rm { targets } => handle_rm(&style, &engine, &workspace, &branch, targets),
        Command::Commit { message } => handle_commit(&style, &engine, &workspace, message),
        Command::Log {
            kind,
            op,
            base,
            since,
            until,
            limit,
            json,
            jsonl,
        } => handle_log(
            &style, &engine, &workspace, kind, op, base, since, until, limit, json, jsonl,
        ),
        Command::Ask {
            query,
            bases,
            top_k,
            min_score,
            json,
            pretty,
        } => handle_ask(
            &style,
            &engine,
            &workspace,
            query,
            bases,
            top_k,
            min_score,
            json,
            pretty,
            cli.verbose,
        ),
        Command::Stats { base, json } => handle_stats(&style, &engine, &workspace, &branch, base, json),
        Command::Reindex {
            base,
            force,
            dry_run,
            json,
        } => handle_reindex(&style, &engine, &workspace, base, force, dry_run, json),
        Command::Release {
            tag,
            branch: release_branch,
            from,
            to,
            append,
            dry_run,
            json,
        } => handle_release(
            &style,
            &engine,
            &workspace,
            tag,
            release_branch,
            from,
            to,
            append,
            dry_run,
            json,
        ),
        Command::Show {
            revision,
            branch: show_branch,
            json,
            kg_dot,
            kg_mermaid,
            max_sources,
            max_kg_nodes,
            max_kg_edges,
        } => handle_show(
            &style,
            &engine,
            &workspace,
            revision,
            show_branch,
            json,
            kg_dot,
            kg_mermaid,
            max_sources,
            max_kg_nodes,
            max_kg_edges,
        ),
        Command::Config { action } => handle_config(&style, &engine, &workspace, action),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", style.message(MessageType::Err, &e.to_string()));
            ExitCode::FAILURE
        }
    }
}

// ============================================================================
// Command handlers
// ============================================================================

fn handle_init(style: &Style, engine: &GikEngine, workspace: &gik_core::Workspace) -> Result<(), GikError> {
    let branch = engine.current_branch(workspace)?;

    match engine.init_workspace(workspace) {
        Ok((revision_id, stats)) => {
            let lang_count = stats.languages.len();
            println!(
                "{}",
                style.message(
                    MessageType::Ok,
                    &format!(
                        "Initialized GIK workspace at {} on branch `{}` (revision {})",
                        workspace.root().display(),
                        branch,
                        style.revision(revision_id.as_str())
                    )
                )
            );
            println!(
                "{}",
                style.message_detail(
                    "Scanned",
                    &format!("{} files, {} languages", stats.total_files, lang_count)
                )
            );
            if !stats.managers.is_empty() {
                println!(
                    "{}",
                    style.message_detail("Managers", &stats.managers.join(", "))
                );
            }
            // Print next steps hint
            println!();
            println!("{}", style.message(MessageType::Hint, "Next steps:"));
            println!("  1. Stage sources:   gik add .");
            println!("  2. Index them:      gik commit -m \"Initial indexing\"");
            println!("  3. Query knowledge: gik ask \"How does this work?\"");
            Ok(())
        }
        Err(GikError::AlreadyInitialized { branch, head }) => {
            // Not an error - just informational
            println!(
                "{}",
                style.message(
                    MessageType::Info,
                    &format!(
                        "GIK workspace already initialized on branch `{}` (HEAD: {})",
                        branch,
                        style.revision(&head)
                    )
                )
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn handle_status(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    branch: &gik_core::BranchName,
    json: bool,
) -> Result<(), GikError> {
    let status = engine.status(workspace, branch)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&status).unwrap_or_default()
        );
    } else {
        println!("{}", style.section("STATUS"));
        println!();
        println!("  {}", style.key_value("Workspace", &status.workspace_root.display().to_string()));
        println!(
            "  {}",
            style.key_value(
                "Branch",
                &format!("{} (initialized: {})", status.branch, if status.is_initialized { "yes" } else { "no" })
            )
        );

        // HEAD info
        if let Some(head) = &status.head {
            let timestamp = head.timestamp.format("%Y-%m-%d %H:%M:%S UTC");
            let msg = head.message.as_deref().unwrap_or("");
            println!(
                "  {}",
                style.key_value(
                    "HEAD",
                    &format!("{} ({:?}) at {}", style.revision(&head.revision_id), head.operation, timestamp)
                )
            );
            if !msg.is_empty() {
                println!("        \"{}\"", msg);
            }
        } else {
            println!("  {}", style.key_value("HEAD", "(none)"));
        }

        // Git-like staged/unstaged status (Phase 8.4)
        let has_staged = status.staged_files.as_ref().map_or(false, |f| !f.is_empty());
        let has_modified = status.modified_files.as_ref().map_or(false, |f| !f.is_empty());
        let working_tree_clean = status.working_tree_clean.unwrap_or(true);

        if has_staged || has_modified {
            println!();

            // Staged files (green)
            if has_staged {
                println!("Changes to be committed:");
                println!("  (use \"gik rm <file>...\" to unstage)");
                println!();
                if let Some(staged) = &status.staged_files {
                    for sf in staged {
                        match sf.change_type {
                            gik_core::ChangeType::New => {
                                println!("{}", style.staged_new(&sf.path));
                            }
                            gik_core::ChangeType::Modified => {
                                println!("{}", style.staged_modified(&sf.path));
                            }
                            gik_core::ChangeType::Unchanged => {
                                // Unchanged files shouldn't be in staged list
                            }
                        }
                    }
                }
            }

            // Modified but not staged (red)
            if has_modified {
                if has_staged {
                    println!();
                }
                println!("Changes not staged for commit:");
                println!("  (use \"gik add <file>...\" to update what will be committed)");
                println!();
                if let Some(modified) = &status.modified_files {
                    for path in modified {
                        println!("{}", style.unstaged_modified(path));
                    }
                }
            }
        } else if working_tree_clean && status.is_initialized {
            println!();
            println!("nothing to commit, working tree clean");
        }

        // Legacy staging info (for backwards compatibility)
        if let Some(staging) = &status.staging {
            if staging.pending_count > 0 || staging.indexed_count > 0 || staging.failed_count > 0 {
                println!();
                println!(
                    "  {}",
                    style.key_value(
                        "Staging",
                        &format!("pending={} indexed={} failed={}", staging.pending_count, staging.indexed_count, staging.failed_count)
                    )
                );
            }
        }

        // Stack info
        if let Some(summary) = &status.stack_summary {
            let file_count = summary.total_files.unwrap_or(0);
            let langs = if summary.languages.is_empty() {
                "(none)".to_string()
            } else {
                summary.languages.join(", ")
            };
            let mgrs = if summary.managers.is_empty() {
                "(none)".to_string()
            } else {
                summary.managers.join(", ")
            };
            println!(
                "  {}",
                style.key_value("Stack", &format!("{} files | {} | {}", file_count, langs, mgrs))
            );
            if !summary.frameworks.is_empty() {
                println!("    Frameworks: {}", summary.frameworks.join(", "));
            }
            if !summary.services.is_empty() {
                println!("    Services: {}", summary.services.join(", "));
            }
        } else if let Some(stack) = &status.stack {
            let lang_count = stack.languages.len();
            println!(
                "  {}",
                style.key_value("Stack", &format!("files={} languages={}", stack.total_files, lang_count))
            );
        } else {
            println!("  {}", style.key_value("Stack", "(none)"));
        }

        // Per-base stats using table
        if let Some(bases) = &status.bases {
            println!();
            println!("{}", style.section("BASES"));
            println!();
            let base_rows: Vec<table::BaseRow> = bases
                .iter()
                .map(|b| table::BaseRow {
                    name: b.base.clone(),
                    documents: b.documents,
                    vectors: b.vectors,
                    files: b.files,
                    size_bytes: b.on_disk_bytes,
                    health: b.health.to_string(),
                    last_indexed: b.last_commit,
                })
                .collect();
            println!("{}", table::render_bases_table(&base_rows, true));
        }
    }
    Ok(())
}

fn handle_bases(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    branch: &gik_core::BranchName,
) -> Result<(), GikError> {
    // Get full status to access base stats
    let status = engine.status(workspace, branch)?;

    if let Some(bases) = &status.bases {
        if bases.is_empty() {
            println!(
                "{}",
                style.message(MessageType::Info, "No bases found. Run `gik add` and `gik commit` to create.")
            );
        } else {
            println!("{}", style.section("BASES"));
            println!();
            let base_rows: Vec<table::BaseRow> = bases
                .iter()
                .map(|b| table::BaseRow {
                    name: b.base.clone(),
                    documents: b.documents,
                    vectors: b.vectors,
                    files: b.files,
                    size_bytes: b.on_disk_bytes,
                    health: b.health.to_string(),
                    last_indexed: b.last_commit,
                })
                .collect();
            println!("{}", table::render_bases_table(&base_rows, true));
        }
    } else {
        println!(
            "{}",
            style.message(MessageType::Info, "No bases found. Run `gik init` to initialize.")
        );
    }
    Ok(())
}

fn handle_add(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    branch: &gik_core::BranchName,
    targets: Vec<String>,
    base: Option<String>,
) -> Result<(), GikError> {
    let opts = AddOptions {
        targets: targets.clone(),
        base,
    };
    let result = engine.add(workspace, branch, opts)?;

    // Print summary of created sources
    if !result.created.is_empty() {
        println!(
            "{}",
            style.message(
                MessageType::Ok,
                &format!("Staged {} source(s) for indexing", result.created.len())
            )
        );
        for target in &targets {
            // Only show targets that weren't skipped
            let was_skipped = result.skipped.iter().any(|s| &s.raw == target);
            if !was_skipped {
                println!("{}", style.list_item("+", target));
            }
        }
    }

    // Print skipped sources
    if !result.skipped.is_empty() {
        println!(
            "{}",
            style.message(
                MessageType::Skip,
                &format!("Skipped {} source(s)", result.skipped.len())
            )
        );
        for skip in &result.skipped {
            println!("{}", style.list_item("-", &format!("{} ({})", skip.raw, skip.reason)));
        }
    }

    // Print stack stats if available
    if let Some(stats) = &result.stack_stats {
        let lang_count = stats.languages.len();
        println!(
            "{}",
            style.message_detail("Stack", &format!("{} files, {} languages", stats.total_files, lang_count))
        );
    }

    // If nothing was staged AND there were skips, all targets failed
    if result.created.is_empty() {
        if !result.skipped.is_empty() {
            return Err(GikError::InvalidArgument(format!(
                "No sources staged: all {} target(s) were skipped",
                result.skipped.len()
            )));
        }
        println!("{}", style.message(MessageType::Info, "Nothing to stage."));
    }

    Ok(())
}

fn handle_add_memory(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    branch: &gik_core::BranchName,
    text: String,
    scope_str: String,
    source_str: String,
) -> Result<(), GikError> {
    // Parse scope
    let scope: MemoryScope = scope_str.parse().map_err(|e: String| {
        GikError::InvalidArgument(format!(
            "Invalid --scope value '{}'. Valid options: project, branch, global. Error: {}",
            scope_str, e
        ))
    })?;

    // Parse source
    let source: MemorySource = source_str.parse().map_err(|e: String| {
        GikError::InvalidArgument(format!(
            "Invalid --source value '{}'. Valid options: manual_note, decision, observation, external_reference, agent_generated, commit_context. Error: {}",
            source_str, e
        ))
    })?;

    // Create memory entry
    let mut entry = MemoryEntry::new(scope, source, &text);
    
    // If branch-scoped, attach the branch name
    if scope == MemoryScope::Branch {
        entry = entry.with_branch(branch.as_str());
    }

    // Ingest the memory entry
    let result = engine.ingest_memory(workspace, vec![entry], Some("Add memory entry via CLI"))?;

    if result.result.ingested_count > 0 {
        println!(
            "{}",
            style.message(
                MessageType::Ok,
                "Memory entry added successfully"
            )
        );
        println!("  {}", style.key_value("Scope", &scope.to_string()));
        println!("  {}", style.key_value("Source", &source.to_string()));
        if let Some(rev_id) = &result.revision_id {
            println!("  {}", style.key_value("Revision", rev_id));
        }
        // Show first 80 chars of text as preview
        let preview = if text.len() > 80 {
            format!("{}...", &text[..80])
        } else {
            text.clone()
        };
        println!("  {}", style.key_value("Text", &preview));
    } else if !result.result.failed.is_empty() {
        let (id, err) = &result.result.failed[0];
        return Err(GikError::InvalidArgument(format!(
            "Failed to add memory entry {}: {}",
            id, err
        )));
    }

    Ok(())
}

fn handle_rm(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    branch: &gik_core::BranchName,
    targets: Vec<String>,
) -> Result<(), GikError> {
    let opts = gik_core::UnstageOptions { targets: targets.clone() };
    let result = engine.unstage(workspace, branch, opts)?;

    // Print summary of removed sources
    if !result.unstaged.is_empty() {
        println!(
            "{}",
            style.message(
                MessageType::Ok,
                &format!("Removed {} source(s) from staging", result.unstaged.len())
            )
        );
        for path in &result.unstaged {
            println!("{}", style.list_item("-", path));
        }
    }

    // Print not found sources
    if !result.not_found.is_empty() {
        println!(
            "{}",
            style.message(
                MessageType::Skip,
                &format!("{} source(s) not found in staging", result.not_found.len())
            )
        );
        for skip in &result.not_found {
            println!("{}", style.list_item("?", &format!("{} ({})", skip.raw, skip.reason)));
        }
    }

    // If nothing was removed AND there were not_found, all targets failed
    if result.unstaged.is_empty() {
        if !result.not_found.is_empty() {
            return Err(GikError::InvalidArgument(format!(
                "No sources removed: {} target(s) not found in staging",
                result.not_found.len()
            )));
        }
        println!("{}", style.message(MessageType::Info, "Nothing to remove from staging."));
    }

    Ok(())
}

fn handle_commit(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    message: Option<String>,
) -> Result<(), GikError> {
    let opts = CommitOptions {
        message: message.clone(),
        use_mock_backend: false, // Production uses real backend
    };

    // Determine progress mode based on quiet flag
    let mode = if style.color_mode() == ColorMode::Never {
        ProgressMode::Quiet
    } else {
        ProgressMode::Interactive
    };

    // Show animated spinner during commit operation
    let progress = Progress::spinner("Indexing sources...", mode);

    let result = engine.commit(workspace, opts);

    // Finish spinner and show step summary
    progress.finish_clear();

    let result = result?;

    // Show completed steps summary
    if mode.is_interactive() {
        println!("├─ Parsing sources done");
        println!("├─ Generating embeddings done");
        println!("└─ Building index done");
    }

    // Display commit results
    println!(
        "{}",
        style.message(
            MessageType::Ok,
            &format!("gik commit {}", style.revision(&result.revision_id))
        )
    );

    if result.total_indexed > 0 {
        println!(
            "{}",
            style.message_detail(
                "Indexed",
                &format!("{} source{}", result.total_indexed, if result.total_indexed == 1 { "" } else { "s" })
            )
        );
    }

    if result.total_failed > 0 {
        println!(
            "{}",
            style.message_detail(
                "Failed",
                &format!("{} source{}", result.total_failed, if result.total_failed == 1 { "" } else { "s" })
            )
        );
    }

    if !result.touched_bases.is_empty() {
        println!(
            "{}",
            style.message_detail("Bases", &result.touched_bases.join(", "))
        );
    }

    // Show per-base details if multiple bases
    if result.bases.len() > 1 {
        println!();
        for base in &result.bases {
            println!(
                "    [{}] {} indexed, {} failed, {} chunks",
                base.base, base.indexed_count, base.failed_count, base.chunk_count
            );
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_log(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    kind: String,
    op: Option<Vec<String>>,
    base: Option<Vec<String>>,
    since: Option<String>,
    until: Option<String>,
    limit: Option<usize>,
    json: bool,
    jsonl: bool,
) -> Result<(), GikError> {
    use gik_core::{LogEntry, LogKind, LogQueryScope, TimelineOperationKind};

    // Parse log kind
    let log_kind: LogKind = kind
        .parse()
        .map_err(|e: String| GikError::Other(anyhow::anyhow!("Invalid log kind: {}", e)))?;

    // Build scope
    let mut scope = LogQueryScope::new().with_kind(log_kind);

    // Parse operation filters
    if let Some(ops) = op {
        let parsed_ops: Vec<TimelineOperationKind> =
            ops.iter().map(|s| s.parse().unwrap()).collect();
        scope = scope.with_ops(parsed_ops);
    }

    // Add base filter
    if let Some(bases) = base {
        scope = scope.with_bases(bases);
    }

    // Parse time filters
    if let Some(since_str) = since {
        let since_dt = chrono::DateTime::parse_from_rfc3339(&since_str)
            .map_err(|e| GikError::Other(anyhow::anyhow!("Invalid --since timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        scope = scope.with_since(since_dt);
    }

    if let Some(until_str) = until {
        let until_dt = chrono::DateTime::parse_from_rfc3339(&until_str)
            .map_err(|e| GikError::Other(anyhow::anyhow!("Invalid --until timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        scope = scope.with_until(until_dt);
    }

    // Add limit
    if let Some(lim) = limit {
        scope = scope.with_limit(lim);
    }

    // Run query
    let result = engine.log_query(workspace, scope)?;

    if result.entries.is_empty() {
        if !json && !jsonl {
            println!("{}", style.message(MessageType::Info, "No log entries found."));
        } else if json {
            println!("[]");
        }
        // jsonl: print nothing for empty
    } else if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result.entries).unwrap_or_default()
        );
    } else if jsonl {
        for entry in &result.entries {
            println!("{}", serde_json::to_string(entry).unwrap_or_default());
        }
    } else {
        // Human-readable output
        match log_kind {
            LogKind::Timeline => {
                println!("{}", style.section("TIMELINE"));
                println!();
                for entry in &result.entries {
                    if let LogEntry::Timeline(t) = entry {
                        let msg = t.message.as_deref().unwrap_or("");
                        let ts = format::format_relative_time(t.timestamp);

                        // Build bases summary with health (if available in meta)
                        let bases_summary = if let Some(meta) = &t.meta {
                            if let Some(base_stats) =
                                meta.get("baseStats").and_then(|v| v.as_array())
                            {
                                let parts: Vec<String> = base_stats
                                    .iter()
                                    .filter_map(|bs| {
                                        let base = bs.get("base")?.as_str()?;
                                        let docs = bs.get("documents")?.as_u64().unwrap_or(0);
                                        Some(format!("{}:{}", base, docs))
                                    })
                                    .collect();
                                if !parts.is_empty() {
                                    format!(" [{}]", parts.join(", "))
                                } else {
                                    String::new()
                                }
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };

                        println!(
                            "  {} {:>10} {:>12}{}",
                            style.revision(&t.revision_id),
                            ts,
                            t.operation,
                            bases_summary
                        );
                        if !msg.is_empty() {
                            println!("       \"{}\"", msg);
                        }
                    }
                }
            }
            LogKind::Ask => {
                println!("{}", style.section("ASK LOG"));
                println!();
                for entry in &result.entries {
                    if let LogEntry::Ask(a) = entry {
                        println!(
                            "  {} [{}] \"{}\" ({} hits)",
                            format::format_relative_time(a.timestamp),
                            a.bases.join(","),
                            format::truncate_str(&a.question, 50),
                            a.total_hits
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_ask(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    query: String,
    bases: Option<Vec<String>>,
    top_k: usize,
    min_score: Option<f32>,
    json: bool,
    pretty: bool,
    verbose: bool,
) -> Result<(), GikError> {
    // Get current branch
    let branch = engine.current_branch(workspace)?;

    // Build ask options
    // Use top_k for both the per-base retrieval limit AND the final output count.
    // This ensures --top-k controls how many chunks the user actually receives.
    let opts = gik_core::AskPipelineOptions::new(&query)
        .with_top_k(top_k)
        .with_final_k(top_k)  // Override reranker's finalK with CLI value
        .with_stack(true);

    let opts = if let Some(b) = bases {
        opts.with_bases(b)
    } else {
        opts
    };

    let opts = if let Some(score) = min_score {
        opts.with_min_score(score)
    } else {
        opts
    };

    // Run the ask pipeline
    let result = engine.ask(workspace, &branch, opts)?;

    if json || pretty {
        let output = if pretty {
            serde_json::to_string_pretty(&result).unwrap_or_default()
        } else {
            serde_json::to_string(&result).unwrap_or_default()
        };
        println!("{}", output);
    } else {
        // Human-readable output
        println!("{}", style.section("QUERY"));
        println!();
        println!("  {}", style.key_value("Query", &query));
        println!("  {}", style.key_value("Revision", &style.revision(result.revision_id.as_str())));
        println!("  {}", style.key_value("Bases", &result.bases.join(", ")));

        if result.rag_chunks.is_empty() {
            println!();
            println!("{}", style.message(MessageType::Info, "No relevant chunks found."));
        } else {
            println!();
            println!("{}", style.section("RESULTS"));
            println!();
            println!(
                "{}",
                style.message(
                    MessageType::Ok,
                    &format!("Retrieved {} chunks", result.rag_chunks.len())
                )
            );
            println!();

            for (i, chunk) in result.rag_chunks.iter().enumerate() {
                println!(
                    "  {}. [{}] {} (lines {}-{}) - score: {}",
                    i + 1,
                    chunk.base,
                    style.file_path(&chunk.path),
                    chunk.start_line,
                    chunk.end_line,
                    style.score(chunk.score)
                );

                // Show snippet preview (first 100 chars)
                let snippet_preview: String = chunk
                    .snippet
                    .chars()
                    .take(100)
                    .collect::<String>()
                    .replace('\n', " ");
                if !snippet_preview.is_empty() {
                    println!(
                        "     {}{}",
                        snippet_preview,
                        if chunk.snippet.len() > 100 { "..." } else { "" }
                    );
                }
                println!();
            }
        }

        // Show stack summary if available
        if let Some(stack) = &result.stack_summary {
            println!("{}", style.section("STACK"));
            println!();
            if !stack.languages.is_empty() {
                println!("  {}", style.key_value("Languages", &stack.languages.join(", ")));
            }
            if !stack.frameworks.is_empty() {
                println!("  {}", style.key_value("Frameworks", &stack.frameworks.join(", ")));
            }
            if !stack.services.is_empty() {
                println!("  {}", style.key_value("Services", &stack.services.join(", ")));
            }
            if !stack.managers.is_empty() {
                println!("  {}", style.key_value("Managers", &stack.managers.join(", ")));
            }
            if let Some(count) = stack.total_files {
                println!("  {}", style.key_value("Files", &count.to_string()));
            }
        }

        // Show timing info only with --verbose flag
        if verbose {
            if let Some(embed_ms) = result.debug.embed_time_ms {
                if let Some(search_ms) = result.debug.search_time_ms {
                    println!();
                    println!(
                        "{}",
                        style.message(
                            MessageType::Info,
                            &format!("Timing: embed {}ms, search {}ms", embed_ms, search_ms)
                        )
                    );
                }
            }
        }
    }
    Ok(())
}

fn handle_stats(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    branch: &gik_core::BranchName,
    base: Option<String>,
    json: bool,
) -> Result<(), GikError> {
    let query = StatsQuery { base: base.clone() };
    let result = engine.stats(workspace, branch, query)?;

    if json {
        let output = serde_json::to_string_pretty(&result).map_err(GikError::Json)?;
        println!("{}", output);
    } else {
        let scope = base.as_deref().unwrap_or("all bases");
        println!("{}", style.section("STATS"));
        println!();
        println!("  {}", style.key_value("Scope", scope));
        println!("  {}", style.key_value("Branch", &result.branch.to_string()));
        println!();

        if result.bases.is_empty() {
            println!("{}", style.message(MessageType::Info, "No indexed bases found."));
        } else {
            // Build base rows for table
            let base_rows: Vec<table::BaseRow> = result
                .bases
                .iter()
                .map(|b| table::BaseRow {
                    name: b.base.clone(),
                    documents: b.documents,
                    vectors: b.vectors,
                    files: b.files,
                    size_bytes: b.on_disk_bytes,
                    health: b.health.to_string(),
                    last_indexed: None,
                })
                .collect();

            println!("{}", table::render_stats_breakdown(&base_rows, result.total_on_disk_bytes));

            println!();
            println!(
                "  Totals: {} documents, {} vectors, {}",
                format::format_thousands(result.total_documents),
                format::format_thousands(result.total_vectors),
                format::format_bytes(result.total_on_disk_bytes)
            );
        }

        // Stack summary if available
        if let Some(stack) = &result.stack {
            println!();
            println!("{}", style.section("STACK"));
            println!();
            println!("  {}", style.key_value("Files", &stack.total_files.to_string()));
            let langs: Vec<_> = stack.languages.keys().collect();
            if !langs.is_empty() {
                let lang_str: Vec<_> = langs.iter().map(|l| l.as_str()).collect();
                println!("  {}", style.key_value("Languages", &lang_str.join(", ")));
            }
            if !stack.managers.is_empty() {
                println!("  {}", style.key_value("Managers", &stack.managers.join(", ")));
            }
        }
    }
    Ok(())
}

fn handle_reindex(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    base: String,
    force: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), GikError> {
    let opts = ReindexOptions {
        base: base.clone(),
        branch: None, // Use current branch
        force,
        dry_run,
    };

    // Show progress during reindex (skip for dry-run)
    let result = if !dry_run && !json {
        let mode = if style.color_mode() == ColorMode::Never {
            ProgressMode::Quiet
        } else {
            ProgressMode::Interactive
        };
        let progress = Progress::spinner(&format!("Reindexing base '{}'...", base), mode);
        let result = engine.reindex(workspace, opts);
        match &result {
            Ok(_) => progress.finish_clear(),
            Err(_) => progress.finish_err("Failed"),
        }
        result?
    } else {
        engine.reindex(workspace, opts)?
    };

    if json {
        let output = serde_json::to_string_pretty(&result).map_err(GikError::Json)?;
        println!("{}", output);
        return Ok(());
    }

    // Human-readable output
    if dry_run {
        println!(
            "{}",
            style.message(MessageType::Info, &format!("Dry run for base '{}'", base))
        );
    } else if result.bases.iter().any(|b| b.reindexed) {
        println!(
            "{}",
            style.message(MessageType::Ok, &format!("Reindexed base '{}'", base))
        );
    } else {
        println!(
            "{}",
            style.message(
                MessageType::Skip,
                &format!("No reindex needed for base '{}' (model unchanged)", base)
            )
        );
        return Ok(());
    }

    for base_result in &result.bases {
        if base_result.reindexed {
            println!(
                "{}",
                style.message_detail(
                    "Model",
                    &format!(
                        "{} -> {}",
                        base_result.from_model_id.as_deref().unwrap_or("none"),
                        base_result.to_model_id
                    )
                )
            );
            println!(
                "{}",
                style.message_detail("Sources", &base_result.sources_processed.to_string())
            );
            println!(
                "{}",
                style.message_detail("Chunks", &base_result.chunks_reembedded.to_string())
            );

            if !base_result.errors.is_empty() {
                println!(
                    "{}",
                    style.message(
                        MessageType::Warn,
                        &format!("{} error(s) during reindex", base_result.errors.len())
                    )
                );
                for error in &base_result.errors {
                    println!("{}", style.list_item("-", error));
                }
            }
        }
    }

    if let Some(ref revision) = result.revision {
        println!(
            "{}",
            style.message_detail("Revision", &style.revision(revision.id.as_str()))
        );
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_release(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    tag: Option<String>,
    branch: Option<String>,
    from: Option<String>,
    to: Option<String>,
    append: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), GikError> {
    // Validate: append mode requires explicit tag
    if append && tag.is_none() {
        return Err(GikError::InvalidArgument(
            "Append mode requires explicit --tag (e.g., --tag v1.0.0)".to_string(),
        ));
    }

    let range = ReleaseRange {
        from: from.map(RevisionId::new),
        to: to.map(RevisionId::new),
    };

    let mode = if append {
        ReleaseMode::Append
    } else {
        ReleaseMode::Replace
    };

    let opts = ReleaseOptions {
        tag: tag.clone(),
        branch,
        range,
        dry_run,
        mode,
    };

    let result = engine.release(workspace, opts)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        // Human-readable output
        println!("{}", style.section("RELEASE"));
        println!();
        println!("  {}", style.key_value("Tag", &result.tag));
        println!("  {}", style.key_value("Branch", &result.summary.branch.to_string()));

        if dry_run {
            println!("  {}", style.key_value("Mode", "dry-run (no files written)"));
        } else {
            let mode_str = if append { "append" } else { "replace" };
            if let Some(path) = &result.changelog_path {
                println!("  {}", style.key_value("Output", &format!("{} ({})", path, mode_str)));
            }
        }

        println!();

        let total = result.summary.total_entries;
        if total == 0 {
            println!("{}", style.message(MessageType::Info, "No commit entries found in range."));
        } else {
            println!(
                "{}",
                style.message(MessageType::Ok, &format!("{} entries processed", total))
            );
            println!();

            for group in &result.summary.groups {
                println!("  ### {} ({} entries)", group.label, group.entries.len());
                for entry in &group.entries {
                    let scope_part = entry
                        .scope
                        .as_ref()
                        .map(|s| format!("**{}:** ", s))
                        .unwrap_or_default();
                    let breaking_part = if entry.breaking { "BREAKING: " } else { "" };
                    println!(
                        "  - {}{}{} ({})",
                        breaking_part, scope_part, entry.description, style.revision(entry.revision_id.as_str())
                    );
                }
                println!();
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_show(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    revision: String,
    branch: Option<String>,
    json: bool,
    kg_dot: bool,
    kg_mermaid: bool,
    max_sources: usize,
    max_kg_nodes: usize,
    max_kg_edges: usize,
) -> Result<(), GikError> {
    // Build show options
    
    // Build show options
    let mut opts = ShowOptions::new()
        .with_revision_ref(&revision)
        .with_max_sources(max_sources);

    if let Some(ref b) = branch {
        opts = opts.with_branch(b.clone());
    }

    // Run show
    let report = engine.show(workspace, opts)?;

    // Handle output formats
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else if kg_dot || kg_mermaid {
        // Export KG subgraph using engine method (encapsulates KG store access)
        let format = if kg_mermaid {
            KgExportFormat::Mermaid
        } else {
            KgExportFormat::Dot
        };

        let title = format!(
            "KG for revision {}",
            style.revision(&report.revision_id)
        );

        match engine.export_kg_subgraph(
            workspace,
            branch.as_deref(),
            format,
            max_kg_nodes,
            max_kg_edges,
            Some(title),
        )? {
            Some(output) => println!("{}", output),
            None => eprintln!(
                "{}",
                style.message(
                    MessageType::Warn,
                    &format!("No Knowledge Graph found for branch '{}'", report.branch)
                )
            ),
        }
    } else {
        // Human-readable output
        println!("{}", report.render_text());
    }

    Ok(())
}

// ============================================================================
// Config command handler
// ============================================================================

fn handle_config(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    action: ConfigAction,
) -> Result<(), GikError> {
    match action {
        ConfigAction::Check { json } => handle_config_check(style, engine, workspace, json),
        ConfigAction::Show { json } => handle_config_show(style, engine, workspace, json),
    }
}

/// Validate configuration files and report errors/warnings.
fn handle_config_check(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    json: bool,
) -> Result<(), GikError> {
    let validation = engine.validate_config(workspace)?;
    
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&validation).unwrap_or_default()
        );
    } else {
        // Display sources checked
        println!(
            "{}",
            style.message(
                MessageType::Info,
                &format!("Checked {} configuration source(s)", validation.sources.len())
            )
        );
        
        for source in &validation.sources {
            let status = if source.exists {
                if source.valid { "✓" } else { "✗" }
            } else {
                "-"
            };
            println!("  {} {} ({})", status, source.name, source.path.display());
        }
        println!();
        
        // Display warnings
        if !validation.warnings.is_empty() {
            println!(
                "{}",
                style.message(
                    MessageType::Warn,
                    &format!("{} warning(s):", validation.warnings.len())
                )
            );
            for warning in &validation.warnings {
                println!("  • {}", warning);
            }
            println!();
        }
        
        // Display errors
        if !validation.errors.is_empty() {
            println!(
                "{}",
                style.message(
                    MessageType::Err,
                    &format!("{} error(s):", validation.errors.len())
                )
            );
            for error in &validation.errors {
                println!("  • {}", error);
            }
            println!();
        }
        
        // Summary
        if validation.errors.is_empty() && validation.warnings.is_empty() {
            println!(
                "{}",
                style.message(MessageType::Ok, "Configuration is valid")
            );
        } else if validation.errors.is_empty() {
            println!(
                "{}",
                style.message(MessageType::Ok, "Configuration is valid with warnings")
            );
        } else {
            println!(
                "{}",
                style.message(MessageType::Err, "Configuration has errors")
            );
        }
    }
    
    // Return error if there are validation errors
    if !validation.errors.is_empty() {
        return Err(GikError::InvalidConfiguration {
            message: format!("{} configuration error(s) found", validation.errors.len()),
            hint: "Run `gik config validate` for details".to_string(),
        });
    }
    
    Ok(())
}

/// Show resolved configuration (merged from all sources).
fn handle_config_show(
    style: &Style,
    engine: &GikEngine,
    workspace: &gik_core::Workspace,
    json: bool,
) -> Result<(), GikError> {
    let resolved = engine.resolved_config(workspace)?;
    
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&resolved).unwrap_or_default()
        );
    } else {
        // Human-readable output using pretty JSON
        println!(
            "{}",
            style.message(MessageType::Info, "Resolved configuration:")
        );
        println!();
        
        // Display as pretty JSON (more universally readable than debug format)
        let pretty = serde_json::to_string_pretty(&resolved)
            .unwrap_or_else(|_| format!("{:?}", resolved));
        println!("{}", pretty);
    }
    
    Ok(())
}
