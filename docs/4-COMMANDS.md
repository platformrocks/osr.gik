# Guided Indexing Kernel (GIK) – Commands Reference

> Status: Draft v0.2

This document lists all **GIK CLI commands** and their options in a compact, copy‑friendly format.

---

## 1. Commands

| Syntax                             | Description                                                                 |
| ---------------------------------- | --------------------------------------------------------------------------- |
| `gik init`                         | Initialize GIK structures for the current workspace/branch. **Idempotent.** |
| `gik status [--json]`              | Show current GIK status (HEAD, staging, stack).                             |
| `gik bases`                        | List available knowledge bases for the current branch.                      |
| `gik add [TARGET ...] [--base NAME] [--memory TEXT]`| Stage sources and update stack (paths, URLs, archives).           |
| `gik rm <TARGET ...>`              | Remove sources from the staging area.                                       |
| `gik commit [-m MESSAGE]`          | Index staged sources/memory and create a new revision.                      |
| `gik log [OPTIONS]`                | Show knowledge timeline or ask log with filtering support.                  |
| `gik ask <QUERY> [OPTIONS]`        | Query knowledge (RAG/stack/memory/KG) and return context.                   |
| `gik stats [--base NAME] [--json]` | Show aggregated stats for all bases or a single base.                       |
| `gik reindex --base NAME [--force] [--dry-run] [--json]` | Rebuild embeddings and index for a specific base.     |
| `gik release [OPTIONS]`            | Generate `CHANGELOG.md` from commit history (Conventional Commits format).  |
| `gik show [REVISION] [OPTIONS]`  | Inspect a specific revision (like `git show`); supports KG export (DOT/Mermaid). |
| `gik config <check\|show> [--json]` | Validate or show the resolved GIK configuration.                            |

### 1.1 `gik init` Behavior

* Creates `.guided/knowledge/<branch>/` directory structure.
* Writes initial `Init` revision to `timeline.jsonl` and sets `HEAD`.
* Creates empty `staging.json`.
* **Idempotent:** Running `gik init` multiple times is safe:
  * First run: creates all structures, prints success message with revision ID.
  * Subsequent runs: detects existing `HEAD`, prints informational message, exits successfully.
  * No duplicate `Init` revisions are created.

### 1.2 `gik status` Behavior

* Reports the current state of the GIK workspace and branch.
* Output includes:
  * **Workspace root:** Absolute path to the workspace.
  * **Branch:** Current branch name and initialization status.
  * **HEAD:** If initialized, shows the current revision ID, operation type (e.g., `Init`, `Commit`), timestamp, and commit message.
  * **Staging:** Count of pending, indexed, and failed sources.
  * **Stack:** Total file count and number of detected languages.
  * **Bases:** Per-base statistics with health indicators:
    * Document count, vector count, file count
    * On-disk size (sum of core files)
    * Last commit timestamp
    * Embedding and index compatibility status
    * Health state (`Healthy`, `NeedsReindex`, `MissingModel`, `IndexMissing`, `Error`)
    * **Memory base:** Included alongside `code` and `docs` with same stats structure
* When `--json` is passed, outputs a complete `StatusReport` JSON object with camelCase keys, including the `bases` array.
* For uninitialized workspaces, HEAD/staging/stack/bases fields are `null`/`None`.

**Example Output (Human-Readable)**

```
GIK Status
  Workspace: /path/to/project
  Branch: main (initialized: yes)
  HEAD: abc12345 (Commit { bases: ["code", "docs"], source_count: 42 }) at 2025-11-27 18:10:00 UTC
        "Initial code and docs indexing"
  Staging: pending=0 indexed=0 failed=0
  Stack: files=42 languages=3

  Bases:
    BASE       DOCS   VECS FILES       SIZE         HEALTH
    code        100    100    30   512.0 KB             OK
      └─ last: 2025-11-27 18:10
    docs         12     12    12    64.0 KB             OK
      └─ last: 2025-11-27 18:10
    memory        5      5     5     8.0 KB             OK
      └─ last: 2025-11-27 18:15
```

**Example Output (JSON)**

```json
{
  "workspaceRoot": "/path/to/project",
  "branch": "main",
  "isInitialized": true,
  "head": {...},
  "staging": {...},
  "stack": {...},
  "stackSummary": {
    "files": 42,
    "languages": 3,
    "dependencies": 15
  },
  "stagedFiles": ["src/new_file.rs"],
  "modifiedFiles": ["src/engine.rs"],
  "workingTreeClean": false,
  "bases": [
    {
      "base": "code",
      "documents": 100,
      "vectors": 100,
      "files": 30,
      "onDiskBytes": 524288,
      "lastCommit": "2025-11-27T18:10:00Z",
      "embeddingStatus": "compatible",
      "indexStatus": "compatible",
      "health": "Healthy"
    },
    {
      "base": "memory",
      "documents": 5,
      "vectors": 5,
      "files": 5,
      "onDiskBytes": 8192,
      "lastCommit": "2025-11-27T18:15:00Z",
      "embeddingStatus": "compatible",
      "indexStatus": "compatible",
      "health": "Healthy"
    }
  ]
}
```

### 1.3 `gik add` Behavior

* Adds targets to the staging area as `PendingSource` entries in `staging/pending.jsonl`.
* Automatically infers source kind:
  * Local paths → `FilePath` or `Directory` (based on filesystem metadata)
  * URLs (`http://`, `https://`) → `Url`
  * Archives (`.zip`, `.tar`, `.tar.gz`, etc.) → `Archive`
* Automatically infers target knowledge base from source kind/extension:
  * URLs → `docs`
  * Directories → `code`
  * Files → based on extension (`.rs`→`code`, `.md`→`docs`, etc.)
* Use `--base` to explicitly override the inferred base.
* Skips sources that:
  * Don't exist (for local paths)
  * Are already pending for the same `(branch, base, uri)`
* After staging, triggers a full stack rescan to refresh inventory.
* **Memory entries are committed immediately** – when using `--memory "text"`, the entry is
  embedded and indexed right away, creating a new revision. No separate `gik commit` is needed.
* Output includes:
  * Number of sources staged
  * Number of sources skipped (with reasons)
  * Updated stack statistics

### 1.4 `gik rm` Behavior

* Removes targets from the staging area (`staging/sources.jsonl`).
* Accepts one or more target paths (files or directories).
* Only affects pending sources that have not yet been committed.
* Does not affect already-indexed content in knowledge bases.

**Usage**

```bash
gik rm src/old_file.rs            # Remove single file from staging
gik rm src/deprecated/ docs/old/  # Remove multiple paths from staging
```

**Output**

```
Removed 3 sources from staging:
  - src/old_file.rs
  - docs/old/README.md
  - docs/old/GUIDE.md
```

**Errors**

* **Not initialized:** Workspace must be initialized with `gik init`.
* **Source not found:** Warning if a specified path is not in staging (operation continues).

### 1.5 `gik commit` Behavior

* Processes all pending sources from `staging/pending.jsonl`.
* For each source:
  * Reads file content, creates embeddings, and upserts to the base's vector index.
  * Updates `sources.jsonl` and `stats.json` for each base.
* Creates a new `Commit` revision in `timeline.jsonl` and updates `HEAD`.
* Clears successfully indexed sources from staging.

**Limitations**

* **Single chunk per file:** Each file is treated as one chunk. No semantic splitting.
* **Large file limits:** Files >1MB or >10,000 lines are marked as `failed`.
* **URL/Archive not supported:** `Url` and `Archive` sources are marked as `failed`
  with reason `"URL sources not yet supported"` or
  `"Archive sources not yet supported"`.

**Errors**

* **Missing embedding model:** Commit fails if the default Candle model is not
  installed. Error message includes instructions to clone from Hugging Face:
  ```bash
  git clone https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 \
      models/embeddings/all-MiniLM-L6-v2
  ```
* **Nothing to commit:** Fails if no pending sources exist.
* **Model mismatch:** Fails if the active embedding model differs from the one
  used for existing base indexes (requires `gik reindex`).

### 1.6 `gik reindex` Behavior

* Rebuilds the vector index for a specific base using the current embedding model.
* Use when:
  * Changing embedding models (e.g., upgrading from `all-MiniLM-L6-v2` to a newer model).
  * Fixing corrupted or inconsistent indexes.
* Operation flow:
  1. Loads all sources from `bases/<base>/sources.jsonl`.
  2. For each source:
     * If `text` field is present, uses it directly.
     * If `text` is `None`, re-reads file from `file_path` (skips on read failure).
  3. Re-embeds all chunks with the current embedding backend.
  4. Rebuilds the vector index from scratch.
  5. Updates `model-info.json` with the new model info.
  6. Creates a `Reindex` revision in the timeline (unless `--dry-run`).

**Options**

* `--base NAME` (required): The knowledge base to reindex.
* `--force`: Reindex even if the embedding model hasn't changed.
* `--dry-run`: Report what would change without writing to disk or timeline.
* `--json`: Output results as JSON.

**Output (human-readable)**

```
Reindexed base 'code':
  Model: sentence-transformers/all-MiniLM-L6-v2 -> sentence-transformers/all-MiniLM-L6-v2
  Sources processed: 42
  Chunks re-embedded: 42
  Revision: rev-XXXXXXXX
```

**Output (JSON)**

```json
{
  "revision": { "id": "rev-XXXXXXXX", ... },
  "reembeddedChunks": 42,
  "bases": [
    {
      "base": "code",
      "fromModelId": "sentence-transformers/all-MiniLM-L6-v2",
      "toModelId": "sentence-transformers/all-MiniLM-L6-v2",
      "reindexed": true,
      "sourcesProcessed": 42,
      "chunksReembedded": 42,
      "errors": []
    }
  ],
  "dryRun": false
}
```

**Errors**

* **Base not found:** Fails if the specified base doesn't exist.
* **No sources:** Fails if the base has no indexed sources.
* **Missing embedding model:** Fails if the Candle model is unavailable.
* **Source read errors:** Individual sources that fail to read (when `text` is `None`)
  are skipped and recorded in the `errors` array. The operation continues with remaining sources.

### 1.7 `gik log` Behavior

* Queries GIK's knowledge log with filtering support.
* Two log kinds available:
  * `timeline` (default): Branch timeline entries (Init, Commit, Reindex, Release).
  * `ask`: Query history from the ask log.

**Per-Base Stats in Log**

For `Commit` and `Reindex` operations, the timeline entries are enriched with per-base statistics:
* `meta.baseStats`: Array of objects with `base`, `documents`, `vectors`, `files` for each affected base.
* In human-readable mode, bases are shown inline: `[code:12, docs:2]` (base:documents).
* In JSON mode, `meta.baseStats` is included in the full entry.

**Options**

* `--kind <KIND>`: Log kind to query (`timeline` or `ask`). Default: `timeline`.
* `--op <OP,...>`: Filter timeline by operation type(s) (comma-separated): `init`, `commit`, `reindex`, `release`.
* `--base <NAME,...>`: Filter by base name(s) (comma-separated).
* `--since <TIMESTAMP>`: Filter entries since this time (RFC 3339, e.g., `2024-01-15T10:00:00Z`).
* `--until <TIMESTAMP>`: Filter entries until this time (RFC 3339).
* `-n, --limit <N>`: Maximum number of entries to return.
* `--json`: Output as a single JSON array.
* `--jsonl`: Output as JSONL (one JSON object per line).

**Examples**

```bash
# Show timeline (default)
gik log

# Show only commits
gik log --op commit

# Show ask history
gik log --kind ask

# Show last 5 commits to the 'code' base
gik log --op commit --base code --limit 5

# Show timeline entries since a date
gik log --since 2024-01-01T00:00:00Z

# Output as JSON
gik log --json
```

**Output (human-readable, timeline)**

```
Timeline:
  abc12345 2024-01-15 10:00:00 commit [code:12, docs:2]
       "Indexed 14 sources"
  def45678 2024-01-14 09:00:00 init
       "Initialize GIK workspace"
```

**Output (human-readable, ask)**

```
Ask Log:
  2024-01-15 10:30:00 [code,docs] "How does the API work?" (8 hits)
  2024-01-15 10:25:00 [code] "What is the main entry..." (5 hits)
```

**Output (JSON with per-base stats)**

```json
[
  {
    "type": "timeline",
    "branch": "main",
    "timestamp": "2024-01-15T10:00:00Z",
    "operation": "commit",
    "revisionId": "abc12345-...",
    "bases": ["code", "docs"],
    "message": "Indexed 14 sources",
    "meta": {
      "sourceCount": 14,
      "baseStats": [
        {"base": "code", "documents": 12, "vectors": 12, "files": 12},
        {"base": "docs", "documents": 2, "vectors": 2, "files": 2}
      ]
    }
  }
]
```

### 1.8 `gik ask` Behavior

* Performs RAG retrieval across indexed knowledge bases (`code`, `docs`, `memory`).
* Embeds the question using the active Candle embedding backend.
* Runs vector similarity search across requested bases (defaults to all three).
* **Memory separation:** Results from the `memory` base populate `memoryEvents`, not `ragChunks`.
* Returns `AskContextBundle` containing:
  * `ragChunks`: Results from code/docs bases
  * `memoryEvents`: Results from memory base (preserves scope, source, tags)
  * `stackSummary`: Stack inventory summary (if enabled)

**Options**

* `QUESTION` (required): The natural language question to query.
* `--bases <LIST>`: Comma-separated list of bases to search (e.g., `code,docs,memory`). Default: all available.
* `--top-k <N>`: Maximum number of results per base. Default: 8.
* `--json`: Output as JSON.
* `--pretty`: Pretty-print JSON output.

**Output (human-readable)**

```
Query: "What architecture decisions have been made?"

RAG Chunks (3 results):
  [code] src/config.rs:10-30 (score: 0.85)
    "Configuration module for application settings..."

Memory Events (2 results):
  [decision] "Use PostgreSQL for primary datastore" (score: 0.92)
    Tags: database, architecture
    "We decided to use PostgreSQL because..."

Stack: 42 files, 3 languages
```

**Output (JSON)**

```json
{
  "question": "What architecture decisions have been made?",
  "ragChunks": [
    {
      "base": "code",
      "path": "src/config.rs",
      "content": "Configuration module...",
      "score": 0.85
    }
  ],
  "memoryEvents": [
    {
      "scope": "project",
      "source": "decision",
      "title": "Use PostgreSQL for primary datastore",
      "text": "We decided to use PostgreSQL because...",
      "tags": ["database", "architecture"],
      "score": 0.92
    }
  ],
  "stackSummary": {...},
  "revisionId": "rev-abc12345"
}
```

**Errors**

* **Not initialized:** Workspace must be initialized with `gik init`.
* **No indexed bases:** At least one base must have indexed content.
* **Embedding error:** Fails if unable to embed the query.

### 1.9 Notes on positional arguments

* `TARGET ...` (for `gik add`):

  * Zero or more targets; if omitted, defaults to `.` (current directory).
  * Can be paths (`src/`, `docs/`), single files, URLs, or archives.

* `QUESTION` (for `gik ask`):

  * Required free‑form string with the user's question.

### 1.10 `gik release` Behavior

* Generates `CHANGELOG.md` from commit history using Conventional Commits format.
* **Key design decisions:**
  * **Read-only:** Does NOT mutate the timeline (no `RevisionOperation::Release` appended).
  * **Full regeneration:** `CHANGELOG.md` is fully regenerated each time (no incremental merging).
  * **Conventional Commits:** Parses commit messages and groups by type (feat, fix, docs, etc.).
* Operation flow:
  1. Gathers all `Commit` revisions from the timeline in the specified range.
  2. Parses each revision's message using Conventional Commits format.
  3. Groups entries by type (feat, fix, chore, etc.) with appropriate sort order.
  4. Renders and writes `CHANGELOG.md` (unless `--dry-run`).

**Options**

* `--tag TAG`: Release tag used as heading (e.g., "v1.0.0"). Defaults to "Unreleased".
* `-b, --branch BRANCH`: Branch to generate changelog for. Defaults to current branch.
* `--from REV`: Starting revision (exclusive). Defaults to beginning of timeline.
* `--to REV`: Ending revision (inclusive). Defaults to HEAD.
* `--dry-run`: Report what would be written without actually writing.
* `--json`: Output results as JSON.

**Conventional Commits Parsing**

Messages are parsed using the format: `<type>[optional scope][!]: <description>`

Examples:
* `feat: add new feature` → Features
* `fix(cli): resolve parsing error` → Bug Fixes (scope: cli)
* `feat!: breaking change` → Features (BREAKING)
* `docs: update README` → Documentation

**Entry Grouping Order**

1. Features (`feat:`)
2. Bug Fixes (`fix:`)
3. Performance Improvements (`perf:`)
4. Code Refactoring (`refactor:`)
5. Documentation (`docs:`)
6. Styles (`style:`)
7. Tests (`test:`)
8. Build System (`build:`)
9. Continuous Integration (`ci:`)
10. Chores (`chore:`)
11. Reverts (`revert:`)
12. Other Changes (non-conventional messages)

**Output (human-readable)**

```
Release: v0.1.0
Branch:  main
Output:  /path/to/workspace/CHANGELOG.md

Entries: 3 total

### Features (2 entries)
  - **cli:** add release command (abc12345)
  - add new parser (def67890)

### Bug Fixes (1 entries)
  - BREAKING: fix API compatibility (fed98765)
```

**Output (JSON)**

```json
{
  "tag": "v0.1.0",
  "changelogPath": "/path/to/workspace/CHANGELOG.md",
  "summary": {
    "branch": "main",
    "fromRevision": null,
    "toRevision": null,
    "totalEntries": 3,
    "groups": [
      {
        "kind": "feat",
        "label": "Features",
        "entries": [...]
      }
    ],
    "dryRun": false
  }
}
```

**Generated CHANGELOG.md**

```markdown
# Changelog

All notable changes to this project will be documented in this file.

This changelog is auto-generated by GIK from commit history.

## v0.1.0

### Features

- **cli:** add release command (abc12345)
- add new parser (def67890)

### Bug Fixes

- **BREAKING:** fix API compatibility (fed98765)
```

---

### 1.11 `gik memory-metrics` Behavior (Engine Only)

> **Note:** This functionality is available via `GikEngine::memory_metrics()` but not yet exposed as a CLI command.

* Shows metrics for the memory knowledge base.
* Displays entry count, estimated token count, and configured pruning policy.

**Options**

* `-b, --branch BRANCH`: Branch to query metrics for. Defaults to current branch.
* `--json`: Output as JSON.

**Output (human-readable)**

```
Memory Metrics
  Branch: main

  Entry count:      42
  Estimated tokens: 8500

  Pruning Policy:
    Max entries: 1000
    Max tokens:  100000
    Max age:     365 days
    Mode: archive
```

**Output (JSON)**

```json
{
  "branch": "main",
  "metrics": {
    "entryCount": 42,
    "estimatedTokenCount": 8500,
    "totalChars": 34000
  },
  "pruningPolicy": {
    "maxEntries": 1000,
    "maxEstimatedTokens": 100000,
    "maxAgeDays": 365,
    "mode": "archive"
  }
}
```

### 1.12 `gik prune-memory` Behavior (Engine Only)

> **Note:** This functionality is available via `GikEngine::prune_memory()` but not yet exposed as a CLI command.

* Prunes memory entries based on a pruning policy.
* Can use the configured policy from `config.json` or explicit CLI options.
* Creates a `MemoryPrune` revision in the timeline (unless nothing was pruned).
* **Key design decisions:**
  * Two modes: `delete` (permanent) and `archive` (moves to `archive.jsonl`).
  * Archived entries are NOT searchable but preserved for audit.
  * Pruning is explicit only (no auto-pruning in other commands).

**Options**

* `--max-entries N`: Maximum number of entries to keep (oldest pruned first).
* `--max-tokens N`: Maximum estimated tokens to keep (oldest pruned first).
* `--max-age-days N`: Maximum age in days (older entries pruned).
* `--obsolete-tags TAGS`: Comma-separated tags that mark entries as obsolete.
* `--mode MODE`: Pruning mode (`delete` or `archive`). Default: `archive`.
* `-m, --message MSG`: Commit message for the pruning revision.
* `--json`: Output as JSON.

**Output (human-readable)**

```
Memory Pruning Complete

  Pruned:   15 entries
  Archived: 15 entries
  Mode:     archive

  Before: 57 entries, 11500 tokens
  After:  42 entries, 8500 tokens

  Revision: abc12345-...

  Reasons:
    - Exceeded max_entries threshold (42)
```

**Output (JSON)**

```json
{
  "revisionId": "abc12345-...",
  "result": {
    "prunedCount": 15,
    "archivedCount": 15,
    "deletedCount": 0,
    "prunedIds": ["mem-001", "mem-002", ...],
    "metricsBefore": {
      "entryCount": 57,
      "estimatedTokenCount": 11500
    },
    "metricsAfter": {
      "entryCount": 42,
      "estimatedTokenCount": 8500
    },
    "mode": "archive",
    "reasons": ["Exceeded max_entries threshold (42)"]
  }
}
```

**Errors**

* **No policy:** Fails if no pruning policy is configured and no options provided.
* **Not initialized:** Workspace must be initialized with `gik init`.

### 1.13 `gik show` Behavior

* Inspects a specific revision, similar to `git show`.
* Displays revision metadata, base impacts, and optional KG export.
* Supports revision references: `HEAD`, `HEAD~N`, full revision ID, or UUID prefix.

**Usage**

```bash
gik show                       # Show HEAD revision
gik show HEAD~3                # Show 3rd ancestor of HEAD
gik show abc12345              # Show revision by ID or prefix
gik show --branch feature-x    # Show HEAD of a different branch
gik show --json                # Output as JSON
gik show --kg-dot              # Include KG subgraph in DOT format
gik show --kg-mermaid          # Include KG subgraph in Mermaid format
```

**Options**

* `[REVISION]` (positional): Revision reference (defaults to `HEAD`).
* `-b, --branch BRANCH`: Branch to inspect (defaults to current branch).
* `--json`: Output as JSON.
* `--kg-dot`: Include KG subgraph in Graphviz DOT format.
* `--kg-mermaid`: Include KG subgraph in Mermaid format.
* `--max-sources N`: Maximum number of source paths to show per base (default: 20).
* `--max-kg-nodes N`: Maximum KG nodes to include (default: 50).
* `--max-kg-edges N`: Maximum KG edges to include (default: 100).

**Revision Reference Formats**

| Format | Example | Description |
|--------|---------|-------------|
| `HEAD` | `HEAD` | Current branch HEAD |
| `HEAD~N` | `HEAD~3` | Nth ancestor of HEAD |
| Full ID | `abc12345-def6-...` | Exact revision ID match |
| UUID prefix | `abc12` | Prefix match (must be unambiguous) |

**Output (human-readable)**

```
Revision: abc12345-def6-7890-1234-567890abcdef
Kind:     Commit
Branch:   main
Parent:   def67890-...
Git:      abcdef1234567890
Date:     2025-01-15 10:00:00 UTC

    feat(cli): add show command

Bases:
  code (5 sources):
    - src/cli.rs
    - src/show.rs
    - src/engine.rs
    - ... (2 more)

  docs (2 sources):
    - docs/4-API.md
    - docs/5-COMMANDS.md

KG Summary:
  Nodes: 7
  Edges: 5
```

**Output (JSON)**

```json
{
  "revisionId": "abc12345-...",
  "revisionKind": "Commit",
  "branch": "main",
  "author": null,
  "timestamp": "2025-01-15T10:00:00Z",
  "message": "feat(cli): add show command",
  "gitCommit": "abcdef1234567890",
  "parentId": "def67890-...",
  "bases": [
    {
      "name": "code",
      "sourceCount": 5,
      "sources": ["src/cli.rs", "src/show.rs", "..."],
      "truncated": true
    }
  ],
  "kgImpact": {
    "nodeCount": 7,
    "edgeCount": 5,
    "nodes": [...],
    "edges": [...],
    "nodesTruncated": false,
    "edgesTruncated": false
  },
  "operationSummary": "5 sources indexed, 42 chunks created",
  "truncated": false
}
```

**Output (with --kg-dot)**

```
Revision: abc12345-...
...

KG Export (DOT):
digraph kg {
  rankdir=LR;
  node [shape=box];
  n0 [label="src/cli.rs"];
  n1 [label="src/show.rs"];
  n0 -> n1 [label="imports"];
}
```

**Output (with --kg-mermaid)**

```
Revision: abc12345-...
...

KG Export (Mermaid):
graph LR
  n0["src/cli.rs"]
  n1["src/show.rs"]
  n0 -->|imports| n1
```

**Errors**

* **Not initialized:** Workspace must be initialized with `gik init`.
* **Revision not found:** Returns error if revision reference doesn't match any revision.
* **Ambiguous prefix:** Returns error if UUID prefix matches multiple revisions.
* **HEAD not set:** Returns error if `HEAD` is requested but no revisions exist.

---

### 1.14 `gik config` Behavior

* Manages GIK configuration.
* Two subcommands: `check` and `show`.

**Usage**

```bash
gik config check               # Validate configuration
gik config show                # Show resolved configuration
gik config check --json        # Output validation as JSON
gik config show --json         # Output configuration as JSON
```

**Subcommands**

* `check`: Validates the current configuration file. Reports errors if configuration is invalid.
* `show`: Displays the fully resolved configuration (merged from defaults, config file, and environment variables).

**Options**

* `--json`: Output as JSON.

**Output (`gik config check`, human-readable)**

```
Configuration: Valid
  Source: /home/user/project/gik.yaml
```

**Output (`gik config check`, JSON)**

```json
{
  "valid": true,
  "source": "/home/user/project/gik.yaml",
  "errors": []
}
```

**Output (`gik config show`, human-readable)**

```
Configuration (resolved from /home/user/project/gik.yaml):

  Device: auto
  Embedding Model: sentence-transformers/all-MiniLM-L6-v2
  Reranker Model: cross-encoder/ms-marco-MiniLM-L6-v2

  Performance:
    Batch Size: 32
    Parallel Reads: 4

  Memory:
    Max Entries: 1000
    Max Tokens: 100000
```

**Output (`gik config show`, JSON)**

```json
{
  "source": "/home/user/project/gik.yaml",
  "config": {
    "device": "auto",
    "embeddingModel": "sentence-transformers/all-MiniLM-L6-v2",
    "rerankerModel": "cross-encoder/ms-marco-MiniLM-L6-v2",
    "performance": {
      "embeddingBatchSize": 32,
      "parallelReads": 4
    },
    "memory": {
      "maxEntries": 1000,
      "maxEstimatedTokens": 100000
    }
  }
}
```

**Errors**

* **Config parse error:** Reports specific YAML parsing errors with line numbers.
* **Invalid value:** Reports invalid configuration values with field paths.

---

## 2. Options

| Option / Flag       | Applies to                                      | Type / Values                           |      Required | Default                         | Description                                                         |
| ------------------- | ----------------------------------------------- | --------------------------------------- | ------------: | ------------------------------- | ------------------------------------------------------------------- |
| `-h`, `--help`      | global + all commands                           | n/a                                     |            No | n/a                             | Show help for `gik` or a specific subcommand.                       |
| `-V`, `--version`   | global                                          | n/a                                     |            No | n/a                             | Print version information and exit.                                 |
| `-m`, `--message`   | `gik commit`                                    | string                                  |            No | auto‑generated                  | Commit message for the knowledge revision.                          |
| `--bases <LIST>`    | `gik ask`                                       | comma‑separated list (e.g. `code,docs`) |            No | auto‑detected                   | Restrict RAG search to specific bases.                              |
| `--files <PATTERN>` | `gik ask` (future)                              | string (glob/regex, TBD)                |            No | none                            | Additional filter to limit results to matching files.               |
| `--top-k <N>`       | `gik ask`                                       | integer                                 |            No | implementation default (e.g. 8) | Maximum number of chunks per base to return.                        |
| `--base <NAME>`     | `gik add`, `gik stats`, `gik reindex`           | string (e.g. `code`, `docs`)            | For `reindex` | for `add`: inferred; for `stats`: all bases | Target knowledge base. For `add`, overrides inferred base.          |
| `--force`           | `gik reindex`                                   | boolean flag                            |            No | off                             | Force reindex even if embedding model hasn't changed.               |
| `--dry-run`         | `gik reindex`, `gik release`                    | boolean flag                            |            No | off                             | Report what would change without writing to disk or timeline.       |
| `--tag <TAG>`       | `gik release`                                   | string (e.g. `v0.1.0`)                  |            No | `"Unreleased"`                  | Release tag used as heading in CHANGELOG.md.                        |
| `-b`, `--branch`    | `gik release`                                   | string                                  |            No | current branch                  | Branch to generate changelog for.                                   |
| `--from <REV>`      | `gik release`                                   | string (revision ID prefix)             |            No | none (from beginning)           | Starting revision (exclusive) for changelog range.                  |
| `--to <REV>`        | `gik release`                                   | string (revision ID prefix)             |            No | none (to HEAD)                  | Ending revision (inclusive) for changelog range.                    |
| `--kind <KIND>`     | `gik log`                                       | `timeline` or `ask`                     |            No | `timeline`                      | Log kind to query.                                                  |
| `--op <OP,...>`     | `gik log`                                       | comma-separated (e.g. `commit,reindex`) |            No | all operations                  | Filter timeline by operation type(s).                               |
| `--since <TS>`      | `gik log`                                       | RFC 3339 timestamp                      |            No | none                            | Filter entries since this timestamp.                                |
| `--until <TS>`      | `gik log`                                       | RFC 3339 timestamp                      |            No | none                            | Filter entries until this timestamp.                                |
| `-n`, `--limit <N>` | `gik log`                                       | integer                                 |            No | none                            | Maximum number of entries to return.                                |
| `--json`            | `gik status`, `gik stats`, `gik ask`, `gik log`, `gik reindex`, `gik release`, `gik show`, `gik config` | boolean flag        |            No | off                             | Output as a single JSON object instead of human‑readable text.      |
| `--jsonl`           | `gik log` (and possibly `ask`)                  | boolean flag                            |            No | off                             | Output as JSONL (one JSON per line) for easier machine consumption. |
| `--pretty`          | `gik ask`                                       | boolean flag                            |            No | off                             | Pretty‑print the `AskContextBundle` instead of raw JSON.            |
| `--max-entries <N>` | `gik prune-memory`                              | integer                                 |            No | from config                     | Maximum number of memory entries to keep.                           |
| `--max-tokens <N>`  | `gik prune-memory`                              | integer                                 |            No | from config                     | Maximum estimated tokens to keep in memory base.                    |
| `--max-age-days <N>`| `gik prune-memory`                              | integer                                 |            No | from config                     | Maximum age in days for memory entries.                             |
| `--obsolete-tags`   | `gik prune-memory`                              | comma-separated list                    |            No | from config                     | Tags that mark memory entries as obsolete.                          |
| `--mode <MODE>`     | `gik prune-memory`                              | `delete` or `archive`                   |            No | `archive`                       | Pruning mode (delete permanently or archive for audit).             |
| `--kg-dot`          | `gik show`                                      | boolean flag                            |            No | off                             | Include KG subgraph in Graphviz DOT format.                         |
| `--kg-mermaid`      | `gik show`                                      | boolean flag                            |            No | off                             | Include KG subgraph in Mermaid format.                              |
| `--max-sources <N>` | `gik show`                                      | integer                                 |            No | 20                              | Maximum number of source paths to show per base.                    |
| `--max-kg-nodes <N>`| `gik show`                                      | integer                                 |            No | 50                              | Maximum KG nodes to include in output.                              |
| `--max-kg-edges <N>`| `gik show`                                      | integer                                 |            No | 100                             | Maximum KG edges to include in output.                              |

### 2.1 Option semantics

* Options follow standard CLI conventions: flags can come **before or after** the subcommand arguments as long as the parser supports it.
* When both `--json` and `--pretty` are present on `gik ask`, `--pretty` should format the JSON output rather than switching to a custom human‑only view.

---

End of `5-COMMANDS.md` v0.2.
