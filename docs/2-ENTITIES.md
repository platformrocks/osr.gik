# Guided Indexing Kernel (GIK) – Entities & Contracts

> Suggested path: `.guided/architecture/entities.md`
> Status: Draft v0.2

This document defines the **core entities, types and contracts** used by GIK (Guided Indexing Kernel). It complements the PRD and Technical Spec by focusing on the **domain model** and its invariants.

---

## 1. Overview – Domain Map

GIK groups its entities into six main domains:

1. **Workspace & Branching** – where knowledge lives (per project / per branch).
2. **Configuration & Embeddings** – how GIK knows which models and profiles to use.
3. **Stack / Inventory** – raw structural view of the project.
4. **Knowledge Bases (code/docs/memory/kg)** – semantic data and indices.
5. **Timeline & Revisions** – historical evolution of knowledge.
6. **Asking & Context Bundles** – query results and contracts with external tools.

All entities below should be considered **stable contracts** for GIK v0.x, even if internal implementation details change.

---

## 2. Workspace & Branching

### 2.1 `Workspace`

**Definition**
A logical project root where GIK operates.

**Key fields**

* `path: PathBuf` – absolute path to project root.
* `has_git: bool` – whether a `.git/` directory exists.
* `knowledge_root: PathBuf` – usually `<workspace>/.guided/knowledge`.

**Invariants**

* `knowledge_root` must be under the workspace path.
* If `has_git == true`, branch resolution uses Git metadata; otherwise, GIK uses `default`.

### 2.2 `BranchName`

**Definition**
A symbolic name that identifies a knowledge branch.

**Type**

* Alias: `type BranchName = String`.

**Examples**

* `"main"`, `"develop"`, `"feature/gik-kernel"`, `"default"` (when no Git repo).

**Invariants**

* A workspace may have multiple branches, each with its own bases and timeline.
* Branch name must be filesystem-safe (no characters invalid for directory names).

### 2.3 `BaseName`

**Definition**
A logical knowledge base under a branch.

**Type**

* Alias: `type BaseName = String`.

**Well-known base names**

* `"stack"` – structural inventory.
* `"code"` – chunks derived from code.
* `"docs"` – chunks derived from docs/text.
* `"memory"` – structured memory events.
* `"kg"` – knowledge graph (Phase 2).

**Invariants**

* Each `<branch>/<base>` must have a dedicated subdirectory.
* Base names are case-sensitive but should be treated as lower-case canonical identifiers.

---

## 3. Configuration & Embeddings

### 3.1 `GlobalConfig`

**Scope**: Stored at `~/.gik/config.yaml`.

**Fields (simplified)**

* `embeddings: EmbeddingsSection` – configuration for embedding providers and models.

**Contract**

* Must be readable **without** accessing any workspace.
* Provides sensible defaults (local Candle with MiniLM-L6-v2).

### 3.2 `EmbeddingsSection`

**Definition**
Top-level configuration for embedding providers and per-base overrides.

**Fields**

* `default: Option<EmbeddingConfigOverride>` – default settings for all bases.
* `bases: HashMap<String, BaseEmbeddingOverride>` – per-base overrides keyed by base name.

**JSON Example**

```yaml
embeddings:
  default:
    provider: candle
    model_id: sentence-transformers/all-MiniLM-L6-v2
    dimension: 384
    max_tokens: 512
  bases:
    docs:
      provider: ollama
      model_id: nomic-embed-text
      dimension: 768
```

### 3.3 `EmbeddingProviderKind`

**Definition**
Enum identifying the embedding backend.

**Variants**

* `Candle` – local embedding via the Candle framework (default).
* `Ollama` – embedding via local Ollama server.
* `Other(String)` – extensibility for future providers.

**Serialization**

* `"candle"`, `"ollama"`, or custom string.

**Invariants**

* `Candle` is the default provider (local-first).
* `Ollama` requires a running Ollama server.

### 3.4 `EmbeddingModelId`

**Definition**
Newtype wrapping a model identifier string.

**Type**

* `struct EmbeddingModelId(String)`

**Examples**

* `"sentence-transformers/all-MiniLM-L6-v2"` (default)
* `"nomic-embed-text"`
* `"BAAI/bge-small-en-v1.5"`

**Invariants**

* Must be non-empty.
* Format is provider-dependent (HuggingFace ID for Candle, Ollama model name for Ollama).

### 3.5 `EmbeddingConfig`

**Definition**
Resolved embedding configuration for a specific base.

**Fields**

* `provider: EmbeddingProviderKind` – backend type.
* `model_id: EmbeddingModelId` – model identifier.
* `dimension: usize` – embedding vector dimension.
* `max_tokens: usize` – maximum input tokens per chunk.
* `local_path: Option<PathBuf>` – path to local model files (Candle).

**Default Values**

* Provider: `Candle`
* Model ID: `sentence-transformers/all-MiniLM-L6-v2`
* Dimension: `384`
* Max Tokens: `512`
* Local Path: `~/.gik/models/embeddings/all-MiniLM-L6-v2`

**Invariants**

* `dimension > 0`.
* `max_tokens > 0`.
* For Candle provider, `local_path` should point to cloned HuggingFace model.

### 3.6 `ModelInfo`

**Definition**
Persisted metadata about the embedding model used to index a base.

**Fields**

* `provider: EmbeddingProviderKind` – backend that created the embeddings.
* `model_id: EmbeddingModelId` – model identifier.
* `dimension: usize` – vector dimension.
* `created_at: DateTime<Utc>` – timestamp of initial indexing.
* `last_reindexed_at: Option<DateTime<Utc>>` – timestamp of last reindex.

**Storage**

* JSON at `<branch>/bases/<base>/meta.json`.

**Invariants**

* Created when base is first indexed.
* Updated when base is reindexed with a different model.
* Used to detect model compatibility/migration needs.

### 3.7 `ModelCompatibility`

**Definition**
Result of comparing active `EmbeddingConfig` against stored `ModelInfo`.

**Variants**

* `Compatible` – config matches stored model-info; no action needed.
* `MissingModelInfo` – base has no meta.json; fresh index or migration unknown.
* `Mismatch { config, stored }` – config differs from stored; reindex recommended.

**Usage**

* Checked before `gik ask` to warn about potential stale embeddings.
* Checked before `gik commit` to determine if full reindex is needed.

### 3.8 `ProjectConfig`

**Scope**: Stored at `<workspace>/.guided/knowledge/config.yaml`.

**Fields**

* `embeddings: Option<EmbeddingsSection>` – project-level embedding overrides.

**Invariants**

* Project embeddings override global embeddings with the same resolution precedence.
* If not set, falls back to global config.

### 3.9 `EmbeddingBackend` (trait)

**Definition**
Abstraction over concrete embedding implementations.

**Rust signature (simplified)**

```rust
pub trait EmbeddingBackend: Send + Sync {
    fn provider_kind(&self) -> EmbeddingProviderKind;
    fn model_id(&self) -> &EmbeddingModelId;
    fn embed_batch(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, GikError>;
}
```

**Invariants**

* `embed_batch` must always return vectors of the configured dimension.
* The same `EmbeddingBackend` instance must be safe to use concurrently (Send + Sync).

### 3.10 Config Resolution Precedence

When resolving embedding configuration for a base, the following precedence applies:

1. **Project base override** – `project_config.embeddings.bases[base]`
2. **Global base override** – `global_config.embeddings.bases[base]`
3. **Global default** – `global_config.embeddings.default`
4. **Hardcoded default** – MiniLM-L6-v2 via Candle

**ASCII Diagram**

```text
Project Config (base)  →  Global Config (base)  →  Global Default  →  Hardcoded Default
       ↓                        ↓                       ↓                    ↓
  EmbeddingConfig ←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←←
       ↓
   ModelInfo (stored in <base>/meta.json)
       ↓
  ModelCompatibility check
```

---

## 3.A Vector Index Entities (Phase 4.2)

The **vector index layer** stores embeddings for semantic search operations.

### 3.A.1 `VectorIndexBackendKind`

**Definition**
Enum identifying the vector index backend type.

**Variants**

* `LanceDb` – LanceDB backend with efficient columnar storage (default).
* `SimpleFile` – file-based index with linear scan search (deprecated).
* `Other(String)` – extensibility for future backends.

**Serialization**

* `"lancedb"`, `"simple_file"`, or custom string.

### 3.A.2 `VectorMetric`

**Definition**
Distance/similarity metric for vector search.

**Variants**

* `Cosine` – cosine similarity (default).
* `Dot` – dot product similarity.
* `L2` – Euclidean distance.

**Serialization**

* `"cosine"`, `"dot"`, `"l2"`.

### 3.A.3 `VectorId`

**Definition**
Identifier for a vector in the index.

**Type**

* `struct VectorId(pub u64)`

### 3.A.4 `VectorIndexConfig`

**Definition**
Resolved vector index configuration for a specific base.

**Fields**

* `backend: VectorIndexBackendKind` – backend type.
* `metric: VectorMetric` – similarity metric.
* `dimension: u32` – vector dimension (must match embedding dimension).
* `base: String` – knowledge base name.

**Default Values**

* Backend: `LanceDb`
* Metric: `Cosine`
* Dimension: taken from `EmbeddingConfig`

### 3.A.5 `VectorIndexMeta`

**Definition**
Persisted metadata about the vector index for a base.

**Fields**

* `backend: String` – backend type.
* `metric: String` – similarity metric.
* `dimension: u32` – vector dimension.
* `base: String` – knowledge base name.
* `embeddingProvider: String` – embedding provider used.
* `embeddingModelId: String` – embedding model used.
* `createdAt: DateTime<Utc>` – timestamp of index creation.
* `lastUpdatedAt: DateTime<Utc>` – timestamp of last update.

**Storage**

* JSON at `<branch>/bases/<base>/index/meta.json`.

**JSON Example**

```json
{
  "backend": "lancedb",
  "metric": "cosine",
  "dimension": 384,
  "base": "code",
  "embeddingProvider": "candle",
  "embeddingModelId": "sentence-transformers/all-MiniLM-L6-v2",
  "createdAt": "2025-11-27T01:20:00Z",
  "lastUpdatedAt": "2025-11-27T01:20:00Z"
}
```

### 3.A.6 `VectorInsert`

**Definition**
A vector to be inserted into the index.

**Fields**

* `id: VectorId` – unique identifier.
* `embedding: Vec<f32>` – the embedding vector.
* `payload: serde_json::Value` – metadata (doc ID, chunk ID, etc.).

### 3.A.7 `VectorSearchResult`

**Definition**
Result of a vector search query.

**Fields**

* `id: VectorId` – the vector ID.
* `score: f32` – similarity/distance score.
* `payload: serde_json::Value` – associated metadata.

### 3.A.8 `VectorIndexStats`

**Definition**
Statistics for a vector index.

**Fields**

* `count: u64` – total number of vectors.
* `dimension: u32` – vector dimension.
* `backend: String` – backend type.
* `metric: String` – similarity metric.

### 3.A.9 `VectorIndexCompatibility`

**Definition**
Result of comparing active `VectorIndexConfig` + `EmbeddingConfig` against stored `VectorIndexMeta`.

**Variants**

* `Compatible` – config matches stored meta; no action needed.
* `MissingMeta` – no meta.json exists; fresh index or first indexing.
* `EmbeddingMismatch { config_model, meta_model }` – embedding model differs (checked first).
* `DimensionMismatch { config, meta }` – vector dimension differs.
* `BackendMismatch { config_backend, meta_backend }` – backend type differs.

**Usage**

* Checked before `gik ask` to warn about stale index.
* Checked before `gik commit` to determine if reindex is needed.

### 3.A.10 `VectorIndexBackend` (trait)

**Definition**
Abstraction over vector index backend implementations.

**Rust signature (simplified)**

```rust
pub trait VectorIndexBackend: Send + Sync {
    fn backend_kind(&self) -> VectorIndexBackendKind;
    fn config(&self) -> &VectorIndexConfig;
    fn stats(&self) -> Result<VectorIndexStats, GikError>;
    fn upsert(&mut self, items: &[VectorInsert]) -> Result<(), GikError>;
    fn query(&self, query: &[f32], top_k: u32) -> Result<Vec<VectorSearchResult>, GikError>;
    fn delete(&mut self, ids: &[VectorId]) -> Result<(), GikError>;
    fn flush(&mut self) -> Result<(), GikError>;
}
```

### 3.A.11 Config Resolution Precedence

When resolving vector index configuration for a base:

1. **Project base override** – `project_config.indexes.bases[base]`
2. **Global base override** – `global_config.indexes.bases[base]`
3. **Global default** – `global_config.indexes.default`
4. **Hardcoded default** – LanceDb + Cosine

---

## 4. Stack / Inventory Entities

The **stack base** provides a structural, non-embedded inventory of the project.

### 4.1 `StackFileEntry`

**Fields**

* `path: String` – normalized path relative to workspace root.
* `kind: StackFileKind` – `Dir` or `File`.
* `languages: Vec<String>` – detected languages (e.g. `"rust"`, `"typescript"`).
* `file_count: Option<u64>` – number of files under this directory (if `kind == Dir`).

**Storage**

* Stored as JSONL in `stack/files.jsonl` (one `StackFileEntry` per line).

**Invariants**

* `path` must not be absolute.
* `file_count` is `None` for `kind == File`.

### 4.2 `StackFileKind`

**Enum**

* `Dir` – directory.
* `File` – file.

### 4.3 `StackDependencyEntry`

**Fields**

* `manager: String` – dependency manager (`"cargo"`, `"npm"`, `"pip"`, etc.).
* `name: String` – dependency name.
* `version: String` – declared version (raw string).
* `scope: String` – dependency scope (`"runtime"`, `"dev"`, `"build"` etc.).
* `manifest_path: String` – path of the file where this dependency is declared.

**Storage**

* JSONL in `stack/dependencies.jsonl`.

**Invariants**

* `manifest_path` is always relative to workspace.
* There may be multiple entries with the same `(manager, name)` but different manifests.

### 4.4 `StackTechEntry`

**Fields**

* `kind: String` – semantic type (`"framework"`, `"language"`, `"infra"`, `"tool"`).
* `name: String` – technology name (`"Next.js"`, `"Rust"`, `"Postgres"`).
* `source: String` – how this was inferred (`"dependency:next"`, `"files:*.rs"`).
* `confidence: f32` – heuristic confidence [0.0–1.0].

**Storage**

* JSONL in `stack/tech.jsonl`.

**Invariants**

* `0.0 <= confidence <= 1.0`.
* Tools consuming this must treat low confidence entries (e.g. < 0.5) as hints only.

### 4.5 `StackStats`

**Fields (example)**

* `total_files: u64`.
* `languages: HashMap<String, u64>` – per-language file count.
* `managers: Vec<String>` – managers detected (`["cargo", "npm"]`).
* `generated_at: DateTime<Utc>`.

**Storage**

* JSON at `stack/stats.json`.

**Invariants**

* `total_files == sum(languages.values())` is desirable but not guaranteed; best-effort.

---

## 5. Staging Entities

The **staging area** tracks pending sources that are queued for ingestion into knowledge bases.

### 5.1 `PendingSourceId`

**Definition**
Unique identifier for a pending source, generated as a UUID.

**Type**

* Newtype: `PendingSourceId(String)`.

**Examples**

* `"f47ac10b-58cc-4372-a567-0e02b2c3d479"`.

### 5.2 `PendingSourceKind`

**Definition**
Type of pending source.

**Enum**

* `filePath` – a single file path.
* `directory` – a directory (scanned recursively).
* `url` – a URL (web page, API, etc.).
* `archive` – an archive file (ZIP, tar, etc.).
* `other` – custom type with identifier.

**Phase 4.3 Behavior**

During `gik commit`:

* `filePath` and `directory` → fully supported and indexed.
* `url` and `archive` → **marked as `failed`** with a descriptive reason
  (e.g., `"URL sources not supported in Phase 4.3"`). These source kinds will
  be supported in future phases with dedicated ingestion pipelines.

### 5.3 `PendingSourceStatus`

**Definition**
Processing status of a pending source.

**Enum**

* `pending` – source is queued for processing.
* `processing` – source is currently being processed.
* `indexed` – source has been successfully indexed.
* `failed` – source processing failed.

### 5.4 `PendingSource`

**Fields**

* `id: PendingSourceId` – unique identifier.
* `branch: String` – branch this source belongs to.
* `base: String` – target knowledge base (`"code"`, `"docs"`, `"memory"`, `"kg"`).
* `kind: PendingSourceKind` – type of source.
* `uri: String` – normalized path or URL.
* `addedAt: DateTime<Utc>` – when the source was added.
* `status: PendingSourceStatus` – current processing status.
* `lastError: Option<String>` – error message if status is `failed`.
* `metadata: Option<serde_json::Value>` – optional extensibility metadata.

**Storage**

* JSONL in `staging/pending.jsonl`.

**Example**

```json
{
  "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
  "branch": "main",
  "base": "code",
  "kind": "filePath",
  "uri": "src/main.rs",
  "addedAt": "2025-11-27T18:00:00Z",
  "status": "pending"
}
```

### 5.5 `StagingSummary`

**Fields**

* `pendingCount: u64` – sources with status `pending` or `processing`.
* `indexedCount: u64` – sources with status `indexed`.
* `failedCount: u64` – sources with status `failed`.
* `byBase: HashMap<String, u64>` – pending count per knowledge base.
* `lastUpdatedAt: DateTime<Utc>` – when this summary was computed.

**Storage**

* JSON at `staging/summary.json`.

**Example**

```json
{
  "pendingCount": 5,
  "indexedCount": 10,
  "failedCount": 0,
  "byBase": {"code": 3, "docs": 2},
  "lastUpdatedAt": "2025-11-27T18:05:00Z"
}
```

### 5.6 `NewPendingSource`

**Definition**
Input type for adding a new pending source (no id/timestamps).

**Fields**

* `base: Option<String>` – target base (inferred from extension if omitted).
* `uri: String` – path or URL.
* `kind: Option<PendingSourceKind>` – type (inferred from URI if omitted).
* `metadata: Option<serde_json::Value>` – optional metadata.

**Invariants**

* If `base` is `None`, it is inferred from the file extension or source kind.
* If `kind` is `None`, it is inferred from the URI pattern.

### 5.7 `HeadInfo`

**Definition**
Summary of the current HEAD revision for status reporting.

**Fields**

* `revisionId: String` – the revision ID pointed to by HEAD.
* `operation: RevisionOperation` – the primary operation in this revision.
* `timestamp: DateTime<Utc>` – when the revision was created.
* `message: Option<String>` – human-readable commit message.

**Serialization**

* Uses camelCase JSON keys.
* `message` is omitted when `null`.

**Example**

```json
{
  "revisionId": "abc12345-6789-def0-1234-567890abcdef",
  "operation": {"type": "Init"},
  "timestamp": "2025-11-27T18:00:00Z",
  "message": "Initialize GIK workspace"
}
```

### 5.8 `StatusReport`

**Definition**
Complete status report for a workspace and branch, aggregating information from multiple sources.

**Fields**

* `workspaceRoot: PathBuf` – absolute path to the workspace root.
* `branch: BranchName` – current branch name.
* `isInitialized: bool` – whether the branch is initialized for GIK.
* `head: Option<HeadInfo>` – HEAD revision info (if available).
* `staging: Option<StagingSummary>` – staging summary (if available).
* `stack: Option<StackStats>` – stack statistics (if available).
* `bases: Option<Vec<BaseStatsReport>>` – per-base stats and health (Phase 6.2).

**Serialization**

* Uses camelCase JSON keys.
* Optional fields are omitted when `null`.

**Example (initialized with per-base stats)**

```json
{
  "workspaceRoot": "/path/to/project",
  "branch": "main",
  "isInitialized": true,
  "head": {
    "revisionId": "abc12345-...",
    "operation": {"type": "Init"},
    "timestamp": "2025-11-27T18:00:00Z",
    "message": "Initialize GIK workspace"
  },
  "staging": {
    "pendingCount": 3,
    "indexedCount": 0,
    "failedCount": 0,
    "byBase": {"code": 2, "docs": 1},
    "lastUpdatedAt": "2025-11-27T18:05:00Z"
  },
  "stack": {
    "totalFiles": 42,
    "languages": {"rust": 30, "markdown": 12}
  },
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
      "health": "HEALTHY"
    },
    {
      "base": "docs",
      "documents": 12,
      "vectors": 12,
      "files": 12,
      "onDiskBytes": 65536,
      "lastCommit": "2025-11-27T18:10:00Z",
      "embeddingStatus": "compatible",
      "indexStatus": "compatible",
      "health": "HEALTHY"
    }
  ]
}
```

**Example (uninitialized)**

```json
{
  "workspaceRoot": "/path/to/project",
  "branch": "main",
  "isInitialized": false
}
```

### 5.9 `BaseStatsReport` (Phase 6.2)

**Definition**
Extended base statistics with compatibility and health information, used in `StatusReport.bases`.

**Fields**

* `base: String` – the knowledge base name.
* `documents: u64` – total number of source entries (chunks/documents).
* `vectors: u64` – total number of vectors in the index.
* `files: u64` – total number of unique files indexed.
* `onDiskBytes: u64` – approximate on-disk size in bytes (sources + index + meta files).
* `lastCommit: Option<DateTime<Utc>>` – when the base was last updated (from `stats.json.last_updated`).
* `embeddingStatus: Option<String>` – embedding model compatibility status (`"compatible"`, `"missing"`, `"mismatch"`).
* `indexStatus: Option<String>` – vector index compatibility status (`"compatible"`, `"missing"`, `"dimension_mismatch"`, `"backend_mismatch"`, `"embedding_mismatch"`).
* `health: BaseHealthState` – overall health state for the base.

**Serialization**

* Uses camelCase JSON keys.
* `health` is serialized as SCREAMING_SNAKE_CASE (e.g., `"HEALTHY"`, `"NEEDS_REINDEX"`).

**Derivation**

The `onDiskBytes` is computed by summing sizes of core contract files only:

* `<branch>/bases/<base>/sources.jsonl`
* `<branch>/bases/<base>/stats.json`
* `<branch>/bases/<base>/meta.json`
* All files under `<branch>/bases/<base>/index/`

The `lastCommit` uses `stats.json.last_updated` as a proxy (timeline-based derivation is deferred to future phases).

### 5.10 `BaseHealthState` (Phase 6.2)

**Definition**
Health indicator for a knowledge base, derived from embedding model compatibility and vector index compatibility.

**Enum Values**

| Value | Display | Description |
|-------|---------|-------------|
| `Healthy` | `OK` | Base is healthy: model and index are compatible. |
| `NeedsReindex` | `NEEDS_REINDEX` | Base requires reindexing due to model or index mismatch. |
| `MissingModel` | `MISSING_MODEL` | Model info is missing (base never indexed or corrupted). |
| `IndexMissing` | `INDEX_MISSING` | Index files are missing (base never indexed). |
| `Error` | `ERROR` | An error occurred while checking health. |

**Derivation Logic**

1. If `ModelCompatibility::MissingModelInfo` and index directory doesn't exist → `IndexMissing`
2. If `ModelCompatibility::MissingModelInfo` (but index exists) → `MissingModel`
3. If `ModelCompatibility::Mismatch` → `NeedsReindex`
4. If `VectorIndexCompatibility::MissingMeta` → `IndexMissing`
5. If any `VectorIndexCompatibility` mismatch variant → `NeedsReindex`
6. If both `Compatible` → `Healthy`
7. Otherwise → `Error`

---

## 6. Knowledge Bases (code, docs, memory, kg)

### 6.1 `KnowledgeBase`

**Concept (not necessarily a single struct)**

Each base under a branch (e.g., `code/`, `docs/`, `memory/`) follows a standard layout:

* `sources.jsonl` – chunk or entry metadata.
* `meta.json` – embedding and index metadata.
* `stats.json` – aggregated statistics.
* `index/` – vector index backend files (bases with embeddings).

### 6.2 `BaseSourceEntry`

**Fields** (example for code/docs)

* `id: String` – unique chunk id.
* `base: BaseName` – `"code"` or `"docs"`.
* `path: String` – file path relative to workspace.
* `start_line: u32` – 1-based inclusive.
* `end_line: u32` – 1-based inclusive.
* `text: String` – raw text for this chunk (or omitted if text is recovered from file).
* `extra: serde_json::Value` – optional metadata (tags, language, etc.).

**Storage**

* JSONL in `<branch>/bases/<base>/sources.jsonl`.

**Invariants**

* `start_line <= end_line`.
* If `text` is omitted, the consumer must be able to reconstruct it from `(path, start_line, end_line)`.

### 6.3 `BaseStats`

**Fields** (example)

* `chunk_count: u64`.
* `file_count: u64`.
* `last_updated: DateTime<Utc>`.

**Storage**

* JSON at `<branch>/bases/<base>/stats.json`.

### 6.4 `ModelInfo`

**Fields**

* `provider_type: String` – e.g. `"candle-sbert"`.
* `model_id: String`.
* `dim: usize`.
* `created_at: DateTime<Utc>`.

**Storage**

* JSON at `<branch>/bases/<base>/meta.json`.

**Invariants**

* Must match the active embedding profile for any base used in `gik ask`.
* If mismatch is detected, the base is considered **invalid** until `gik reindex` is run.

### 6.5 `MemoryScope`

**Definition**
Defines the visibility and scope of a memory entry.

**Variants**

* `Project` – Project-wide knowledge that applies across all branches.
* `Branch` – Branch-specific knowledge that only applies to a particular branch.
* `Global` – Global knowledge that could apply across multiple projects (future).

**Serialization**

* JSON: `"project"`, `"branch"`, `"global"` (camelCase).

**Invariants**

* Default scope is `Project`.
* Branch-scoped entries should only appear in queries for that branch.

### 6.6 `MemorySource`

**Definition**
Categorizes how a memory entry was created.

**Variants**

* `ManualNote` – Directly added by user or UI as a manual note (default).
* `Decision` – A decision with rationale, typically from discussions or design documents.
* `Observation` – Observation from experiments, tests, debugging, or code review.
* `ExternalReference` – Summarized or linked external content (docs, articles, etc.).
* `AgentGenerated` – Generated by an AI agent during conversation or task execution.
* `CommitContext` – Extracted from commit messages or code review comments.

**Serialization**

* JSON: `"manualNote"`, `"decision"`, `"observation"`, `"externalReference"`, `"agentGenerated"`, `"commitContext"` (camelCase).

**Invariants**

* Default source is `ManualNote`.
* Source type helps with filtering, prioritization, and pruning strategies.

### 6.7 `MemoryEntry`

**Definition**
A single memory record that can be embedded and indexed in the memory base.

**Fields**

* `id: String` – unique memory ID (generated from hash of content + timestamp).
* `created_at: DateTime<Utc>` – when the entry was created.
* `updated_at: DateTime<Utc>` – when the entry was last modified.
* `scope: MemoryScope` – visibility scope (project, branch, global).
* `source: MemorySource` – how the memory was created.
* `title: String` – short summary or headline (≤100 chars).
* `text: String` – full content of the memory entry.
* `tags: Vec<String>` – classification tags.
* `branch: Option<String>` – branch name if scope is `Branch`.
* `origin_revision: Option<RevisionId>` – revision where this memory was created.

**Storage**

* JSONL in `.guided/knowledge/<branch>/bases/memory/sources.jsonl`.
* Same embedding/index pipeline as code/docs bases.

**Invariants**

* `id` must be unique per workspace.
* `title` should be ≤100 characters.
* `text` can be longer but embedding will truncate at `max_tokens`.
* If `scope == Branch`, then `branch` should be set.
* Timestamps are always UTC.

### 6.8 `MemoryEvent` (Response Type)

**Definition**
A memory entry as returned in `AskContextBundle.memory_events`, enriched with search score.

**Fields**

* `id: String` – memory entry ID.
* `scope: MemoryScope` – visibility scope.
* `title: String` – short summary.
* `text: String` – full content.
* `created_at: DateTime<Utc>` – creation timestamp.
* `source: MemorySource` – how the memory was created.
* `tags: Vec<String>` – classification tags.
* `score: Option<f32>` – relevance score from vector search.

**Serialization**

* JSON with camelCase field names.

**Invariants**

* `score` is only present when returned from vector search.

### 6.9 `MemoryIngestionResult` (Phase 7.2)

**Definition**
Result summary from memory ingestion via `ingest_memory_entries()`.

**Fields**

* `ingested_count: usize` – number of entries successfully ingested.
* `failed_count: usize` – number of entries that failed to ingest.
* `ingested_ids: Vec<String>` – IDs of successfully ingested entries.
* `failed: Vec<(String, String)>` – IDs and error messages for failed entries.
* `vector_count: u64` – number of vectors created.

**Serialization**

* JSON with camelCase field names.

**Invariants**

* `is_success()` returns true if `failed_count == 0`.
* `ingested_count + failed_count` equals the number of input entries.

### 6.10 `MemoryIngestResult` (Phase 7.2)

**Definition**
Result from `GikEngine::ingest_memory()` combining the revision ID and detailed ingestion result.

**Fields**

* `revision_id: Option<String>` – timeline revision ID (None if nothing ingested).
* `result: MemoryIngestionResult` – detailed ingestion result.

**Serialization**

* JSON with camelCase field names.

**Invariants**

* `revision_id` is `Some` if and only if at least one entry was ingested.

### 6.11 KG Entities (Phase 9.1+)

The Knowledge Graph (KG) stores structural relationships extracted from code and docs.

**Phase 9.1**: Storage infrastructure (entities, store, file layout).
**Phase 9.2**: KG extraction from bases (file-level imports), sync on commit/reindex.
**Phase 9.3+**: KG-aware ask, endpoint detection, symbol-level graphs.

#### KG Extraction (Phase 9.2)

The KG is **derived** from existing bases (`code`, `docs`) – it is NOT a primary source of truth.
Extraction produces:

- **File nodes** (`kind = "file"`) from `code` base sources
- **Doc nodes** (`kind = "doc"`) from `docs` base sources
- **Import edges** (`kind = "imports"`) between file nodes, detected via regex heuristics

**Supported import detection** (file-level only):

| Language | Patterns |
|----------|----------|
| JavaScript/TypeScript | `import ... from '...'`, `require('...')` |
| Rust | `use crate::...`, `use super::...`, `mod ...` |
| Python | `import ...`, `from ... import ...` |

**Recommended props conventions**:

| Node/Edge | Props Key | Description |
|-----------|-----------|-------------|
| File node | `base` | `"code"` |
| File node | `path` | Workspace-relative path |
| Doc node | `base` | `"docs"` |
| Doc node | `path` | Workspace-relative path |
| Import edge | `rawImport` | Raw import string from source |

**Example: import graph**

```
file:src/index.ts  --imports-->  file:src/utils.ts
file:src/index.ts  --imports-->  file:src/config.ts
file:src/api/handler.ts  --imports-->  file:src/utils.ts
```

**Sync strategy**: Full rebuild per branch on `gik commit` and `gik reindex`.

#### `KgNode`

Represents an entity in the knowledge graph (file, module, function, class, concept, etc.).

**Fields**

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | Unique node identifier. Convention: `<type>:<path>` (e.g., `file:src/main.rs`, `fn:lib::parse`) |
| `kind` | `String` | Node type: `"file"`, `"module"`, `"function"`, `"class"`, `"struct"`, `"trait"`, `"concept"`, `"dependency"`, `"service"`, `"endpoint"` |
| `label` | `String` | Human-readable label for display |
| `props` | `serde_json::Value` | Arbitrary properties as a JSON object (language, line count, complexity, tags, etc.) |
| `branch` | `Option<String>` | Optional branch name if this node is branch-specific |
| `createdAt` | `DateTime<Utc>` | Timestamp when the node was created |
| `updatedAt` | `DateTime<Utc>` | Timestamp when the node was last updated |

**Example (JSON)**

```json
{
  "id": "file:src/main.rs",
  "kind": "file",
  "label": "src/main.rs",
  "props": {"language": "rust", "lines": 150},
  "branch": "main",
  "createdAt": "2025-11-28T10:00:00Z",
  "updatedAt": "2025-11-28T10:00:00Z"
}
```

**Endpoint Node Example**

When `extract_endpoints: true` (default), endpoint nodes are created for detected API routes:

```json
{
  "id": "endpoint:/api/users",
  "kind": "endpoint",
  "label": "/api/users",
  "props": {
    "route": "/api/users",
    "httpMethod": "GET,POST",
    "definedIn": "src/app/api/users/route.ts"
  },
  "branch": "main",
  "createdAt": "2025-11-28T10:00:00Z",
  "updatedAt": "2025-11-28T10:00:00Z"
}
```

**Supported endpoint patterns** (Phase 9.3, Next.js only):

| Pattern | Framework | Example |
|---------|-----------|---------|
| `app/api/**/route.ts` | Next.js App Router | `app/api/users/route.ts` → `/api/users` |
| `pages/api/**.ts` | Next.js Pages Router | `pages/api/users.ts` → `/api/users` |

> **TODO (Phase 9.4+)**: Support Express, FastAPI, Actix, and other frameworks.

**Symbol Node Example (Phase 9.2.1)**

When `extract_symbols: true` (default), symbol nodes are created for detected code constructs:

```json
{
  "id": "sym:ts:src/utils.ts:function:calculateTotal",
  "kind": "function",
  "label": "calculateTotal",
  "props": {
    "language": "ts",
    "definedIn": "src/utils.ts",
    "framework": null
  },
  "branch": "main",
  "createdAt": "2025-01-15T10:00:00Z",
  "updatedAt": "2025-01-15T10:00:00Z"
}
```

**Symbol ID Format**: `sym:<lang>:<normalizedFilePath>:<kind>:<name>[#<index>]`

- `<lang>`: Language tag (e.g., `ts`, `js`, `py`, `rs`, `rb`, `go`, `java`, `kt`, `cs`, `c`, `cpp`, `php`, `sql`, `md`)
- `<normalizedFilePath>`: File path with `/` separators (Windows-safe)
- `<kind>`: Symbol kind (e.g., `function`, `class`, `struct`, `interface`, `trait`, `module`, `constant`, `type`)
- `<name>`: Symbol name
- `#<index>`: Optional suffix for duplicates (e.g., `#1`, `#2`)

**Supported languages and symbol kinds** (Phase 9.2.1):

| Language | File Extensions | Symbol Kinds Extracted |
|----------|-----------------|------------------------|
| JavaScript/TypeScript | `.js`, `.ts`, `.jsx`, `.tsx`, `.mjs`, `.mts`, `.cjs`, `.cts` | `function`, `class`, `interface`, `type`, `reactComponent`, `uiComponent`, `ngComponent`, `ngModule`, `ngService`, `ngRoute` |
| Python | `.py`, `.pyi` | `function`, `class`, `constant` |
| Rust | `.rs` | `function`, `struct`, `enum`, `trait`, `module`, `constant`, `type` |
| Ruby | `.rb`, `.rake`, `.gemspec` | `class`, `method` |
| C# | `.cs` | `class`, `interface` |
| Java | `.java` | `class`, `interface`, `method` |
| Go | `.go` | `function`, `struct`, `interface`, `constant`, `type` |
| Kotlin | `.kt`, `.kts` | `function`, `class`, `object` |
| C | `.c`, `.h` | `function`, `struct`, `macro` |
| C++ | `.cpp`, `.hpp`, `.cc`, `.cxx`, `.hxx` | `class`, `namespace`, `enum` |
| PHP | `.php` | `function`, `class`, `interface` |
| SQL | `.sql` | `table`, `function`, `view`, `index` |
| Markdown | `.md`, `.mdx` | `heading` |
| CSS | `.css`, `.scss`, `.sass`, `.postcss` | `styleClass`, `styleId`, `cssVariable`, `tailwindDirective` |
| HTML | `.html`, `.htm` | `htmlTemplate`, `htmlSection`, `htmlAnchor` |

**Frontend-specific symbol kinds** (Phase 9.2.2):

| Symbol Kind | Language | Description |
|-------------|----------|-------------|
| `reactComponent` | JS/TS | React functional components (PascalCase functions returning JSX) |
| `uiComponent` | JS/TS | shadcn/ui component imports (`@/components/ui/*`) |
| `ngComponent` | JS/TS | Angular components (`@Component` decorator) |
| `ngModule` | JS/TS | Angular modules (`@NgModule` decorator) |
| `ngService` | JS/TS | Angular services (`@Injectable` decorator) |
| `ngRoute` | JS/TS | Angular route definitions |
| `styleClass` | CSS | CSS class selectors (`.classname`) |
| `styleId` | CSS | CSS ID selectors (`#id`) |
| `cssVariable` | CSS | CSS custom properties (`--variable-name`) |
| `tailwindDirective` | CSS | Tailwind directives (`@tailwind base/components/utilities`) |
| `htmlTemplate` | HTML | Root HTML document template |
| `htmlSection` | HTML | Semantic sections with IDs (`section`, `header`, `footer`, etc.) |
| `htmlAnchor` | HTML | Non-section elements with IDs (anchor targets) |

**Framework Detection**:

Symbols include framework hints when detected:
- **Next.js**: React components in `app/` or `pages/` directories
- **React**: Files importing from `'react'` or containing JSX
- **shadcn/ui**: Files importing from `@/components/ui/*`
- **Angular**: Files with `@Component`, `@NgModule`, `@Injectable` decorators
- **Tailwind**: CSS files with `@tailwind` or `@apply` directives
- **Django**: Classes in `views.py`, `models.py`, etc.
- **Flask**: Functions decorated with routes
- **Rails**: Classes ending in `Controller`, `Model`, etc.
- **Spring**: Classes annotated with `@Controller`, `@Service`, etc.
- **ASP.NET**: Controllers inheriting from `Controller`
- **Gin/Fiber**: Go router handlers

**Symbol Extraction Config**:

| Config Key | Type | Default | Description |
|------------|------|---------|-------------|
| `extract_symbols` | `bool` | `true` | Enable/disable symbol extraction |
| `max_symbols_per_file` | `Option<usize>` | `None` | Limit symbols per file (for large files) |

#### `KgEdge`

Represents a directed relationship between two nodes.

**Fields**

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | Unique edge identifier. Convention: `edge:<hash>:<kind>` or explicit ID |
| `from` | `String` | Source node ID |
| `to` | `String` | Target node ID |
| `kind` | `String` | Relationship type: `"dependsOn"`, `"calls"`, `"contains"`, `"imports"`, `"extends"`, `"implements"`, `"uses"`, `"ownedBy"`, `"relatedTo"`, `"definesEndpoint"` |
| `props` | `serde_json::Value` | Arbitrary properties (weight, count, confidence, etc.) |
| `branch` | `Option<String>` | Optional branch name if this edge is branch-specific |
| `createdAt` | `DateTime<Utc>` | Timestamp when the edge was created |
| `updatedAt` | `DateTime<Utc>` | Timestamp when the edge was last updated |

**Edge Kinds**

| Kind | Description | Example |
|------|-------------|---------|
| `imports` | Source file imports target file | `file:a.ts` → `file:b.ts` |
| `definesEndpoint` | File defines an API endpoint | `file:api/route.ts` → `endpoint:/api` |
| `defines` | File defines a symbol | `file:src/utils.ts` → `sym:ts:src/utils.ts:function:helper` |
| `dependsOn` | General dependency relationship | module → package |
| `calls` | Function/method call relationship | `fn:main` → `fn:helper` |
| `contains` | Containment relationship | module → function |
| `extends` | Inheritance relationship | class → base class |
| `implements` | Interface implementation | class → interface |
| `usesClass` | Component uses CSS class (Phase 9.2.2) | `file:Button.tsx` → `sym:css:*:styleClass:btn` |
| `usesUiComponent` | File uses UI component (Phase 9.2.2) | `file:page.tsx` → `uiComponent:Button` |
| `belongsToModule` | Angular component belongs to module (Phase 9.2.2) | `ngComponent:Header` → `ngModule:AppModule` |

**Frontend Relation Edges** (Phase 9.2.2):

Frontend extractors create edges that may reference **unresolved** targets:

| Edge Kind | From | To | Props |
|-----------|------|-----|-------|
| `usesClass` | `file:<path>` | `sym:css:*:styleClass:<name>` | `className`, `source`, `unresolved` |
| `usesUiComponent` | `file:<path>` | `uiComponent:<name>` | `componentName`, `source`, `unresolved` |
| `belongsToModule` | `sym:js:<path>:ngComponent:<name>` | `sym:js:<path>:ngModule:<name>` | `moduleName` |

> **Note**: Edges with `unresolved: true` indicate the target symbol may not exist as a node.
> The CSS class may be defined in a different file, or may be a Tailwind utility class.
> Resolution of these edges is deferred to Phase 9.4+.

**Example (JSON)**

```json
{
  "id": "edge:a1b2c3d4e5f6:imports",
  "from": "file:src/main.rs",
  "to": "file:src/lib.rs",
  "kind": "imports",
  "props": {"count": 3},
  "createdAt": "2025-11-28T10:00:00Z",
  "updatedAt": "2025-11-28T10:00:00Z"
}
```

**definesEndpoint Edge Example**

```json
{
  "id": "edge:b2c3d4e5f6a7:definesEndpoint",
  "from": "file:src/app/api/users/route.ts",
  "to": "endpoint:/api/users",
  "kind": "definesEndpoint",
  "props": {},
  "createdAt": "2025-11-28T10:00:00Z",
  "updatedAt": "2025-11-28T10:00:00Z"
}
```

#### `KgStats`

Aggregate statistics for the knowledge graph.

**Fields**

| Field | Type | Description |
|-------|------|-------------|
| `nodeCount` | `u64` | Total number of nodes in the graph |
| `edgeCount` | `u64` | Total number of edges in the graph |
| `lastUpdated` | `DateTime<Utc>` | Timestamp when the stats were last computed |
| `version` | `String` | Schema version string (e.g., `"kg-v1"`) |

**Example (JSON)**

```json
{
  "nodeCount": 200,
  "edgeCount": 500,
  "lastUpdated": "2025-11-28T10:00:00Z",
  "version": "kg-v1"
}
```

**Storage**

* Nodes: JSONL at `.guided/knowledge/<branch>/kg/nodes.jsonl`
* Edges: JSONL at `.guided/knowledge/<branch>/kg/edges.jsonl`
* Stats: JSON at `.guided/knowledge/<branch>/kg/stats.json`

**Invariants**

* Both `KgNode.id` and `KgEdge.id` MUST be unique within their respective files.
* `KgEdge.from` and `KgEdge.to` SHOULD reference existing node IDs (soft constraint).
* The `kg/` directory is created lazily on first write (not during `gik init`).
* If KG has not been built yet, `kg/` may be absent.

---

## 6. Timeline & Revisions

### 6.1 `RevisionId`

**Type**

* Alias: `type RevisionId = String`.

**Format**

* UUIDv4 or a hash-like string. Must be unique per branch.

### 6.2 `Revision`

**Fields**

* `id: RevisionId`.
* `parent_id: Option<RevisionId>`.
* `branch: BranchName`.
* `git_commit: Option<String>` – associated Git commit hash, if available.
* `timestamp: DateTime<Utc>`.
* `message: String`.
* `operations: Vec<RevisionOperation>`.

**Storage**

* JSONL at `timeline.jsonl` (one `Revision` per line).

**Invariants**

* `parent_id` must be `None` only for `Init` revisions.
* All revisions in `timeline.jsonl` form a DAG; GIK initially assumes a linear chain per branch.

### 6.3 `RevisionOperation`

**Variants**

* `Init` – initial workspace/branch setup.
* `Commit { bases: Vec<BaseName>, source_count: usize }` – index update.
* `MemoryIngest { count: usize }` – memory entries added to the `memory` base.
* `MemoryPrune { count: usize, archived_count: usize, deleted_count: usize }` – memory entries removed or archived.
* `Reindex { base: BaseName, from_model_id: String, to_model_id: String }` – embedding model change.
* `Release { tag: Option<String> }` – release / changelog event.
* `Custom { name: String, data: Option<Value> }` – extensibility for custom operations.

**Serialization**

* Uses tagged JSON format with `"type"` discriminant.
* Fields use camelCase: `sourceCount`, `fromModelId`, `toModelId`, `archivedCount`, `deletedCount`.

**Invariants**

* `source_count` must reflect the number of staged sources processed.
* `from_model_id` and `to_model_id` must be valid model ids as used in `ModelInfo`.
* `MemoryIngest` is created only when at least one entry was successfully ingested.
* `MemoryPrune` is created only when at least one entry was pruned.

### 6.4 `HEAD`

**Definition**
A single file containing the current `RevisionId` for a branch.

**Invariants**

* Must always point to an existing revision in `timeline.jsonl`.
* After each `Init`, `Commit`, `Reindex`, `MemoryIngest`, `MemoryPrune`, or `Release`, `HEAD` must be updated.

### 6.5 Log Query Types (Phase 6.1)

#### `LogKind`

**Variants**

* `Timeline` – branch timeline entries (commits, reindex, releases).
* `Ask` – ask log entries (query history).

#### `TimelineOperationKind`

**Variants**

* `Init` – workspace initialization.
* `Commit` – commit of staged sources.
* `MemoryIngest` – memory entries added.
* `MemoryPrune` – memory entries removed or archived.
* `Reindex` – reindex of a base.
* `Release` – release with optional tag.
* `Other(String)` – custom or unknown operation.

#### `LogQueryScope`

**Purpose**
Filters and scope for a log query.

**Fields**

* `branch: Option<String>` – branch to query (None = current).
* `kind: Option<LogKind>` – log kind (None = Timeline).
* `timeline_ops: Option<Vec<TimelineOperationKind>>` – operation filter.
* `bases: Option<Vec<String>>` – base filter.
* `since: Option<DateTime<Utc>>` – time filter (inclusive).
* `until: Option<DateTime<Utc>>` – time filter (inclusive).
* `limit: Option<usize>` – maximum entries.

#### `TimelineLogEntry`

**Purpose**
A single entry from the timeline log.

**Fields**

* `branch: String` – branch name.
* `timestamp: DateTime<Utc>` – when the operation occurred.
* `operation: TimelineOperationKind` – the operation type.
* `revision_id: String` – the revision ID.
* `bases: Vec<String>` – affected bases.
* `message: Option<String>` – commit message.
* `meta: Option<Value>` – extra metadata.

#### `AskLogEntry`

**Purpose**
An entry in the ask log (stored in `asks/ask.log.jsonl`).

**Fields**

* `timestamp: DateTime<Utc>` – when the query was executed.
* `branch: String` – branch context.
* `question: String` – the original question.
* `bases: Vec<String>` – bases that were queried.
* `total_hits: u32` – number of RAG chunks returned.
* `bundle_path: Option<String>` – path to persisted bundle (future).

**Invariants**

* Every successful `gik ask` appends exactly one entry.
* Failed queries do not create entries.

#### `AskLogView`

**Purpose**
A view of an ask log entry for query results (same fields as `AskLogEntry`).

#### `LogEntry`

**Variants**

* `Timeline(TimelineLogEntry)` – a timeline entry.
* `Ask(AskLogView)` – an ask log entry.

**Serialization**

Tagged JSON with `type` discriminant:

```json
{"type": "timeline", "branch": "main", "timestamp": "...", ...}
{"type": "ask", "branch": "main", "timestamp": "...", ...}
```

#### `LogQueryResult`

**Fields**

* `entries: Vec<LogEntry>` – matching entries, sorted newest-first.

### 6.6 Release Types (Phase 6.3)

#### `ReleaseRange`

**Purpose**
Specifies the revision range for gathering release entries.

**Fields**

* `from: Option<RevisionId>` – starting revision (exclusive). If None, starts from beginning.
* `to: Option<RevisionId>` – ending revision (inclusive). If None, ends at HEAD.

#### `ReleaseEntryKind`

**Purpose**
The kind of a release entry, parsed from Conventional Commits.

**Variants**

* `Feat` – new feature (`feat:`)
* `Fix` – bug fix (`fix:`)
* `Docs` – documentation changes (`docs:`)
* `Style` – code style changes (`style:`)
* `Refactor` – code refactoring (`refactor:`)
* `Perf` – performance improvements (`perf:`)
* `Test` – tests (`test:`)
* `Build` – build system changes (`build:`)
* `Ci` – CI configuration (`ci:`)
* `Chore` – chores (`chore:`)
* `Revert` – reverts (`revert:`)
* `Other` – other/unknown type

**Serialization**

Lowercase string in JSON (e.g., `"feat"`, `"fix"`, `"other"`).

#### `ReleaseEntry`

**Purpose**
A single entry in the release changelog.

**Fields**

* `kind: ReleaseEntryKind` – the kind of change.
* `scope: Option<String>` – optional scope (e.g., "cli", "core").
* `description: String` – the description text.
* `breaking: bool` – whether this is a breaking change.
* `revision_id: RevisionId` – the revision ID this entry came from.
* `timestamp: DateTime<Utc>` – the timestamp of the revision.
* `source_count: usize` – number of sources indexed in this commit.
* `bases: Vec<String>` – bases touched by this commit.

**Invariants**

* `breaking` is true if the commit message contains `!` before the colon (e.g., `feat!:`).
* `scope` is parsed from parentheses (e.g., `feat(cli):` has scope `"cli"`).

#### `ReleaseGroup`

**Purpose**
A group of entries of the same kind for rendering.

**Fields**

* `kind: ReleaseEntryKind` – the kind of entries in this group.
* `label: String` – the display label (e.g., "Features", "Bug Fixes").
* `entries: Vec<ReleaseEntry>` – entries in this group.

#### `ReleaseSummary`

**Purpose**
Summary of a release, containing grouped entries.

**Fields**

* `branch: String` – the branch this release covers.
* `from_revision: Option<RevisionId>` – the starting revision (exclusive), or None if from beginning.
* `to_revision: Option<RevisionId>` – the ending revision (inclusive), or None if to HEAD.
* `total_entries: usize` – total number of entries.
* `groups: Vec<ReleaseGroup>` – entries grouped by kind.
* `dry_run: bool` – whether this was a dry run.

#### `ReleaseOptions`

**Purpose**
Options for the release command.

**Fields**

* `tag: Option<String>` – optional release tag (e.g., "v1.0.0"). Used as heading in CHANGELOG.
* `branch: Option<String>` – branch to generate release for (defaults to current branch).
* `range: ReleaseRange` – revision range to include.
* `dry_run: bool` – dry run: report what would be written without actually writing.

#### `ReleaseResult`

**Purpose**
Result of the release command.

**Fields**

* `tag: String` – the tag that was used for the release heading.
* `changelog_path: Option<String>` – path to the generated CHANGELOG.md (if not dry run).
* `summary: ReleaseSummary` – the release summary.

**Invariants**

* `changelog_path` is None when `dry_run` is true.
* Release does **not** mutate the timeline (no `RevisionOperation::Release` is appended).

---

## 7. Asking & Context Bundles

### 7.1 `AskContextBundle`

**Purpose**
Canonical output of `gik ask`, used by external tools (Guided CLI, agents, IDEs).

**Fields**

* `revision_id: RevisionId` – knowledge revision used for this query.
* `question: String` – original question string.
* `bases: Vec<BaseName>` – bases consulted.
* `rag_chunks: Vec<RagChunk>` – retrieved chunks for RAG.
* `kg_results: Vec<AskKgResult>` – KG subgraphs relevant to the query.
* `memory_events: Vec<MemoryEvent>` – relevant memory events.
* `stack_summary: Option<StackSummary>` – optional inventory summary.
* `debug: AskDebugInfo` – technical metadata.

**Invariants**

* `revision_id` must be an existing revision in the current branch.
* `bases` must reflect all bases used during retrieval.

### 7.2 `RagChunk`

**Fields**

* `base: BaseName` – `"code"` or `"docs"`.
* `score: f32` – similarity / relevance score.
* `path: String` – file path.
* `start_line: u32`.
* `end_line: u32`.
* `snippet: String` – text snippet.

**Invariants**

* `0.0 <= score <= 1.0` is recommended but not strictly required.
* `start_line <= end_line`.

### 7.3 `AskKgResult`

**Purpose**
A rich subgraph returned by KG-aware ask queries. Replaces the placeholder `KgResult`.

**Fields**

| Field | Type | Description |
|-------|------|-------------|
| `reason` | `String` | Why this subgraph was included (e.g., `"matched RAG chunk: src/api/route.ts"`) |
| `rootNodeIds` | `Vec<String>` | Entry point node IDs where BFS traversal began |
| `nodes` | `Vec<KgNode>` | All nodes in this subgraph |
| `edges` | `Vec<KgEdge>` | All edges in this subgraph |

**Example (JSON)**

```json
{
  "reason": "matched RAG chunk: src/api/route.ts",
  "rootNodeIds": ["file:src/api/route.ts"],
  "nodes": [
    {"id": "file:src/api/route.ts", "kind": "file", "label": "src/api/route.ts", ...},
    {"id": "endpoint:/api/users", "kind": "endpoint", "label": "/api/users", ...}
  ],
  "edges": [
    {"from": "file:src/api/route.ts", "to": "endpoint:/api/users", "kind": "definesEndpoint", ...}
  ]
}
```

**Invariants**

* Each `AskKgResult` represents a single connected subgraph.
* `nodes` and `edges` are bounded by `KgQueryConfig` limits.
* `reason` should be human-readable and explain the relevance.

### 7.4 `KgQueryConfig`

**Purpose**
Configuration for KG-aware ask queries. Controls traversal limits and heuristics.

**Fields**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `maxSubgraphs` | `usize` | 3 | Maximum number of subgraphs to return |
| `maxNodesPerSubgraph` | `usize` | 32 | Maximum nodes in each subgraph |
| `maxEdgesPerSubgraph` | `usize` | 48 | Maximum edges in each subgraph |
| `maxHops` | `usize` | 2 | Maximum BFS depth from root nodes |
| `endpointHeuristics` | `bool` | true | Enable endpoint detection heuristics |

**Invariants**

* Defaults are tuned to keep LLM context reasonable (~5-10K tokens for KG).
* Higher limits increase context size but may improve relevance.

### 7.5 `StackSummary`

**Fields**

* `languages: Vec<String>` – detected programming languages.
* `frameworks: Vec<String>` – detected frameworks (e.g., "Next.js", "React").
* `services: Vec<String>` – high-level service names, if detected.
* `managers: Vec<String>` – package managers in use (e.g., "cargo", "npm").
* `total_files: Option<u64>` – total file count in the project.

**Purpose**

* Gives a quick **project fingerprint** to LLMs and tools.

### 7.5 `AskDebugInfo`

**Fields**

* `embedding_model_id: String` – the embedding model used for the query.
* `used_bases: Vec<String>` – the bases that were actually searched.
* `per_base_counts: Vec<AskBaseCount>` – per-base result counts.
* `embed_time_ms: Option<u64>` – query embedding time in milliseconds.
* `search_time_ms: Option<u64>` – vector search time in milliseconds.

**Purpose**

* Helps debugging retrieval behavior and ensuring the correct model is being used.
* Provides timing information for performance analysis.

### 7.6 `AskBaseCount`

**Fields**

* `base: String` – base name.
* `count: usize` – number of results returned from this base.

---

## 8. Engine & Index Contracts

### 8.1 `VectorIndex` (trait)

**Methods**

* `add_embeddings(base, vectors, metadata)` – add new vectors.
* `search(base, query, top_k)` – search vectors for a base.
* `rebuild(base, entries)` – rebuild index from scratch for a base.

**Invariants**

* `search` must never return more than `top_k` entries.
* `metadata` must include enough information to reconstruct `RagChunk` (path, lines, snippet or pointer).

### 8.2 `GikEngine`

**Key Responsibilities**

* Interpret configs and resolve active embedding profile.
* Orchestrate stack scans, staging, commit, ask, reindex, log, release.
* Guarantee invariants on-disk (directory creation, consistent HEAD, valid timeline).

**Contract with CLI**

* All public methods return `Result<_, anyhow::Error>`.
* Engine must not perform CLI formatting; it returns typed structs for the CLI to render.

---

## 9. Contract Stability & Evolution

* All entities in this document are considered **public contracts** for GIK v0.x.

* Structural changes (e.g. renaming fields, changing meanings) require either:

  * a clear **migration path** (e.g. `gik migrate`), or
  * versioned formats (e.g. `version` field inside key JSON files).

* Backwards-compatible extensions are allowed:

  * adding optional fields to JSON.
  * adding new variants in enums that are consumed in a tolerant way.
  * adding new bases that follow the base directory schema.

---

End of `entities.md` v0.2.
