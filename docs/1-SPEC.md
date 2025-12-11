# Guided Indexing Kernel (GIK) – Technical Specification (with ASCII Diagrams)

> Version: v0.3
> Updated: 2025-12-09
> Owner: Gui Santos
> Scope: GIK workspace (core library + CLI)

---

## 1. Overview

GIK (Guided Indexing Kernel) is a **Rust-based, local-first knowledge engine** that provides:

* Project-level **RAG** (code + docs chunks + embeddings).
* **Stack / Inventory** of the project structure and dependencies.
* **Memory** of events and decisions.
* **Knowledge Graph (KG)** layer with auto-sync.

It exposes:

* A **Rust library API** (`gik-core`) used by other tools (Guided CLI, IDE plugins, agents).
* A **standalone CLI** (`gik`) that developers can run directly in a workspace.

GIK does **not** call LLMs. It focuses on indexing, querying, and tracking the evolution of knowledge.

### 1.1 High-level Architecture (ASCII)

```text
           +-----------------------------+
           |        External Tools       |
           |  (Guided CLI, IDE, Agents)  |
           +--------------+--------------+
                          |
                          | Rust API calls (library)
                          v
                    +-----------+
                    | gik-core  |
                    | (engine)  |
                    +-----------+
                          ^
                          | internal calls
                          |
+-----------+      +------+-------+       +-------------------------+
|   User    | ---> |   gik-cli   | ----> | Filesystem & Local DBs  |
+-----------+  CLI | (binary)    |       | (.guided, indices, etc.)|
                   +-------------+       +-------------------------+
```

---

## 2. Technology Stack

### 2.1 Language and Tooling

* Language: **Rust** (edition 2021 or 2024).
* MSRV: to be defined (e.g., `1.75+`), explicit in `Cargo.toml`.
* Build: `cargo` workspace.
* Testing: `cargo test` (unit + integration).
* Formatting: `rustfmt` + `clippy`.

### 2.2 Crate Layout

Workspace layout:

```text
gik/
  Cargo.toml                # workspace
  crates/
    gik-core/               # domain, engine, orchestration, pipelines
    gik-cli/                # CLI binary `gik`
    gik-db/                 # LanceDB vectors, KG persistence
    gik-model/              # Candle embeddings, reranking
    gik-utils/              # URL fetching, HTML parsing
  .github/ (optional)       # CI
```

ASCII view of crate responsibilities:

```text
gik-cli (thin UX layer: CLI parsing, user output)
    ↓ calls GikEngine
gik-core (domain logic, orchestration, pipelines)
    ↓ adapters (db_adapter.rs, model_adapter.rs)
gik-db (LanceDB vectors, KG persistence)    gik-model (Candle embeddings/reranking)
    ↓                                            ↓
gik-utils (URL fetching, HTML parsing)
```

### 2.3 Crate Responsibilities

| Crate | Owns | Forbidden Dependencies |
|-------|------|------------------------|
| `gik-core` | Domain types, `GikEngine`, pipelines (ask, commit, reindex, release) | `lancedb`, `candle-*`, `arrow-*` |
| `gik-cli` | CLI parsing (clap), `Style` helpers, no business logic | Direct storage access |
| `gik-db` | Vector index (LanceDB), KG store, Arrow schema wrappers | ML inference |
| `gik-model` | Embedding models (Candle), reranker, model locator | Storage concerns |
| `gik-utils` | URL fetching, HTML parsing utilities | None specific |

**Key principle**: Heavy dependencies (LanceDB, Candle, Arrow) are isolated in leaf crates. Core never imports them directly—uses adapter traits (`IntoGikResult`, `db_adapter.rs`, `model_adapter.rs`).

### 2.4 Dependencies (by Crate)

**gik-core** (domain logic, no heavy deps)

* `anyhow` / `thiserror` – error handling.
* `serde`, `serde_json`, `serde_yaml` – config & JSONL.
* `chrono` – timestamps.
* `uuid` – IDs for revisions, events, etc.
* `walkdir` – directory traversal.
* `ignore` – gitignore-like file filtering.
* `globset` – pattern matching.
* `rayon` – parallel processing.
* `tracing` – structured logging.

**gik-db** (storage layer)

* `lancedb` – vector index backend (default).
* `arrow-*` – Arrow schema for LanceDB.
* `tokio` – async runtime for LanceDB.

**gik-model** (ML inference)

* `candle-core`, `candle-nn`, `candle-transformers` – local embeddings.
* `tokenizers` – tokenization for transformer models.
* `reqwest` (optional) – HTTP client for `Ollama` provider.

**gik-cli** (binary)

* `clap` – command-line parsing.
* `nu-ansi-term` – terminal styling.

### 2.5 Feature Flags

| Feature | Crate | Description |
|---------|-------|-------------|
| `metal` | `gik-model` | macOS GPU acceleration via Metal |
| `cuda` | `gik-model` | NVIDIA GPU acceleration via CUDA |

---

## 3. On-disk Layout

### 3.1 Global Configuration

```text
~/.gik/
  config.yaml
  models/
    embeddings/
      all-MiniLM-L6-v2/
        ...              # model files (e.g. safetensors, gguf, etc.)
```

Example `~/.gik/config.yaml`:

```yaml
embedding:
  defaultProfile: "minilm-l6-v2"
  profiles:
    minilm-l6-v2:
      type: "candle-sbert"
      modelId: "sentence-transformers/all-MiniLM-L6-v2"
      dim: 384
      path: "/home/user/.gik/models/embeddings/all-MiniLM-L6-v2"
```

### 3.2 Project Layout

```text
<workspace>/
  .git/ (optional)
  .gikignore       # optional
  .guided/
    knowledge/
      config.yaml
      <branch>/           # e.g. main, default, feature-x
        HEAD
        timeline.jsonl
        staging/
          pending.jsonl
          summary.json
        stack/
          files.jsonl
          dependencies.jsonl
          tech.jsonl
          stats.json
        bases/              # ← all knowledge bases under this subdirectory
          code/
            sources.jsonl
            stats.json
            meta.json       # VectorIndexMeta (includes embedding model info)
            vectors/        # LanceDB vector storage
          docs/
            sources.jsonl
            stats.json
            meta.json
            vectors/
          memory/
            events.jsonl
            stats.json
            meta.json (if vector index is used)
            vectors/
        kg/                 # Knowledge graph (fully implemented)
          nodes.jsonl
          edges.jsonl
          stats.json
```

ASCII tree emphasizing branch and base structure:

```text
.guided/knowledge/
  config.yaml
  <branch>/
    HEAD
    timeline.jsonl
    +-- staging/
    +-- stack/
    +-- bases/
    |     +-- code/
    |     +-- docs/
    |     +-- memory/
    +-- kg/
```

### 3.3 Key Files

* `~/.gik/config.yaml` – global config (embedding settings).
* `.guided/knowledge/config.yaml` – project config (`embedding`, etc.).
* `.guided/knowledge/HEAD` – GIK-specific branch override (optional).
* `timeline.jsonl` – revision history (one JSON per line).
* `staging/pending.jsonl` – pending sources queued for indexing.
* `staging/summary.json` – aggregate statistics about staging.
* `HEAD` – pointer to current `revisionId`.
* `stack/*.jsonl` – project inventory (files, dependencies, technologies).
* `bases/<base>/sources.jsonl` – metadata of indexed chunks.
* `bases/<base>/meta.json` – VectorIndexMeta (backend, metric, dimension, embedding info).
* `bases/<base>/stats.json` – aggregated stats.
* `bases/<base>/vectors/` – LanceDB vector storage.
* `kg/nodes.jsonl` – KG entity nodes.
* `kg/edges.jsonl` – KG relationship edges.

### 3.4 Model Search Paths

GIK searches for embedding models in the following order:

1. `$GIK_MODELS_DIR` environment variable
2. `~/.gik/models` user directory
3. `{exe_dir}/models` next to the binary

### 3.4 Workspace Detection

GIK determines the **workspace root** by walking up the directory tree from the
current directory until it finds one of these markers:

1. `.guided/` directory (GIK workspace marker) – takes priority
2. `.git/` directory (Git repository root)

If neither marker is found, the current directory is used as the workspace root,
allowing `gik init` to work in any directory.

```text
Workspace Resolution Algorithm:

  start_dir = current directory
  
  while current_dir is not filesystem root:
    if .guided/ exists in current_dir:
      return current_dir as workspace root
    if .git/ exists in current_dir:
      return current_dir as workspace root
    current_dir = parent of current_dir
  
  return start_dir as workspace root (for `gik init`)
```

The `.guided/` marker takes precedence over `.git/` to support nested Git
repositories where an inner project has its own GIK workspace.

### 3.5 Branch Resolution

GIK uses **branch names** to organize knowledge per development branch. Branch
names map directly to directory names under `.guided/knowledge/<branch>/`.

**Branch Detection Priority:**

1. **GIK HEAD override** (`.guided/knowledge/HEAD`): If this file exists and
   contains a non-empty branch name, it is used. This allows users to override
   the Git branch for GIK operations.

2. **Git HEAD** (`.git/HEAD`): If the workspace is a Git repository:
   - If HEAD is a symbolic ref (`ref: refs/heads/<branch>`), the branch name is extracted.
   - If HEAD is a commit hash (detached HEAD state), GIK uses `"HEAD"` as the branch name.

3. **Default branch** (`"main"`): If no Git repository exists and no GIK override
   is set, the default branch is `"main"`.

```text
Branch Detection Algorithm:

  if .guided/knowledge/HEAD exists and is non-empty:
    return contents as branch name
  
  if .git/HEAD exists:
    content = read .git/HEAD
    if content starts with "ref: refs/heads/":
      return branch name from ref
    if content is 40-char hex (commit hash):
      return "HEAD" (detached state)
  
  return "main" (default)
```

**Branch Name Validation:**

Valid branch names must:
- Be non-empty
- Contain only alphanumeric characters, hyphens (`-`), underscores (`_`), periods (`.`), and forward slashes (`/`)
- Not start or end with a slash
- Not contain consecutive slashes (`//`)

Examples of valid branch names:
- `main`, `develop`, `feature/my-feature`, `release/v1.0`, `user/john/experiment`

Invalid branch names:
- `` (empty)
- `my feature` (contains space)
- `/leading` (starts with slash)
- `trailing/` (ends with slash)
- `double//slash` (consecutive slashes)

---

## 4. Core Types (gik-core)

### 4.1 Identifiers

```rust
type BranchName = String;     // e.g. "main", "feature-x", "default"
type BaseName = String;       // e.g. "code", "docs", "stack", "memory"
type RevisionId = String;     // UUIDv4 or hash
```

### 4.2 Config Types

```rust
pub struct GikConfig {
    pub embedding: EmbeddingConfig,
    pub device: Option<DevicePreference>,  // auto, gpu, cpu
    pub performance: Option<PerformanceConfig>,
}

pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,  // candle, ollama
    pub model_id: String,
    pub dim: usize,
    pub path: Option<PathBuf>,        // candle: model directory
    pub host: Option<String>,         // ollama: server host
}

pub enum EmbeddingProvider {
    Candle,
    Ollama,
}

pub enum DevicePreference {
    Auto,
    Gpu,
    Cpu,
}

pub struct PerformanceConfig {
    pub embedding_batch_size: usize,  // default: 32
}

pub struct ProjectConfig {
    pub embedding: Option<EmbeddingConfig>,
}
```

### 4.3 Revision and Timeline

```rust
#[derive(Serialize, Deserialize)]
pub struct Revision {
    pub id: RevisionId,
    pub parent_id: Option<RevisionId>,
    pub branch: BranchName,
    pub git_commit: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub message: String,
    pub operations: Vec<RevisionOperation>,
}

#[derive(Serialize, Deserialize)]
pub enum RevisionOperation {
    Init,
    Commit { bases: Vec<BaseName>, source_count: usize },
    Reindex { base: BaseName, from_model_id: String, to_model_id: String },
    Release { tag: Option<String> },
    MemoryIngest { count: usize },
    MemoryPrune { pruned: usize, archived: usize },
}
```

### 4.4 Staging Types

```rust
/// Unique identifier for a pending source (UUIDv4).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PendingSourceId(pub String);

/// Classification of pending source input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingSourceKind {
    FilePath,
    Directory,
    Url,
    Archive,
    Other,
}

/// Processing state for a pending source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingSourceStatus {
    #[default]
    Pending,
    Processing,
    Indexed,
    Failed,
}

/// A staged source awaiting commit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingSource {
    pub id: PendingSourceId,
    pub branch: BranchName,
    pub base: BaseName,
    pub kind: PendingSourceKind,
    pub uri: String,
    pub added_at: chrono::DateTime<chrono::Utc>,
    pub status: PendingSourceStatus,
    pub last_error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Aggregate staging statistics.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StagingSummary {
    pub pending_count: usize,
    pub indexed_count: usize,
    pub failed_count: usize,
    pub by_base: HashMap<BaseName, usize>,
    pub last_updated_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### 4.5 Stack Types

```rust

#[derive(Serialize, Deserialize)]
pub struct StackFileEntry {
    pub path: String,
    pub kind: StackFileKind, // Dir/File
    pub languages: Vec<String>,
    pub file_count: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub enum StackFileKind { Dir, File }

#[derive(Serialize, Deserialize)]
pub struct StackDependencyEntry {
    pub manager: String,      // cargo, npm, etc.
    pub name: String,
    pub version: String,
    pub scope: String,        // runtime, dev, build
    pub manifest_path: String,
}

#[derive(Serialize, Deserialize)]
pub struct StackTechEntry {
    pub kind: String,         // framework, language, infra
    pub name: String,
    pub source: String,
    pub confidence: f32,
}
```

### 4.6 Embeddings

```rust
pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    fn embed_texts(&self, inputs: &[String]) -> anyhow::Result<Vec<Vec<f32>>>;
}

pub struct CandleSbertProvider {
    // config + model handles
}

pub struct OllamaEmbeddingProvider {
    // host, model, dim
}
```

### 4.7 Vector Index Layer

The **Vector Index Layer** provides a clean abstraction for vector storage and similarity search,
decoupled from the embedding provider. This separation allows:

* Different backends (SimpleFile, LanceDB) per workspace or per base.
* Consistent compatibility checks across embedding and index layers.
* Future migration between backends without changing the engine API.

**Architecture**

```text
┌───────────────────────────────────────────────────────────────────────────┐
│                           GIK Engine                                      │
├───────────────────────────────────────────────────────────────────────────┤
│                                                                           │
│   ┌─────────────────┐         ┌─────────────────────────────────────┐     │
│   │ EmbeddingConfig │         │ VectorIndexConfig                   │     │
│   │ (provider+model)│         │ (backend+metric+dimension)          │     │
│   └────────┬────────┘         └──────────────┬──────────────────────┘     │
│            │                                  │                           │
│            │  generates embeddings            │  stores/queries vectors   │
│            ▼                                  ▼                           │
│   ┌─────────────────┐         ┌─────────────────────────────────────┐     │
│   │EmbeddingBackend │         │ VectorIndexBackend (trait)          │     │
│   │ trait (Phase 4.1)│         │  ├── upsert(vectors)                │     │
│   │ embed_texts()   │         │  ├── query(embedding, top_k)        │     │
│   └─────────────────┘         │  ├── delete(ids)                    │     │
│                               │  ├── flush()                        │     │
│                               │  └── stats()                        │     │
│                               └──────────────┬──────────────────────┘     │
│                                              │                           │
│                                              │ implementations           │
│                                              ▼                           │
│                               ┌─────────────────────────────────────┐     │
│                               │ LanceDbVectorIndex (default)        │     │
│                               │  - disk-based ANN via LanceDB       │     │
│                               │  - hybrid search (BM25 + vector)    │     │
│                               │  - RRF fusion for result ranking    │     │
│                               │  - suitable for all index sizes     │     │
│                               ├─────────────────────────────────────┤     │
│                               │ SimpleFileVectorIndex (deprecated)  │     │
│                               │  - records.jsonl (JSONL storage)    │     │
│                               │  - linear scan search               │     │
│                               │  - legacy only, not recommended     │     │
│                               └─────────────────────────────────────┘     │
│                                                                           │
└───────────────────────────────────────────────────────────────────────────┘
```

**Per-Base Index Storage**

Each knowledge base (`code`, `docs`, `memory`) has its own vector index stored under:

```text
.guided/knowledge/<branch>/bases/<base>/
├── sources.jsonl   # Indexed chunk metadata
├── stats.json      # Base statistics
├── meta.json       # VectorIndexMeta (backend, metric, dimension, embedding info)
└── vectors/        # LanceDB vector storage directory
```

**Compatibility Flow**

Before using an index, GIK performs a two-stage compatibility check:

1. **Embedding Model Check**: Compare active `EmbeddingConfig` with stored `ModelInfo`.
   - If model changed → embeddings are stale → reindex required.

2. **Vector Index Check**: Compare active `VectorIndexConfig` with stored `VectorIndexMeta`.
   - Dimension mismatch → reindex required.
   - Backend mismatch → warning (can proceed, but may require migration).
   - Metric mismatch → warning (can proceed, but search results may differ).

```rust
pub enum VectorIndexBackendKind {
    LanceDb,     // default, disk-based ANN with hybrid search
    SimpleFile,  // deprecated, linear scan, JSONL storage
    Other(String),
}

pub enum VectorMetric {
    Cosine,  // default
    Dot,
    L2,
}

pub struct VectorIndexConfig {
    pub backend: VectorIndexBackendKind,
    pub metric: VectorMetric,
    pub dimension: usize,
    pub base: String,
}

pub struct VectorIndexMeta {
    pub backend: VectorIndexBackendKind,
    pub metric: VectorMetric,
    pub dimension: usize,
    pub base: String,
    pub embedding_provider: EmbeddingProviderKind,
    pub embedding_model_id: EmbeddingModelId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub trait VectorIndexBackend: Send + Sync {
    fn upsert(&mut self, vectors: Vec<VectorInsert>) -> Result<(), GikError>;
    fn query(&self, embedding: &[f32], top_k: usize) -> Result<Vec<VectorSearchResult>, GikError>;
    fn delete(&mut self, ids: &[VectorId]) -> Result<(), GikError>;
    fn flush(&mut self) -> Result<(), GikError>;
    fn stats(&self) -> Result<VectorIndexStats, GikError>;
}
```

### 4.8 Ask Context Bundle

```rust
#[derive(Serialize, Deserialize)]
pub struct AskContextBundle {
    pub revision_id: RevisionId,
    pub question: String,
    pub bases: Vec<BaseName>,
    pub rag_chunks: Vec<RagChunk>,
    pub kg_results: Vec<KgResult>,
    pub memory_events: Vec<MemoryEvent>,
    pub stack_summary: Option<StackSummary>,
    pub debug: AskDebugInfo,
}

pub struct RagChunk {
    pub base: BaseName,
    pub score: f32,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub snippet: String,
}

pub struct KgResult {
    pub node_id: String,
    pub node_type: String,  // function, file, module, endpoint, etc.
    pub label: String,
    pub edges: Vec<KgEdge>,
}

pub struct KgEdge {
    pub relation: String,   // calls, defines, imports, etc.
    pub target_id: String,
    pub target_label: String,
}

pub struct MemoryEvent {
    pub id: String,
    pub scope: String,      // project, global
    pub message: String,
    pub timestamp: DateTime<Utc>,
    pub tags: Vec<String>,
    pub token_count: Option<usize>,
}

pub struct StackSummary {
    // aggregated info useful for LLM context (languages, frameworks, etc.)
}

pub struct AskDebugInfo {
    pub embedding_model_id: String,
    pub used_bases: Vec<BaseName>,
}
```

### 4.9 Engine Facade

```rust
pub struct GikEngine {
    // references to config, embedding provider, vector index, etc.
}

impl GikEngine {
    // Core operations
    pub fn init_workspace(&self, workspace: &Path) -> anyhow::Result<()> { /* ... */ }
    pub fn add_targets(&self, workspace: &Path, targets: &[String]) -> anyhow::Result<()> { /* ... */ }
    pub fn remove_targets(&self, workspace: &Path, targets: &[String]) -> anyhow::Result<()> { /* ... */ }
    pub fn commit(&self, workspace: &Path, message: Option<String>) -> anyhow::Result<()> { /* ... */ }
    pub fn ask(&self, workspace: &Path, question: &str, opts: AskOptions) -> anyhow::Result<AskContextBundle> { /* ... */ }
    pub fn reindex(&self, workspace: &Path, base: &BaseName) -> anyhow::Result<()> { /* ... */ }
    pub fn status(&self, workspace: &Path) -> anyhow::Result<StatusReport> { /* ... */ }
    pub fn release(&self, workspace: &Path, tag: Option<String>) -> anyhow::Result<()> { /* ... */ }
    
    // Show/inspect operations
    pub fn show_timeline(&self, workspace: &Path, opts: ShowOptions) -> anyhow::Result<Vec<Revision>> { /* ... */ }
    pub fn show_revision(&self, workspace: &Path, revision_id: &str) -> anyhow::Result<Revision> { /* ... */ }
    
    // Memory operations
    pub fn memory_metrics(&self, workspace: &Path) -> anyhow::Result<MemoryMetrics> { /* ... */ }
    pub fn memory_prune(&self, workspace: &Path, opts: PruneOptions) -> anyhow::Result<PruneResult> { /* ... */ }
    
    // KG operations
    pub fn kg_export(&self, workspace: &Path, format: KgExportFormat) -> anyhow::Result<String> { /* ... */ }
}
```

---

## 5. CLI Architecture (gik-cli)

### 5.1 Command Parsing

`gik-cli` uses `clap` to map CLI commands to `GikEngine` calls.

#### Global Flags

| Flag | Description |
|------|-------------|
| `-v, --verbose` | Enable debug logging |
| `-q, --quiet` | Suppress progress messages |
| `-c, --config <PATH>` | Custom config file |
| `--device <auto\|gpu\|cpu>` | Device preference |
| `--color <auto\|always\|never>` | Color output mode |

#### Main Subcommands

| Command | Description | Key Flags |
|---------|-------------|-----------|
| `gik init` | Initialize GIK workspace | — |
| `gik status` | Show workspace status | `--json` |
| `gik bases` | List knowledge bases | — |
| `gik add <PATH>` | Stage sources for indexing | `-b, --base`, `--url`, `-t, --type`, `--metadata` |
| `gik rm <PATH>` | Remove from staging | — |
| `gik commit` | Index staged sources | `-m, --message` |
| `gik show` | Show timeline/ask history | `--timeline`, `--history`, `--last <N>`, `--since`, `--until`, `--tag`, `-b, --base`, `--json` |
| `gik ask <QUERY>` | Query knowledge (RAG) | `-b, --base`, `--top-k`, `--memory`, `--url`, `--json` |
| `gik stats` | Show base statistics | `-b, --base`, `--json` |
| `gik reindex` | Rebuild embeddings | `-b, --base`, `--force`, `--batch-size` |
| `gik release` | Generate CHANGELOG | `--from`, `--to`, `-o, --output`, `--format`, `--tag`, `--push` |
| `gik inspect <REV>` | Inspect revision | `--chunks`, `--sources`, `--stats`, `--json` |
| `gik memory-metrics` | Show memory statistics | `--json` |
| `gik memory-prune` | Prune memory entries | `--older-than`, `--max-tokens`, `--archive` |
| `gik config validate` | Validate configuration | `--json` |
| `gik config show` | Show resolved config | `--json` |

### 5.2 Shared CLI Flow (ASCII)

```text
+---------+      +----------+      +-----------+      +----------------------+
|  User   | ---> |  gik CLI | ---> | GikEngine | ---> | FS & Local Indexes   |
+---------+      +----------+      +-----------+      +----------------------+
                    ^                  ^
                    |                  |
              Global/Project      EmbeddingProvider
                Config           + VectorIndex
```

For every command, the CLI:

1. Resolves workspace.
2. Loads global and project config.
3. Detects branch.
4. Instantiates `GikEngine`.
5. Calls the appropriate method.

---

## 6. Command Workflows (with ASCII Flows)

### 6.1 `gik init`

**Goal:** initialize GIK structure and perform first stack scan.

#### Steps

1. Check for existing `.guided/knowledge`.
2. Create `config.yaml`, `branches/<branch>/...`.
3. Write initial `Revision` (`Init`) and `HEAD`.
4. Run full stack scan → populate `stack/*`.
5. Finish with a consistent `stack` + initial timeline.

#### Flow Diagram

```text
User
  |
  |  gik init
  v
+---------+      +-----------+        +-----------------------+
| gik CLI | ---> | GikEngine | -----> | .guided/knowledge/... |
+---------+      +-----------+        +-----------------------+
                     |
                     | 1. create dirs/files
                     | 2. write initial Revision
                     | 3. run full stack scan
                     v
              Stack base populated
```

### 6.2 `gik add [TARGET ...] [--base NAME]`

**Goal:** stage sources for indexing and refresh stack inventory.

#### Steps

1. Resolve targets (default `.`).
2. For each target:
   - Infer `PendingSourceKind` (FilePath, Directory, Url, Archive).
   - Normalize URI (workspace-relative for local paths).
   - Infer target knowledge base from kind/extension (or use `--base`).
   - Skip if source doesn't exist or is already pending.
   - Create `PendingSource` entry in `staging/pending.jsonl`.
3. After processing all targets:
   - Trigger full stack rescan (full scan for correctness in this phase).
   - Update `staging/summary.json`.
4. Return summary: created IDs, skipped sources, stack stats.

**Note:** This phase uses full stack rescan for correctness. Future phases may optimize to incremental updates.

#### Flow Diagram

```text
User
  |
  |  gik add src/ README.md https://docs.example.com
  v
+---------+          +-----------+
| gik CLI |  ----->  | GikEngine |
+---------+          +-----------+
                         |
                         | 1. Infer kind/base for each target
                         | 2. Skip invalid/duplicate sources
                         | 3. Append to staging/pending.jsonl
                         | 4. Recompute staging/summary.json
                         | 5. Full stack rescan
                         v
              +----------------------+     +--------------------+
              | staging/             |     | stack/             |
              |   pending.jsonl      |     |   files.jsonl      |
              |   summary.json       |     |   dependencies.jsonl|
              +----------------------+     |   tech.jsonl       |
                                           |   stats.json       |
                                           +--------------------+
```

#### Source Kind Inference

| Input Pattern | Kind Inferred |
|---------------|---------------|
| `http://...` or `https://...` | `Url` |
| `.zip`, `.tar`, `.tar.gz`, `.tgz` | `Archive` |
| Existing directory | `Directory` |
| Existing file or path with extension | `FilePath` |
| Unknown | `Other` (skipped) |

#### Base Inference

| Source | Default Base |
|--------|--------------|
| URLs | `docs` |
| Directories | `code` |
| `.rs`, `.py`, `.js`, `.ts`, etc. | `code` |
| `.md`, `.txt`, `.pdf`, etc. | `docs` |

### 6.3 `gik commit [-m MESSAGE]`

**Goal:** process staging, update bases, and register new revision.

#### Steps

1. Read `staging/pending.jsonl`.
2. Apply `.gikignore` + `.gitignore`.
3. Determine active embedding config; create `EmbeddingProvider` via `gik-model`.
4. For each staged source:

   * collect files, classify into `code`/`docs`,
   * read content, chunk, embed (batched),
   * update `bases/<base>/sources.jsonl`, `bases/<base>/vectors/`, `bases/<base>/stats.json`.
5. For memory: append `bases/memory/events.jsonl`, update stats (and vectors if used).
6. Auto-sync KG: extract symbols and relationships, update `kg/nodes.jsonl` and `kg/edges.jsonl`.
7. Write `Revision` with `Commit` operation to `timeline.jsonl`.
8. Update `HEAD`.
9. Clear staging.

#### Flow Diagram

```text
User
  |
  |  gik commit -m "Index src/ and docs/"
  v
+---------+        +-----------+        +------------------------+
| gik CLI | -----> | GikEngine | -----> | gik-model (embeddings) |
+---------+        +-----------+        +------------------------+
                     |   ^
                     |   |
                     |   +------------------------------+
                     v                                  |
           .guided/knowledge/<branch>/                  |
           + staging/pending.jsonl                      |
           + bases/code/ (sources.jsonl, vectors/, stats)|
           + bases/docs/ (sources.jsonl, vectors/, stats)|
           + bases/memory/ (events.jsonl, stats, vectors/)|
           + kg/ (nodes.jsonl, edges.jsonl)             |
           + timeline.jsonl                             |
           + HEAD                                       |
```

#### Phase 4.3 Limitations

The current commit implementation has the following intentional limitations:

* **Single chunk per file**: Each file is treated as one chunk. No semantic chunking
  or splitting is performed. Large files exceeding 1MB or 10,000 lines are marked
  as `failed` with a descriptive reason.

* **Local files only**: Only `File` and `Directory` source kinds are fully supported.
  `Url` and `Archive` sources are **marked as `failed`** during commit with reason
  `"URL sources not supported in Phase 4.3"` or `"Archive sources not supported in Phase 4.3"`.
  URL/archive ingestion pipelines are planned for future phases.

* **No runtime mock backend**: The Candle embedding backend is required at runtime.
  If the default model is not available at `models/embeddings/all-MiniLM-L6-v2`,
  commit will fail with an actionable error message instructing the user to clone
  the model from Hugging Face:

  ```bash
  git clone https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 \
      models/embeddings/all-MiniLM-L6-v2
  ```

  `MockEmbeddingBackend` is only available in test builds (`#[cfg(test)]`).

### 6.4 `gik ask "QUESTION" [--bases ...] [--files PATTERN]`

**Goal:** run semantic query and return structured context.

#### Steps

1. Determine target bases (default: all relevant).
2. For each base with embeddings:

   * load `bases/<base>/meta.json`, check compatibility with active embedding config,
   * fail fast if mismatch (ask for `gik reindex`).
3. Build `EmbeddingProvider` via `gik-model`.
4. Embed the question (with optional query expansion for multiple embeddings).
5. For each base:

   * hybrid search: BM25 + vector search with RRF fusion → `rag_chunks`.
6. Retrieve memory events (vector search on memory base).
7. Expand context via KG neighborhood (related symbols, callers/callees).
8. Rerank combined results using cross-encoder (optional).
9. Build `StackSummary`.
10. Return `AskContextBundle` as JSON.

#### Flow Diagram

```text
User
  |
  |  gik ask "How does the web app call the API?"
  v
+---------+       +-----------+       +------------------------+
| gik CLI | ----> | GikEngine | ----> | gik-model (embeddings) |
+---------+       +-----------+       +------------------------+
                      |
                      v
              +----------------+
              | gik-db (LanceDB)|
              | hybrid search  |
              +----------------+
                      |
                      v
          rag_chunks + memory_events + kg_results
                      |
                      v
              AskContextBundle (JSON)
                      |
                      v
                printed to stdout
```

### 6.5 `gik reindex --base NAME`

**Goal:** rebuild embeddings + index for a base using the current embedding model.

#### Steps

1. Read project config and active embedding config.
2. Read `bases/<base>/meta.json` (old model info).
3. Build `EmbeddingProvider` via `gik-model`.
4. Read `bases/<base>/sources.jsonl` or equivalent input.
5. Re-embed all entries (batched).
6. Rebuild vector index in `bases/<base>/vectors/`.
7. Update `bases/<base>/meta.json` with new model info.
8. Auto-sync KG if relevant base.
9. Append `Revision` with `Reindex` operation to `timeline.jsonl`.
10. Update `HEAD`.

#### Flow Diagram

```text
User
  |
  |  gik reindex --base code
  v
+---------+       +-----------+       +------------------------+
| gik CLI | ----> | GikEngine | ----> | EmbeddingProvider      |
+---------+       +-----------+       +------------------------+
                      |
                      v
                read code/sources.jsonl
                      |
                      v
                +--------------+
                | VectorIndex  |
                |  (rebuild)   |
                +--------------+
                      |
                      v
   update model-info.json + add Reindex revision to timeline
```

### 6.6 `gik status`

**Goal:** show a quick overview of GIK state.

#### Steps

1. Check `.guided/knowledge` existence.
2. Detect branch.
3. Enumerate bases under `bases/`.
4. For each base, read `stats.json` and `meta.json`.
5. Inspect staging.
6. Check KG stats.
7. Build `StatusReport` and print.

### 6.7 `gik bases`

**Goal:** list available bases for the current branch.

Simple list of base directories under `<branch>/bases/`.

### 6.8 `gik stats [--base NAME]`

**Goal:** show aggregated statistics (per base or global).

### 6.9 `gik show`

**Goal:** show knowledge timeline, ask history, or export KG.

#### Key Flags

* `--timeline` – show revision timeline (default)
* `--history` – show ask query history
* `--last <N>` – limit to last N entries
* `--kg-dot` – export KG in DOT format
* `--kg-mermaid` – export KG in Mermaid format

### 6.10 `gik release [--tag TAG]`

**Goal:** generate or update `CHANGELOG.md` based on revisions since last release.

#### High-level Flow

```text
Revisions (timeline.jsonl)
          |
          v
+-----------+       +----------------+
| GikEngine | ----> | CHANGELOG.md   |
+-----------+       +----------------+
```

### 6.11 `gik memory-metrics`

**Goal:** show memory base statistics.

* Entry count
* Token estimation
* Age distribution

### 6.12 `gik memory-prune`

**Goal:** prune memory entries based on age or token limits.

#### Key Flags

* `--older-than <DAYS>` – prune entries older than N days
* `--max-tokens <N>` – prune to stay under token limit
* `--archive` – move pruned entries to `archive.jsonl` instead of deleting

---

## 7. Error Handling & Logging

### 7.1 Error Handling

* Use `anyhow::Result<T>` at public boundaries (CLI and engine).
* Use `thiserror` for internal, domain-specific errors.
* Messages must suggest concrete actions (e.g., `gik init`, `gik reindex --base code`).

### 7.2 Logging

* Use `tracing` with `info`, `debug`, `warn`, `error`.
* Log level controlled via env (e.g., `GIK_LOG=debug`).

---

## 8. Extensibility

* **Embeddings:** new providers implement `EmbeddingProvider` in `gik-model` and are configured via `gik.yaml`.
* **Index backends:** `VectorIndexBackend` trait in `gik-db` allows swapping backends. LanceDB is the recommended default.
* **New bases:** follow the pattern `bases/<base>/` directory + `sources.jsonl` + `stats.json` + `meta.json` + `vectors/`.
* **KG & Memory:** fully implemented as modules in `gik-core`, with auto-sync on commit/reindex.
* **Adapter pattern:** `gik-core` uses `IntoGikResult` trait and adapter modules (`db_adapter.rs`, `model_adapter.rs`) to convert errors from leaf crates.

---

## 9. Performance

### 9.1 Batched Embeddings

Embeddings are computed in batches (default: 32) to optimize GPU/CPU utilization.

### 9.2 Parallel File Reading

File reading uses `rayon` for parallel processing across CPU cores.

### 9.3 Warm-up Embedding

First embedding call "warms up" the model to pay initialization cost upfront.

### 9.4 Hybrid Search

Ask queries use hybrid search combining:

* **BM25** (keyword-based) for exact term matching
* **Vector search** for semantic similarity
* **RRF (Reciprocal Rank Fusion)** to combine rankings

### 9.5 Query Expansion

Optional multi-query embedding averaging for improved recall on complex queries.

---

End of Technical Spec v0.3 (with ASCII diagrams).
