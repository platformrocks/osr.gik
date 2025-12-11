# Guided Indexing Kernel (GIK) – Data Contracts

> Version: v0.2

This document defines the **on-disk data contracts** for GIK. It focuses on:

* File locations and formats (JSON / JSONL).
* Required and optional fields.
* Backward-compatibility rules.

It complements `2-ENTITIES.md` (in-memory types) by describing how those entities are serialized and stored.

---

## 1. Conventions

### 1.1 Formats

* **JSON** – structured config or stats (`*.json`).
* **JSONL** – log or collection of independent entries (one JSON object per line, `*.jsonl`).

### 1.2 Baseline Rules

* All JSON/JSONL files must be **UTF-8**.
* New fields may be added as **optional** to keep backward compatibility.
* Removing or renaming fields **requires migration** or versioning.

---

## 2. Global Config – `~/.gik/config.yaml`

**Path**
`$HOME/.gik/config.yaml`

**Format**
YAML

```yaml
# Example ~/.gik/config.yaml
embeddings:
  default:
    provider: candle
    model_id: sentence-transformers/all-MiniLM-L6-v2
    dimension: 384
    max_tokens: 512
    local_path: ~/.gik/models/embeddings/all-MiniLM-L6-v2
  bases:
    docs:
      provider: ollama
      model_id: nomic-embed-text
      dimension: 768
```

**Fields**

* `embeddings: object` – optional (defaults to Candle with MiniLM-L6-v2).

  * `default: object` – default embedding configuration for all bases.
  * `bases: object` – per-base overrides keyed by base name.

**EmbeddingConfigOverride**

* `provider: string` – optional (`"candle"`, `"ollama"`, etc.).
* `model_id: string` – optional, model identifier.
* `dimension: number` – optional, positive integer (embedding vector dimension).
* `max_tokens: number` – optional, maximum input tokens per chunk.
* `local_path: string` – optional; local path to model files (Candle).

**Defaults**

If the file does not exist, GIK uses sensible defaults:
* Provider: `candle`
* Model ID: `sentence-transformers/all-MiniLM-L6-v2`
* Dimension: `384`
* Max Tokens: `512`
* Local Path: `~/.gik/models/embeddings/all-MiniLM-L6-v2`

**Compatibility**

* Per-base overrides may be added freely.
* Changing default config is allowed but may require `gik reindex` for existing bases.

---

## 3. Project Config – `.guided/knowledge/config.yaml`

**Path**
`<workspace>/.guided/knowledge/config.yaml`

**Format**
YAML

```yaml
# Example .guided/knowledge/config.yaml
embeddings:
  bases:
    code:
      provider: candle
      model_id: BAAI/bge-small-en-v1.5
      dimension: 384
```

**Fields**

* `embeddings: object` – optional; same structure as global config.

**Defaults**

If the file does not exist or the field is omitted, GIK falls back to `GlobalConfig.embedding.default_profile`.

**Compatibility**

* New top-level keys may be added in future versions; existing code should ignore unknown fields.

---

## 4. Branch Structure

**Path root**
`<workspace>/.guided/knowledge/<branch>/`

Within each branch:

```text
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
bases/
  code/
    sources.jsonl
    stats.json
    model-info.json
    index/
  docs/
    sources.jsonl
    stats.json
    model-info.json
    index/
  memory/
    sources.jsonl
    stats.json
    model-info.json
    config.json
    archive.jsonl
    index/
kg/ (Phase 9+)
```

**Global paths (branch-agnostic):**

```text
<workspace>/.guided/knowledge/
  asks/
    ask.log.jsonl      # Ask query history (Phase 6.1)
```

### 4.0 Initialization (`gik init`)

Running `gik init` creates the branch directory structure with:

| File/Directory | Created on Init | Contents |
| -------------- | --------------- | -------- |
| `HEAD` | Yes | Revision ID of the `Init` revision |
| `timeline.jsonl` | Yes | Single `Init` revision entry |
| `staging/` | Yes (empty) | Directory for pending sources |
| `stack/` | Yes (empty) | Directory for stack scan results |
| `bases/` | Yes | Directory for knowledge bases |
| `bases/code/` | Yes (empty) | Directory for code base |
| `bases/docs/` | Yes (empty) | Directory for docs base |
| `bases/memory/` | Yes (empty) | Directory for memory base |

**Idempotency**

* `gik init` is **idempotent**: calling it on an already-initialized branch is safe.
* On first call: creates directories, writes `HEAD` and `timeline.jsonl`.
* On subsequent calls: detects existing `HEAD` and returns `AlreadyInitialized` error (handled gracefully by CLI as informational message).
* Timeline is **append-only**: no duplicate `Init` revisions are created on repeat calls.

**Example after `gik init`:**

```text
.guided/
  knowledge/
    main/
      HEAD                 # "3f4e9c0a-1b23-4d5e-8abc-1234567890ab"
      timeline.jsonl       # Single Init revision
      staging/             # Pending sources directory
      stack/               # Stack scan results
      bases/
        code/              # Code knowledge base
        docs/              # Documentation knowledge base
        memory/            # Memory knowledge base
```

### 4.1 `HEAD`

**Path**
`<branch>/HEAD`

**Format**
Plain text – a single line with the current `RevisionId`.

```text
3f4e9c0a-1b23-4d5e-8abc-1234567890ab
```

**Contract**

* Must always reference a revision present in `timeline.jsonl`.
* Created by `gik init` with the ID of the initial `Init` revision.
* Updated after `gik commit`, `gik reindex`.

### 4.2 `timeline.jsonl`

**Path**
`<branch>/timeline.jsonl`

**Format**
JSONL – one `Revision` object per line.

```jsonc
{"id":"3f4e9c0a-1b23-4d5e-8abc-1234567890ab","parentId":null,"branch":"main","gitCommit":null,"timestamp":"2025-11-27T00:00:00Z","message":"Init GIK","operations":[{"type":"Init"}]}
{"id":"a1b2c3d4","parentId":"3f4e9c0a-1b23-4d5e-8abc-1234567890ab","branch":"main","gitCommit":"abc123","timestamp":"2025-11-27T01:00:00Z","message":"Index src/","operations":[{"type":"Commit","bases":["code"],"sourceCount":42}]}
```

**Fields – `Revision`**

* `id: string` – required.
* `parentId: string | null` – required.
* `branch: string` – required.
* `gitCommit: string | null` – optional.
* `timestamp: string` – required, ISO 8601 UTC.
* `message: string` – required.
* `operations: RevisionOperation[]` – required.

**Fields – `RevisionOperation`**

Common field:

* `type: string` – discriminant.

Variants:

1. `Init`

   ```jsonc
   {"type":"Init"}
   ```

2. `Commit`

   ```jsonc
   {"type":"Commit","bases":["code","docs"],"sourceCount":42}
   ```

   * `bases: string[]` – required.
   * `sourceCount: number` – required.

3. `MemoryIngest`

   ```jsonc
   {"type":"MemoryIngest","count":5}
   ```

   * `count: number` – required, number of memory entries ingested.

4. `MemoryPrune`

   ```jsonc
   {"type":"MemoryPrune","count":10,"archivedCount":7,"deletedCount":3}
   ```

   * `count: number` – required, total entries pruned.
   * `archivedCount: number` – required, entries moved to archive.
   * `deletedCount: number` – required, entries permanently deleted.

5. `Reindex`

   ```jsonc
   {"type":"Reindex","base":"code","fromModelId":"old-model","toModelId":"new-model"}
   ```

   * `base: string` – required.
   * `fromModelId: string` – required.
   * `toModelId: string` – required.

6. `Release` (reserved for future use)

   ```jsonc
   {"type":"Release","tag":"v0.1.0"}
   ```

   * `tag: string | null` – optional.
   * **Note**: Current `gik release` command is read-only and does NOT add revisions.
     This operation type is reserved for future timeline-mutating releases.

7. `Custom`

   ```jsonc
   {"type":"Custom","name":"my-operation","data":{"key":"value"}}
   ```

   * `name: string` – required, operation name.
   * `data: object | null` – optional, arbitrary JSON data.

**Compatibility**

* New operation types may be added with new `type` values.
* Consumers must ignore operations with unknown `type` when possible.

### 4.3 `asks/ask.log.jsonl` (Phase 6.1)

**Path**
`<workspace>/.guided/knowledge/asks/ask.log.jsonl`

This file is branch-agnostic and stores the history of all `gik ask` queries.

**Format**
JSONL – `AskLogEntry` objects.

```jsonc
{"timestamp":"2024-01-15T10:30:00Z","branch":"main","question":"How does the API work?","bases":["code","docs"],"totalHits":8,"bundlePath":null}
{"timestamp":"2024-01-15T10:25:00Z","branch":"main","question":"What is the main entry point?","bases":["code"],"totalHits":5}
```

**Fields – `AskLogEntry`**

* `timestamp: string` – required, ISO 8601 UTC. When the query was executed.
* `branch: string` – required. The branch context used for the query.
* `question: string` – required. The original question string.
* `bases: string[]` – required. The bases that were queried.
* `totalHits: number` – required. Total number of RAG chunks returned.
* `bundlePath: string | null` – optional. Path to persisted full context bundle (future).

**Behavior**

* Every successful `gik ask` execution appends one entry.
* The directory `asks/` is created automatically on first ask.
* Failed queries (e.g., no indexed bases) do not create entries.

**Compatibility**

* New fields may be added in future versions; consumers must ignore unknown fields.

---

## 5. Stack Base – `stack/`

**Path**
`<branch>/stack/`

### How `gik add` Affects Stack

When `gik add` is executed with at least one valid target:

1. **Full rescan** – The entire workspace is rescanned, not just the added targets.
2. **Overwrite** – All stack files (`files.jsonl`, `dependencies.jsonl`, `tech.jsonl`, `stats.json`) are completely regenerated.
3. **Rationale** – Full rescan ensures correctness. Future versions may optimize to incremental updates.

### 5.1 `files.jsonl`

**Format**
JSONL – `StackFileEntry` objects.

```jsonc
{"path":"src/main.rs","kind":"File","languages":["rust"],"fileCount":null}
{"path":"src","kind":"Dir","languages":["rust"],"fileCount":10}
```

**Fields – `StackFileEntry`**

* `path: string` – required.
* `kind: string` – required (`"Dir"` or `"File"`).
* `languages: string[]` – required (may be empty).
* `fileCount: number | null` – optional, only for directories.

### 5.2 `dependencies.jsonl`

**Format**
JSONL – `StackDependencyEntry` objects.

```jsonc
{"manager":"cargo","name":"serde","version":"1.0","scope":"runtime","manifestPath":"Cargo.toml"}
```

**Fields – `StackDependencyEntry`**

* `manager: string` – required.
* `name: string` – required.
* `version: string` – required.
* `scope: string` – required.
* `manifestPath: string` – required, relative.

### 5.3 `tech.jsonl`

**Format**
JSONL – `StackTechEntry` objects.

```jsonc
{"kind":"framework","name":"Next.js","source":"dependency:next","confidence":0.9}
```

**Fields – `StackTechEntry`**

* `kind: string` – required.
* `name: string` – required.
* `source: string` – required.
* `confidence: number` – required.

### 5.4 `stats.json`

**Format**
JSON – `StackStats`.

```jsonc
{
  "totalFiles": 120,
  "languages": {"rust": 50, "typescript": 70},
  "managers": ["cargo", "npm"],
  "generatedAt": "2025-11-27T01:10:00Z"
}
```

**Fields – `StackStats`**

* `totalFiles: number` – required.
* `languages: object` – required.
* `managers: string[]` – required.
* `generatedAt: string` – required, ISO 8601.

---

## 6. Staging – `staging/`

**Path**
`<branch>/staging/`

The staging area tracks pending sources that are queued for ingestion into knowledge bases.

### How `gik add` Affects Staging

When `gik add` is executed:

1. **Appends to `pending.jsonl`** – Each valid target creates a new `PendingSource` entry.
2. **Recomputes `summary.json`** – Summary is regenerated from `pending.jsonl` after each addition.
3. **Duplicate detection** – Sources with the same `(branch, base, uri)` that are already pending are skipped.
4. **Append-only** – `pending.jsonl` is append-only; existing entries are never modified by `add`.

### 6.1 `pending.jsonl`

**Format**
JSONL – `PendingSource` objects.

```jsonc
{"id":"f47ac10b-58cc-4372-a567-0e02b2c3d479","branch":"main","base":"code","kind":"filePath","uri":"src/main.rs","addedAt":"2025-11-27T18:00:00Z","status":"pending"}
{"id":"a1b2c3d4-5678-90ab-cdef-1234567890ab","branch":"main","base":"docs","kind":"url","uri":"https://docs.example.com/api","addedAt":"2025-11-27T18:01:00Z","status":"pending"}
```

**Fields – `PendingSource`**

* `id: string` – required, UUID.
* `branch: string` – required.
* `base: string` – required (`"code"`, `"docs"`, `"memory"`, `"kg"`).
* `kind: string` – required (`"filePath"`, `"directory"`, `"url"`, `"archive"`, `"other"`).
* `uri: string` – required, normalized path or URL.
* `addedAt: string` – required, ISO 8601.
* `status: string` – required (`"pending"`, `"processing"`, `"indexed"`, `"failed"`).
* `lastError: string` – optional, error message if status is `"failed"`.
* `metadata: object` – optional extensibility.

**Lifecycle**

1. Created by `gik add` (or engine's `add_pending_source`).
2. Status changes to `processing` during commit.
3. Status changes to `indexed` on success, `failed` on error.
4. Processed sources remain in history for audit; future versions may prune them.

**Phase 4.3 Status Transitions**

During `gik commit`:

| Source Kind | Status Transition | Notes |
|-------------|------------------|-------|
| `filePath` | `pending` → `indexed` | Successfully embedded and indexed |
| `directory` | `pending` → `indexed` | Directory contents processed |
| `url` | `pending` → `failed` | Reason: `"URL sources not supported in Phase 4.3"` |
| `archive` | `pending` → `failed` | Reason: `"Archive sources not supported in Phase 4.3"` |
| Large file (>1MB or >10k lines) | `pending` → `failed` | Reason: `"File too large (...)"` |

**Embedding Model Requirement**

If the embedding model is not available, `gik commit` fails entirely with
`GikError::EmbeddingProviderUnavailable`. The error message includes instructions
to clone the model from Hugging Face. There is no silent fallback to a mock
backend at runtime.

### 6.2 `summary.json`

**Format**
JSON – `StagingSummary`.

```json
{
  "pendingCount": 5,
  "indexedCount": 10,
  "failedCount": 1,
  "byBase": {
    "code": 3,
    "docs": 2
  },
  "lastUpdatedAt": "2025-11-27T18:05:00Z"
}
```

**Fields – `StagingSummary`**

* `pendingCount: number` – required, sources with status `pending` or `processing`.
* `indexedCount: number` – required.
* `failedCount: number` – required.
* `byBase: object` – required, pending count per base.
* `lastUpdatedAt: string` – required, ISO 8601.

**Recomputation**

* `summary.json` is recomputed from `pending.jsonl` whenever sources are added or status changes.
* If `summary.json` is missing, it can be regenerated by scanning `pending.jsonl`.

---

## 7. Code & Docs Bases – `bases/code/` and `bases/docs/`

**Path**
`<branch>/bases/code/` and `<branch>/bases/docs/`

### 7.1 `sources.jsonl`

**Format**
JSONL – `BaseSourceEntry` objects.

```jsonc
{
  "id": "chunk-001",
  "base": "code",
  "path": "src/main.rs",
  "startLine": 1,
  "endLine": 40,
  "text": "fn main() { ... }",
  "extra": {"language": "rust"}
}
```

**Fields – `BaseSourceEntry`**

* `id: string` – required.
* `base: string` – required (`"code"` or `"docs"`).
* `path: string` – required, relative.
* `startLine: number` – required.
* `endLine: number` – required.
* `text: string` – optional but recommended.
* `extra: object` – optional metadata.

### 7.2 `stats.json`

**Format**
JSON – `BaseStats`.

```jsonc
{
  "chunkCount": 420,
  "fileCount": 30,
  "lastUpdated": "2025-11-27T01:20:00Z"
}
```

**Fields – `BaseStats`**

* `chunkCount: number` – required.
* `fileCount: number` – required.
* `lastUpdated: string` – required.

### 7.3 `model-info.json`

**Format**
JSON – `ModelInfo`.

```jsonc
{
  "provider": "candle",
  "modelId": "sentence-transformers/all-MiniLM-L6-v2",
  "dimension": 384,
  "createdAt": "2025-11-27T01:20:00Z",
  "lastReindexedAt": null
}
```

**Fields – `ModelInfo`**

* `provider: string` – required (`"candle"`, `"ollama"`, etc.).
* `modelId: string` – required, model identifier.
* `dimension: number` – required, positive integer.
* `createdAt: string` – required, ISO 8601 UTC timestamp.
* `lastReindexedAt: string | null` – optional, ISO 8601 UTC timestamp of last reindex.

**Usage**

* Created on first `gik commit` that indexes the base.
* Updated on `gik reindex` when model changes.
* Read by `gik ask` and `gik status` to check compatibility.

**Compatibility Check**

When comparing active `EmbeddingConfig` against stored `ModelInfo`:

* **Compatible**: `provider` and `modelId` match.
* **MissingModelInfo**: file doesn't exist (fresh base or legacy).
* **Mismatch**: different provider or model; reindex recommended.

### 7.4 `index/`

**Path**
`<base>/index/`

**Structure**

```text
<base>/index/
├── meta.json         # VectorIndexMeta – required
└── records.jsonl     # VectorInsert records – backend-specific (SimpleFile)
```

The `index/` directory stores per-base vector data. Each backend implementation decides on internal storage format, but **must** use this directory as its root and **must** write/read `meta.json` for compatibility.

---

#### 7.4.1 `meta.json`

**Format**
JSON – `VectorIndexMeta` object.

```jsonc
{
  "backend": "simple_file",
  "metric": "cosine",
  "dimension": 384,
  "base": "code",
  "embeddingProvider": "candle",
  "embeddingModelId": "bge-small-en-v1.5",
  "createdAt": "2025-11-28T12:00:00Z",
  "updatedAt": "2025-11-28T14:30:00Z"
}
```

**Fields – `VectorIndexMeta`**

| Field              | Type             | Required | Description                                      |
| ------------------ | ---------------- | -------- | ------------------------------------------------ |
| `backend`          | string           | Yes      | `"simple_file"`, `"lancedb"`, or `"other:<id>"`. |
| `metric`           | string           | Yes      | `"cosine"`, `"dot"`, or `"l2"`.                  |
| `dimension`        | number           | Yes      | Embedding dimension (e.g., `384`).               |
| `base`             | string           | Yes      | Base name (e.g., `"code"`, `"docs"`).            |
| `embeddingProvider`| string           | Yes      | Provider kind used to generate embeddings.       |
| `embeddingModelId` | string           | Yes      | Model ID used to generate embeddings.            |
| `createdAt`        | string           | Yes      | ISO 8601 UTC timestamp of index creation.        |
| `updatedAt`        | string           | Yes      | ISO 8601 UTC timestamp of last modification.     |

**Invariants**

* `dimension` must match the embedding model's output dimension.
* `embeddingProvider` and `embeddingModelId` must match current `EmbeddingConfig` for compatibility.
* `backend` and `metric` should match current `VectorIndexConfig` or user will be warned.

**Usage**

* Created on first `gik commit` that indexes the base.
* Updated (`updatedAt`) on every upsert or delete operation.
* Read by `gik ask`, `gik status`, and `gik reindex` to check compatibility.

---

#### 7.4.2 `records.jsonl` (SimpleFile backend)

**Format**
JSONL – `VectorInsert` objects. One record per line.

```jsonc
{"id":1,"embedding":[0.123,-0.456,0.789,...],"payload":{"chunk_id":"chunk-001","file":"src/main.rs"}}
{"id":2,"embedding":[0.321,-0.654,0.987,...],"payload":{"chunk_id":"chunk-002","file":"src/lib.rs"}}
```

**Fields – `VectorInsert`**

| Field      | Type         | Required | Description                                              |
| ---------- | ------------ | -------- | -------------------------------------------------------- |
| `id`       | number (u64) | Yes      | Unique vector ID within this index.                      |
| `embedding`| number[]     | Yes      | Float32 vector of length `dimension` from `meta.json`.   |
| `payload`  | object       | Yes      | Arbitrary JSON payload (chunk metadata, file info, etc.).|

**Invariants**

* Each `id` is unique within `records.jsonl`.
* `embedding.length` must equal `dimension` from `meta.json`.
* Records are append-only during normal operation; delete marks require rewrite on flush.

**Usage**

* Written on `gik commit` (upsert vectors).
* Read on `gik ask` (load into memory for search).
* Rewritten on `gik reindex` or when deletes require compaction.

> **Note:** The `records.jsonl` format is specific to the `SimpleFile` backend. Other backends (e.g., `LanceDb`) use their own internal storage format within the `index/` directory but must still maintain `meta.json`.

---

#### 7.4.3 LanceDB Backend (`lancedb`)

When `backend` is `"lancedb"`, the `index/` directory contains a LanceDB database instead of `records.jsonl`.

**Structure**

```text
<base>/index/
├── meta.json                    # VectorIndexMeta (backend: "lancedb")
└── vectors.lance/               # LanceDB database directory
    ├── _versions/               # Version manifests
    ├── _indices/                # Index structures (IVF_PQ, etc.)
    ├── data/                    # Columnar data files
    └── _transactions/           # Transaction logs
```

**LanceDB Table Schema**

The `vectors` table uses the following Arrow schema:

| Column         | Arrow Type                          | Nullable | Description                              |
| -------------- | ----------------------------------- | -------- | ---------------------------------------- |
| `id`           | `UInt64`                            | No       | Unique vector ID.                        |
| `vector`       | `FixedSizeList[Float32, dimension]` | No       | Embedding vector.                        |
| `payload`      | `Utf8`                              | No       | JSON-encoded chunk metadata.             |
| `base`         | `Utf8`                              | No       | Knowledge base name (code, docs, memory).|
| `branch`       | `Utf8`                              | Yes      | Branch name.                             |
| `source_type`  | `Utf8`                              | No       | Source kind (file, memory, url, archive).|
| `path`         | `Utf8`                              | Yes      | File path or logical key.                |
| `tags`         | `List[Utf8]`                        | Yes      | User-defined tags for filtering.         |
| `revision_id`  | `Utf8`                              | Yes      | Timeline revision ID.                    |
| `created_at`   | `Timestamp(Microsecond, UTC)`       | No       | Record creation time.                    |
| `updated_at`   | `Timestamp(Microsecond, UTC)`       | No       | Last modification time.                  |

**Filtering Support**

LanceDB supports SQL-like predicates for filtered queries:

```sql
-- Filter by base
base = 'code'

-- Filter by source type
source_type = 'file'

-- Filter by path prefix
path LIKE 'src/%'

-- Combined filters
base = 'code' AND source_type = 'file' AND path LIKE 'src/%'
```

**Features**

* **Approximate Nearest Neighbor (ANN)**: Uses IVF-PQ index for fast similarity search.
* **Rich Metadata**: Full columnar storage enables efficient filtering.
* **Local-first**: No remote server required; data stored on disk.
* **ACID Transactions**: LanceDB provides transactional guarantees.

**Usage**

* **Default backend**: LanceDB is the default backend as of Phase 8.1.
* Created on first `gik commit` when `backend` is `"lancedb"`.
* Supports `query()` with top-k retrieval and `query_filtered()` with predicate.
* Read by `gik ask` for similarity search.

**Backward Compatibility**

When reading an existing index, GIK checks `meta.json` to determine the actual backend:

* If `backend: "simple_file"`, uses `SimpleFileVectorIndex`.
* If `backend: "lancedb"`, uses `LanceDbVectorIndex`.

This ensures existing indexes created with `SimpleFile` continue to work without reindexing.

---

#### 7.4.4 Compatibility Rules

Compatibility between the active embedding configuration, stored model info, and vector index metadata:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          COMPATIBILITY FLOW                                 │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  1. Check Embedding Compatibility (EmbeddingConfig vs ModelInfo)            │
│     ├── model-info.json missing    → MissingMeta (can proceed, new index)   │
│     ├── provider mismatch          → EmbeddingMismatch (reindex required)   │
│     └── modelId mismatch           → EmbeddingMismatch (reindex required)   │
│                                                                             │
│  2. Check Index Compatibility (VectorIndexConfig vs VectorIndexMeta)        │
│     ├── meta.json missing          → MissingMeta (can proceed, new index)   │
│     ├── dimension mismatch         → DimensionMismatch (reindex required)   │
│     ├── backend mismatch           → BackendMismatch (warning, can proceed) │
│     └── metric mismatch            → Compatible (warning, can proceed)      │
│                                                                             │
│  3. Result                                                                  │
│     └── All checks pass            → Compatible (ready for use)             │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Resolution Precedence for VectorIndexConfig**

```
project.indexes.bases[<base>]     (highest priority)
        ↓ fallback
global.indexes.bases[<base>]
        ↓ fallback
global.indexes.default
        ↓ fallback
hardcoded (LanceDb + Cosine)      (lowest priority)
```

---

## 8. Memory Base – `bases/memory/`

**Path**
`.guided/knowledge/<branch>/bases/memory/`

The memory base stores high-level, human-readable knowledge about the project. Unlike `code` and `docs` bases which index raw files, memory entries are structured records that capture contextual knowledge.

**Directory Layout**

```
memory/
├── sources.jsonl    # MemoryEntry records
├── model-info.json  # Embedding model metadata
├── stats.json       # Aggregated statistics
└── index/           # Vector index files
    ├── meta.json
    └── records.jsonl
```

### 8.1 `sources.jsonl`

**Format**
JSONL – `MemoryEntry` objects (same role as `BaseSourceEntry` for code/docs).

```jsonc
{
  "id": "mem-a1b2c3d4e5f6",
  "createdAt": "2025-01-15T10:30:00Z",
  "updatedAt": "2025-01-15T10:30:00Z",
  "scope": "project",
  "source": "decision",
  "title": "Use GIK for knowledge indexing",
  "text": "Decided to adopt GIK as the primary knowledge indexing tool for this project. GIK provides local-first embedding and vector search capabilities that align with our privacy requirements.",
  "tags": ["architecture", "tooling"],
  "branch": null,
  "originRevision": "rev-12345"
}
```

**Fields – `MemoryEntry`**

* `id: string` – required, unique memory ID (hash-based).
* `createdAt: string` – required, ISO 8601 UTC timestamp.
* `updatedAt: string` – required, ISO 8601 UTC timestamp.
* `scope: string` – required, one of `"project"`, `"branch"`, `"global"`.
* `source: string` – required, one of `"manualNote"`, `"decision"`, `"observation"`, `"externalReference"`, `"agentGenerated"`, `"commitContext"`.
* `title: string` – optional, short summary (≤100 chars). Omitted if empty.
* `text: string` – required, full content.
* `tags: string[]` – required (can be empty).
* `branch: string | null` – optional, set if `scope == "branch"`.
* `originRevision: string | null` – optional, revision ID where entry was created.
* `importance: number | null` – optional, relevance score [0.0–1.0].

**Invariants**

* `id` must be unique per workspace.
* `title` should be ≤100 characters.
* `scope` and `source` values are case-sensitive (camelCase).

### 8.2 `stats.json`

**Format**

```jsonc
{
  "base": "memory",
  "chunkCount": 15,
  "fileCount": 15,
  "vectorCount": 15,
  "failedCount": 0,
  "lastUpdated": "2025-01-15T11:00:00Z"
}
```

**Fields**

* `base: string` – required, always `"memory"`.
* `chunkCount: number` – required, number of memory entries.
* `fileCount: number` – required, same as chunkCount for memory (1:1).
* `vectorCount: number` – required, number of vectors in index.
* `failedCount: number` – required, entries that failed embedding.
* `lastUpdated: string` – required, ISO 8601 UTC timestamp.

### 8.3 `model-info.json`

Same contract as `code/docs` bases. Required for vector search.

```jsonc
{
  "provider": "candle",
  "modelId": "sentence-transformers/all-MiniLM-L6-v2",
  "dimension": 384,
  "maxTokens": 512,
  "createdAt": "2025-01-15T10:25:00Z"
}
```

### 8.4 `index/`

Same structure as `code/docs` bases. Contains:

* `meta.json` – Vector index metadata.
* `records.jsonl` – Vector records mapping IDs to vectors.

### 8.5 `config.json` (Pruning Policy)

**Format**
JSON – Memory base configuration including pruning policy.

```jsonc
{
  "pruningPolicy": {
    "maxEntries": 1000,
    "maxEstimatedTokens": 100000,
    "maxAgeDays": 365,
    "obsoleteTags": ["deprecated", "obsolete"],
    "mode": "archive"
  }
}
```

**Fields – `MemoryPruningPolicy`**

* `maxEntries: number` – optional, maximum number of entries to keep.
* `maxEstimatedTokens: number` – optional, maximum estimated token count.
* `maxAgeDays: number` – optional, maximum age in days before pruning.
* `obsoleteTags: string[]` – optional, tags that mark entries for pruning.
* `mode: string` – optional, one of `"delete"` or `"archive"` (default: `"archive"`).

**Pruning Behavior**

* Entries are pruned if they match ANY of the criteria (OR logic).
* If `maxEntries` or `maxEstimatedTokens` is exceeded, oldest entries are pruned first.
* If `mode == "archive"`, pruned entries are moved to `archive.jsonl`.
* If `mode == "delete"`, pruned entries are permanently removed.
* Archived entries are NOT searchable (removed from vector index).

### 8.6 `archive.jsonl`

**Format**
JSONL – Archived `BaseSourceEntry` objects (same schema as `sources.jsonl`).

Created when pruning with `mode: "archive"`. Contains entries removed from the active index but preserved for audit purposes.

**Compatibility**

* Memory base follows the same embedding/index pipeline as code/docs.
* Memory entries are retrieved via vector search in `gik ask` when `memory` is in the bases list.

---

## 9. Knowledge Graph Base – `kg/` (Phase 9.1+)

The Knowledge Graph (KG) stores structural relationships **derived** from code and documentation bases.
The KG is NOT a primary source of truth – it is always derivable from base sources.

**Implementation Status**:
- **Phase 9.1**: Storage infrastructure (entities, JSONL/JSON files, store API) ✅
- **Phase 9.2**: KG extraction from bases (file-level imports), sync on commit/reindex ✅
- **Phase 9.3**: KG-aware ask with subgraph results (pending)
- **Phase 9.4**: KG stats in status/log commands (pending)

**Data Flow**

```
Base Sources (code, docs)
         │
         ▼
   KG Extraction  ──► nodes.jsonl, edges.jsonl
   (file-level)        (full rebuild per sync)
         │
         ▼
      stats.json
```

**Path**

```
.guided/knowledge/<branch>/kg/
├── nodes.jsonl    # KgNode objects, one per line
├── edges.jsonl    # KgEdge objects, one per line
└── stats.json     # KgStats object
```

**Lazy Initialization**

The `kg/` directory is created lazily on first write, NOT during `gik init`.
This keeps KG optional for projects that don't use it.

### 9.1 `nodes.jsonl`

**Format**: JSONL – one `KgNode` JSON object per line.

**Schema**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | `string` | ✓ | Unique node identifier (e.g., `file:src/main.rs`, `fn:lib::parse`) |
| `kind` | `string` | ✓ | Node type: `file`, `module`, `function`, `class`, `struct`, `trait`, `concept`, `dependency`, `service`, `endpoint`, `reactComponent`, `uiComponent`, `ngComponent`, `ngModule`, `ngService`, `ngRoute`, `styleClass`, `styleId`, `cssVariable`, `tailwindDirective`, `htmlTemplate`, `htmlSection`, `htmlAnchor` |
| `label` | `string` | ✓ | Human-readable label |
| `props` | `object` | ✓ | Arbitrary properties (defaults to `{}`) |
| `branch` | `string` | | Optional branch name if branch-specific |
| `createdAt` | `string` | ✓ | ISO 8601 timestamp |
| `updatedAt` | `string` | ✓ | ISO 8601 timestamp |

**ID Conventions**

| Node Type | ID Format | Example |
|-----------|-----------|---------|
| File | `file:<path>` | `file:src/main.rs` |
| Endpoint | `endpoint:<route>` | `endpoint:/api/users` |
| Symbol | `sym:<lang>:<path>:<kind>:<name>[#<idx>]` | `sym:ts:src/utils.ts:function:helper` |
| CSS Symbol | `sym:css:<path>:<kind>:<name>` | `sym:css:styles.css:styleClass:btn` |
| HTML Symbol | `sym:html:<path>:<kind>:<name>` | `sym:html:index.html:htmlTemplate:index` |

**Example**

```json
{"id":"file:src/main.rs","kind":"file","label":"src/main.rs","props":{"language":"rust","lines":150},"branch":"main","createdAt":"2025-11-28T10:00:00Z","updatedAt":"2025-11-28T10:00:00Z"}
{"id":"fn:main","kind":"function","label":"main()","props":{"visibility":"public"},"createdAt":"2025-11-28T10:00:00Z","updatedAt":"2025-11-28T10:00:00Z"}
{"id":"sym:rs:src/lib.rs:function:parse","kind":"function","label":"parse","props":{"language":"rs","definedIn":"src/lib.rs","framework":null},"createdAt":"2025-01-15T10:00:00Z","updatedAt":"2025-01-15T10:00:00Z"}
```

### 9.2 `edges.jsonl`

**Format**: JSONL – one `KgEdge` JSON object per line.

**Schema**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | `string` | ✓ | Unique edge identifier (e.g., `edge:<hash>:<kind>` or custom ID) |
| `from` | `string` | ✓ | Source node ID |
| `to` | `string` | ✓ | Target node ID |
| `kind` | `string` | ✓ | Relationship type: `dependsOn`, `calls`, `contains`, `imports`, `extends`, `implements`, `uses`, `ownedBy`, `relatedTo`, `definesEndpoint`, `defines` |
| `props` | `object` | ✓ | Arbitrary properties (defaults to `{}`) |
| `branch` | `string` | | Optional branch name if branch-specific |
| `createdAt` | `string` | ✓ | ISO 8601 timestamp |
| `updatedAt` | `string` | ✓ | ISO 8601 timestamp |

**Edge Kinds**

| Kind | Description |
|------|-------------|
| `imports` | File imports another file |
| `definesEndpoint` | File defines an API endpoint |
| `defines` | File defines a symbol (function, class, etc.) |
| `dependsOn` | General dependency |
| `calls` | Function/method call |
| `contains` | Containment (module→function) |
| `extends` | Inheritance |
| `implements` | Interface implementation |
| `usesClass` | Component uses CSS class (Phase 9.2.2) |
| `usesUiComponent` | File uses UI component (Phase 9.2.2) |
| `belongsToModule` | Angular component belongs to module (Phase 9.2.2) |

**Example**

```json
{"id":"edge:a1b2c3d4:imports","from":"file:src/main.rs","to":"file:src/lib.rs","kind":"imports","props":{"count":3},"createdAt":"2025-11-28T10:00:00Z","updatedAt":"2025-11-28T10:00:00Z"}
{"id":"edge:e5f6g7h8:calls","from":"fn:main","to":"fn:helper","kind":"calls","props":{"weight":1.5},"createdAt":"2025-11-28T10:00:00Z","updatedAt":"2025-11-28T10:00:00Z"}
{"id":"edge:x9y0z1a2:defines","from":"file:src/lib.rs","to":"sym:rs:src/lib.rs:function:parse","kind":"defines","props":{},"createdAt":"2025-01-15T10:00:00Z","updatedAt":"2025-01-15T10:00:00Z"}
```

### 9.3 `stats.json`

**Format**: JSON – single `KgStats` object (pretty-printed).

**Schema**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `nodeCount` | `integer` | ✓ | Total number of nodes |
| `edgeCount` | `integer` | ✓ | Total number of edges |
| `lastUpdated` | `string` | ✓ | ISO 8601 timestamp |
| `version` | `string` | ✓ | Schema version (e.g., `"kg-v1"`) |

**Example**

```json
{
  "nodeCount": 200,
  "edgeCount": 500,
  "lastUpdated": "2025-11-28T10:00:00Z",
  "version": "kg-v1"
}
```

### 9.4 Compatibility Notes

* **Optional**: Absence of `kg/` indicates no graph built yet (returns empty collections).
* **Read-only defaults**: Reading from non-existent files returns empty vectors/default stats.
* **Version field**: `KgStats.version` enables future schema migrations (check this field before parsing).
* **ID uniqueness**: Both `KgNode.id` and `KgEdge.id` MUST be unique within their files.
* **Edge ID field**: Unlike the original contract, `KgEdge` now includes an `id` field for symmetry with nodes and to support deduplication/updates.

---

## 10. Ask Result – `AskContextBundle` (CLI output)

While not stored in a fixed file by default, `AskContextBundle` is a **JSON contract** used as output of `gik ask`.

**Example output**

```jsonc
{
  "revisionId": "a1b2c3d4",
  "question": "How does the web app call the API?",
  "bases": ["code", "docs"],
  "ragChunks": [
    {
      "base": "code",
      "score": 0.92,
      "path": "src/api/client.rs",
      "startLine": 10,
      "endLine": 40,
      "snippet": "pub async fn call_api(...) { ... }"
    }
  ],
  "kgResults": [
    {
      "reason": "matched RAG chunk: src/api/client.rs",
      "rootNodeIds": ["file:src/api/client.rs"],
      "nodes": [
        {"id": "file:src/api/client.rs", "kind": "file", "label": "src/api/client.rs", "props": {}, "createdAt": "...", "updatedAt": "..."},
        {"id": "endpoint:/api/users", "kind": "endpoint", "label": "/api/users", "props": {"route": "/api/users", "httpMethod": "GET,POST"}, "createdAt": "...", "updatedAt": "..."}
      ],
      "edges": [
        {"id": "edge:...", "from": "file:src/api/client.rs", "to": "endpoint:/api/users", "kind": "definesEndpoint", "props": {}, "createdAt": "...", "updatedAt": "..."}
      ]
    }
  ],
  "memoryEvents": [],
  "stackSummary": {
    "languages": ["rust", "typescript"],
    "frameworks": ["Next.js"],
    "services": ["web", "api"],
    "managers": ["cargo", "npm"]
  },
  "debug": {
    "embeddingModelId": "sentence-transformers/all-MiniLM-L6-v2",
    "usedBases": ["code", "docs"]
  }
}
```

**Fields**

* Mirrors the `AskContextBundle` entity definition in `2-ENTITIES.md`.
* `kgResults` contains `AskKgResult` objects with rich subgraph data (nodes, edges, reason).

**Fields – `RagChunk`**

* `base: string` – required, knowledge base name.
* `score: number` – required, combined relevance score.
* `path: string` – required, file path relative to workspace.
* `startLine: number` – required, 1-based start line.
* `endLine: number` – required, 1-based end line.
* `snippet: string` – required, text content of the chunk.
* `denseScore: number | null` – optional, raw vector similarity score before reranking.
* `rerankerScore: number | null` – optional, reranker model score.

**Compatibility**

* New fields can be added at top-level or inside nested objects as optional.
* Tools consuming this JSON must ignore unknown fields.

---

## 11. Ignore Rules – `.gikignore`

**Path**
`<workspace>/.gikignore`

**Format**
Plain text, gitignore-like patterns.

**Contract**

* Same semantics as `.gitignore` where possible (globs, directories, negations).
* Patterns in `.gikignore` override `.gitignore` when both apply.

---

## 12. Versioning & Migrations

### 12.1 Version Fields

GIK v0.x may operate without explicit version fields in each file, but **future versions** are encouraged to add:

* `version` to key JSON files (e.g. `model-info.json`, `stats.json`).

### 12.2 Migration Strategies

If a breaking change is introduced (field rename, type change):

1. Provide a `gik migrate` command that:

   * reads old formats,
   * writes new formats,
   * updates a migration log.
2. Avoid implicit migrations; always require explicit user action.

---

End of `3-CONTRACTS.md` v0.2.
