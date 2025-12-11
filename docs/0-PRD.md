# Guided Indexing Kernel (GIK) – Product Requirements Document

> Version: v0.3
> Owner: Gui Santos
> Scope: GIK core library + standalone CLI (`gik`)
> Last updated: 2025-12-09

---

## 1. Product Definition

### 1.1 What is GIK?

**GIK (Guided Indexing Kernel)** is a local-first knowledge engine that indexes, queries, and tracks the evolution of knowledge for a project. It provides a unified layer for:

* **RAG (Retrieval-Augmented Generation)** – semantic search over code, docs, and structured data.
* **KG (Knowledge Graph)** – entities and relationships between files, modules, services, domains, dependencies.
* **Memory** – events, decisions, rationales logged over time.
* **Stack / Inventory** – a raw, structural view of the project (folders, files, dependencies, technologies).

GIK runs as a **standalone CLI** (`gik`) and as a **Rust library** that other tools (e.g., CLI, IDEs, agents) consume to build prompts and context bundles.

### 1.2 Vision

GIK aims to be for **knowledge and context** what **Git is for source code**:

* Git tracks the evolution of **files and code**.
* GIK tracks the evolution of **knowledge and understanding** of a codebase and its domain.

Key aspects of the vision:

* Local-first, reproducible, and inspectable knowledge indices.
* Clear timelines of *what we know* and *why we decided* at each point in time.
* A single API to get high-quality context for LLMs and tools, without hardcoding RAG logic everywhere.
* Clean separation between **knowledge engine (GIK)** and **LLM orchestration (Guided)**.

### 1.3 Target Users

* Developers and teams building local-first tooling (CLIs, IDEs, agents).
* Users of Guided CLI / Guided Agent who need structured knowledge and context.
* Anyone who wants Git-like workflows (add/commit/log) for semantic knowledge instead of raw text.

### 1.4 Goals

* Provide a single, local-first **knowledge kernel** for projects.
* Standardize access to **RAG + KG + Memory + Stack** behind a small API and CLI.
* Track knowledge evolution and embedding/model changes over time.
* Make it easy for other tools to obtain high-quality context bundles without reimplementing indexing logic.

### 1.5 Non-goals

* GIK does **not** call LLMs or generate natural-language answers by itself.
* GIK does **not** replace Git for source control.
* GIK does **not** manage remote, multi-tenant hosted vector stores.
* GIK does **not** implement task orchestration or prompt workflows (this belongs to Guided and other layers).

### 1.6 GIK vs Guided

* **GIK**

  * Focus: indexing, storage, and querying of project knowledge.
  * Outputs: structured bundles (RAG chunks, KG expansions, Memory events, Stack data).
  * Responsibilities: parsing, chunking, embeddings, local storage, timelines.

* **Guided (CLI / Agents / IDE)**

  * Focus: prompting, LLM calls, task planning, and execution.
  * Uses GIK as a **context provider**.
  * Responsibilities: calling LLMs, applying diffs, generating code/docs, workflows.

GIK is a **foundation** that other tools stand on.

---

## 2. Problems GIK Solves

1. **Ad-hoc RAG pipelines everywhere**
   Every tool reimplements chunking, embeddings, and index layout. This duplicates logic, creates inconsistencies, and makes debugging hard. GIK centralizes this into a single kernel.

2. **No versioning or timeline for knowledge**
   Most RAG indexes are opaque: you don’t know *when* or *why* something was indexed, or when the embedding model changed. GIK introduces a **timeline** of knowledge revisions, with metadata about operations and models.

3. **No first-class memory of decisions and context**
   Decisions and rationales live in chats, tickets, and commits. Agents lack structured access to “project memory”. GIK provides a **Memory** layer: structured, queryable, tied to the project timeline.

4. **No shared abstraction for KG + RAG**
   Code structure, dependencies, and domain entities are either ignored or handled ad hoc. GIK links RAG results (chunks) with KG expansions (entities + relations) into a unified context bundle.

5. **Lack of local-first, offline-friendly knowledge engine**
   Many stacks rely on cloud APIs and external vector stores. GIK is **local-first by design**: embeddings, indices, and graph live on disk, under the project and/or user home.

6. **No canonical view of the project stack**
   Tools constantly re-scan folders, manifests, and dependencies. GIK maintains a **Stack base** with project inventory (structure, deps, tech), reusable by any consumer.

---

## 3. Core Concepts & Data Model

* **Workspace**
  Root folder of a project. Contains source files, docs, `.guided/`, `.git/`, etc.

* **Branch**
  Logical knowledge branch. When Git is present, GIK aligns with the current Git branch (e.g. `main`, `feature-x`). Without Git, GIK uses a logical branch named `default`.

* **Base**
  A logical knowledge base within a branch. At minimum:

  * `code` – chunks derived from source code.
  * `docs` – chunks derived from documentation and text files.
  * `memory` – structured memory events (decisions, notes, rationales).
  * `stack` – project inventory (folders, files, dependencies, technologies).

* **Revision**
  Immutable record of a knowledge update (e.g., `gik commit`, `gik reindex`, `gik release`) stored with metadata (`revisionId`, parent, branch, gitCommit, operations, message).

* **Embedding Profile**
  Configuration that defines how embeddings are produced:

  * `provider` (e.g., `candle`, `ollama`),
  * `model_id`,
  * `dimension`,
  * `local_path` (for local models),
  * model-specific options.
    Projects define embeddings inline in config; each base stores which model was used via `index/meta.json`.

* **AskContextBundle**
  Structured result returned by `gik ask`, including:

  * `revisionId` (HEAD),
  * RAG chunks from requested bases,
  * KG expansions (entities + relations),
  * Memory events,
  * optional Stack info,
  * debug metadata (scores, filters, etc.).

---

## 4. Comparison with Git

### 4.1 What Git does

* Tracks versions of **files and code**.
* Provides operations like `add`, `commit`, `log`, `branch`, `merge`, `diff`.
* Focuses on syntax-level changes and file contents.

### 4.2 What GIK does

* Tracks versions of **knowledge** about a project:

  * semantic chunks (code + docs),
  * relationships (KG),
  * decisions and events (Memory),
  * stack inventory (folders, dependencies, tech).
* Provides Git-inspired operations for knowledge:

  * `gik add` – stage sources, scan stack incrementally, and stage memory events.
  * `gik commit` – process staged items (parse, chunk, embed, index, graph) and create a knowledge revision.
  * `gik log` – view history of knowledge revisions.
  * `gik reindex` – rebuild a base when embedding model or settings change.
  * `gik release` – generate a `CHANGELOG.md` for knowledge.

### 4.3 Scope differences

* Git:

  * cares about bytes and lines in files.
  * oblivious to semantics, domains, and decisions.

* GIK:

  * cares about semantic meaning, relationships, and decisions.
  * associates knowledge to timelines, branches, and embedding models.
  * complements Git; it does not replace it.

### 4.4 Branch behavior

* With a Git repo, GIK aligns its knowledge branches with Git branches:
  `branches/main/`, `branches/feature-x/`, etc.
* Without Git, GIK uses a logical `default` branch.
* Each knowledge branch has its own bases (code/docs/memory/stack), indices, KG, and timeline.

---

## 5. Scope & Phases

High-level scope split between **Phase 1** and later phases.

### 5.1 Phase 1 Scope

* Local-first, offline knowledge engine.
* Bases: `code`, `docs`, `stack` (full inventory), `memory` (with metrics and pruning), `kg` (auto-synced).
* Embedding engine with a default local model (e.g., `all-MiniLM-L6-v2`).
* Git-aligned branches + `default` branch when Git is absent.
* Timeline with revisions (`gik commit`, `gik reindex`).
* Core CLI surface: `init`, `add`, `rm`, `commit`, `ask`, `log`, `status`, `bases`, `reindex`, `stats`, `release`, `show`, `memory-stats`, `prune-memory`, `config`.
* Knowledge Graph with auto-sync on commit/reindex, symbol extraction (13 languages), and endpoint detection.
* Hybrid search (BM25 + vector) with query expansion and reranking.

**Known gap**: Embedded default model (Phase 4.5) is not yet implemented—models must be manually cloned to the workspace `models/` directory.

### 5.2 Phase 2+ (Not Phase 1)

* Rich Memory UX (`gik memory list`, `gik memory ask`).
* Advanced Stack commands (`gik stack show`, `gik stack rescan`).
* Advanced diagnostics (`gik doctor`).
* Embedded default model in CLI binary (Phase 4.5).
* More embedding profiles and migration tools.

---

## 6. Functional Requirements

> Tags: `[Phase 1]` = required for first usable version; `[Phase 2]` = planned later.

### 6.1 Initialization and Configuration

* **FR-01 [Phase 1]** – `gik init` must create the base directory structure:

  ```
  <workspace>/.guided/knowledge/
  ├── config.yaml
  └── <branch>/
      ├── HEAD
      ├── timeline.jsonl
      ├── staging/
      │   ├── pending.jsonl
      │   └── summary.json
      ├── stack/
      │   ├── files.jsonl
      │   ├── dependencies.jsonl
      │   ├── tech.jsonl
      │   └── stats.json
      ├── bases/
      │   ├── code/
      │   │   ├── sources.jsonl
      │   │   ├── stats.json
      │   │   └── index/
      │   │       └── meta.json
      │   ├── docs/
      │   │   └── ...
      │   └── memory/
      │       ├── config.json
      │       ├── archive.jsonl
      │       └── index/
      └── kg/
          ├── nodes.jsonl
          ├── edges.jsonl
          └── stats.json
  ```

* **FR-02 [Phase 1]** – GIK must detect the current Git branch when `.git/` is present; otherwise it must use `default`.

* **FR-03 [Phase 1]** – Global configuration must live under `~/.gik/config.yaml` and define embedding profiles and defaults.

  > **Note**: Global config is not auto-created; users must manually create it if needed. This is flagged as a planned improvement.

* **FR-04 [Phase 1]** – Project-level configuration (`.guided/knowledge/config.yaml`) may override the embedding profile and other settings.

### 6.2 Stack Base (Inventory)

* **FR-05 [Phase 1]** – `gik init` must perform an initial **stack scan** for the current branch, populating the `stack` base with:

  * folders and files overview (paths, counts, languages),
  * manifests (e.g., `Cargo.toml`, `package.json`, `go.mod`, etc.),
  * dependencies extracted from manifests,
  * inferred technologies (frameworks, languages, infra).

* **FR-06 [Phase 1]** – The `stack` base must be stored in plain JSON/JSONL (e.g., `files.jsonl`, `dependencies.jsonl`, `tech.jsonl`, `stats.json`) under `stack/`, without embeddings.

* **FR-07 [Phase 1]** – `gik add` must ensure the `stack` base is up to date **for the added targets** by performing an incremental stack scan when necessary (new paths, changed manifests), before staging sources.

### 6.3 Staging and Commit Workflow

* **FR-08 [Phase 1]** – `gik add` must accept generic targets (no type flags):

  * directories, files, archives (zip/tar), URLs, or `.`.
  * GIK must infer target kind (dir/file/url/archive).

* **FR-09 [Phase 1]** – Staged items must be stored in JSONL under:

  * `staging/pending.jsonl` (unified staging file for all source types)
  * `staging/summary.json` (staging summary metadata)

* **FR-10 [Phase 1]** – `gik commit` must:

  * read staged items (sources + memory),
  * apply `.gikignore` and `.gitignore` rules,
  * parse and chunk sources into semantic chunks,
  * compute embeddings using the current embedding profile,
  * update vector indices for bases (at least `code` and `docs`),
  * update Memory store with committed events,
  * auto-sync KG with new nodes/edges derived from sources,
  * write a new revision entry into `timeline.jsonl` with `revisionId`, `parentRevisionId`, `branch`, `gitCommit`, `message`, and operations.

* **FR-11 [Phase 1]** – `gik commit` must support an optional `-m` message; if omitted, it must synthesize an automatic summary (bases touched, paths, counts).

* **FR-12 [Phase 1]** – After a successful commit, staging files must be cleared.

### 6.4 Asking and Context Bundles

* **FR-13 [Phase 1]** – `gik ask` must accept a natural language question and optional filters (bases, file patterns, scopes).

* **FR-14 [Phase 1]** – If no bases are specified, `gik ask` must use all available bases for the current branch (at least `code`, `docs`, and `memory` if present).

* **FR-15 [Phase 1]** – For each `ask`, GIK must:

  * embed the question using the active embedding profile (with optional query expansion),
  * perform hybrid search combining BM25 lexical search and vector similarity,
  * apply Reciprocal Rank Fusion (RRF) to merge results,
  * optionally rerank results using a cross-encoder model,
  * expand KG around relevant entities using bounded BFS,
  * query Memory for relevant events/decisions,
  * optionally include Stack information when requested or configured,
  * return an **AskContextBundle**.

* **FR-15a [Phase 1]** – `gik ask` must support hybrid search with BM25 + vector scoring and RRF fusion for improved recall and precision.

* **FR-15b [Phase 1]** – `gik ask` must support query expansion (multi-query embedding averaging) to improve recall for ambiguous queries.

* **FR-15c [Phase 1]** – `gik ask` must support optional reranking via cross-encoder models (e.g., `ms-marco-MiniLM-L6-v2`) for improved result ordering.

* **FR-15d [Phase 1]** – `gik ask` must detect filenames in the query and boost chunks from matching files.

* **FR-16 [Phase 1]** – CLI `gik ask` must output JSON by default (pipe-friendly for other tools / LLMs).

### 6.5 Embeddings and Models

* **FR-17 [Phase 1]** – GIK must use a local embedding engine by default (Candle + local model, e.g., `all-MiniLM-L6-v2`).

* **FR-18 [Phase 1]** – Embedding configuration must be defined in `~/.gik/config.yaml` or `.guided/knowledge/config.yaml` with:

  * `provider` (e.g., `candle`, `ollama`),
  * `model_id`,
  * `dimension`,
  * `local_path` (path to local model files),
  * `max_tokens` (optional),
  * per-base overrides under `embeddings.bases.<base_name>`.

* **FR-19 [Phase 1]** – Project config can choose an `embeddingProfile`; if absent, the global default profile must be used.

* **FR-20 [Phase 1]** – Each base (`code`, `docs`, `memory` if vectorized) must persist embedding metadata at `index/meta.json` including `model_id`, `provider`, and `dimension`.

* **FR-21 [Phase 1]** – When the active embedding profile does not match the base metadata (different `modelId` or `dim`), `gik ask` must refuse to use that base and instruct the user to run `gik reindex`.

* **FR-22 [Phase 1]** – `gik reindex --base <NAME>` must:

  * read existing sources for that base,
  * recompute embeddings using the new model,
  * rebuild indices,
  * update `model-info.json`,
  * register a new revision in the timeline.

### 6.6 Knowledge Bases and Storage

* **FR-23 [Phase 1]** – GIK must support multiple bases per branch (at least `code`, `docs`, `memory`, `stack`).

* **FR-24 [Phase 1]** – Each base must have its own storage subtree, including:

  * vector index (for bases that use embeddings),
  * config file,
  * `sources.jsonl` where applicable,
  * `stats.json`.

* **FR-25 [Phase 1]** – GIK must use a local storage backend for vectors and KG (when present); no external network calls are allowed in core operations.

### 6.7 Knowledge Graph

* **FR-26 [Phase 1]** – GIK must maintain a KG per branch that captures entities and relations derived from sources (files, modules, services, endpoints, symbols, dependencies). The KG is stored under `<branch>/kg/` with `nodes.jsonl`, `edges.jsonl`, and `stats.json`.

* **FR-27 [Phase 1]** – KG must auto-sync on `gik commit` and `gik reindex`; no explicit `gik kg build` command is required.

* **FR-28 [Phase 1]** – `gik show --kg-dot` and `gik show --kg-mermaid` must export the KG as DOT or Mermaid format for visualization.

* **FR-28a [Phase 1]** – KG must support multi-language symbol extraction (13 languages) for function, class, and method nodes.

* **FR-28b [Phase 1]** – KG must detect and index API endpoints (e.g., Next.js API routes) as `endpoint` nodes.

### 6.8 Memory

* **FR-29 [Phase 1]** – GIK must support structured memory events with at least: `id`, `scope`, `message`, `timestamp`, `tags`.

* **FR-30 [Phase 1]** – Memory events can be added via `gik add --memory "<message>"` with optional `--scope` and `--source` flags, and committed with `gik commit`.

* **FR-30c [Phase 1]** – Memory pruning configuration must be stored in `memory/config.json` and support `max_entries`, `max_age_days`, and `archive_pruned` options.

* **FR-31 [Phase 2]** – `gik memory list [--scope SCOPE]` must list memory events for a scope.

* **FR-32 [Phase 2]** – `gik memory ask "QUESTION"` must query only the Memory layer semantically.

### 6.9 Timeline and Releases

* **FR-33 [Phase 1]** – Each branch must have a `timeline.log.jsonl` file with ordered revisions.

* **FR-34 [Phase 1]** – `gik log` must present a human-readable view of the knowledge timeline (similar to `git log`).

* **FR-35 [Phase 1]** – `gik release [--tag <TAG>]` must:

  * collect revisions since the last release,
  * aggregate changes by base (code/docs/memory/stack) and KG/Memory operations (when available),
  * generate/update a `CHANGELOG.md` at the repo root (supports `--append` mode),
  * optionally record a `release` entry in the timeline.

  > **Current behavior**: `gik release` is read-only and does NOT record a timeline revision. Recording a timeline entry is planned for a future update.

### 6.10 Ignore Rules

* **FR-36 [Phase 1]** – GIK must support a `.gikignore` file at the workspace root with gitignore-like patterns.

* **FR-37 [Phase 1]** – GIK must also respect `.gitignore` patterns by default for file-based sources.

* **FR-38 [Phase 1]** – `.gikignore` takes precedence over `.gitignore` when both match.

---

## 7. Non-Functional Requirements

### 7.1 Local-First and Offline

* **NFR-01 [Phase 1]** – All core operations (`init`, `add`, `commit`, `ask`, `reindex`, `log`, `release`) must be fully functional offline.

* **NFR-02 [Phase 1]** – Embedding models must be loadable from local disk; no automatic downloads or calls to external APIs during normal use.

### 7.2 Performance

* **NFR-03 [Phase 1]** – GIK must be usable on a typical developer laptop (no dedicated GPU), focusing on CPU-only performance. GPU acceleration is optional and can be enabled via `--device gpu` flag or `GIK_DEVICE` environment variable.

* **NFR-04 [Phase 1]** – `gik ask` should respond in acceptable time for medium-sized projects (e.g., ~1–2 seconds for tens of thousands of chunks on a modern laptop).

* **NFR-05 [Phase 1]** – Index updates (`commit`, `reindex`) should be incremental where possible, avoiding full rebuilds unless necessary.

  > **Current behavior**: `gik commit` re-embeds all staged sources in each commit. True incremental embedding (skip unchanged chunks) is a future optimization.

### 7.3 Reliability and Safety

* **NFR-06 [Phase 1]** – GIK must never mix embeddings from different models within the same base; such a state must be detected and blocked.

* **NFR-07 [Phase 1]** – Corruption detection: if indexes or metadata are inconsistent, GIK should fail fast with clear diagnostics and recommend `gik reindex`.

* **NFR-08 [Phase 1]** – All on-disk formats (JSONL, configs) must be forward-compatible where possible; migrations must be explicit.

### 7.4 Developer Experience

* **NFR-09 [Phase 1]** – Directory structure, file names, and metadata must be clear and documented so users can inspect and debug.

* **NFR-10 [Phase 1]** – CLI error messages must be explicit and actionable (e.g., suggesting `gik reindex`, `gik init`, or checking `.gikignore`).

* **NFR-11 [Phase 1]** – GIK core must be exposed as a Rust library with a clean facade, so other tools (Guided CLI, IDE integrations, agents) can embed it without depending on the CLI.

### 7.5 Security and Privacy

* **NFR-12 [Phase 1]** – No user data must be sent to external services without explicit configuration.

* **NFR-13 [Phase 1]** – GIK should not require elevated permissions; all files live within the project directory and user home.

---

## 8. Command List

High-level CLI surface for `gik`, with phase indication.

```text
Command              | Description                                                                               | Phase
gik init             | Initialize GIK in the current workspace; create structure and initial stack scan.         | Phase 1
gik status           | Show current branch, bases, staged items, and index health.                               | Phase 1
gik bases            | List available bases (code/docs/memory/stack/kg) for the current branch.                  | Phase 1
gik add [TARGET]     | Stage targets, update stack incrementally, and record pending sources/memory.             | Phase 1
gik add --memory     | Add a memory event with optional --scope and --source flags.                              | Phase 1
gik rm [PATHS]       | Remove files from the staging area.                                                       | Phase 1
gik commit [-m]      | Process staged items, update bases and timeline, create a knowledge revision.             | Phase 1
gik log              | Show knowledge timeline (revisions) for the current branch.                               | Phase 1
gik log --kind ask   | Show ask query history for the current branch.                                            | Phase 1
gik ask "..."        | Run semantic query over RAG + Memory + KG; output JSON. Supports hybrid search.           | Phase 1
gik stats [--base]   | Show stats for all knowledge or a specific base.                                          | Phase 1
gik reindex --base   | Rebuild embeddings and indices for a base using the active embedding profile.             | Phase 1
gik release [--tag]  | Generate/update CHANGELOG.md based on revisions since the last release.                   | Phase 1
gik show [REVISION]  | Inspect a specific revision by ID (similar to git show). Defaults to HEAD.                | Phase 1
gik show --kg-dot    | Export the KG as DOT format for visualization.                                            | Phase 1
gik show --kg-mermaid| Export the KG as Mermaid format for visualization.                                        | Phase 1
gik config check     | Validate configuration files.                                                             | Phase 1
gik config show      | Display resolved configuration.                                                           | Phase 1
gik config init      | Create a default global config file at `.gik/config.yaml` with all available settings.   | Phase 1
gik config set KEY VALUE | Set a configuration value. Validates the key and value before persisting.            | Phase 1

gik memory list      | List memory events, optionally filtered by scope.                                         | Phase 2
gik memory ask       | Query only the Memory layer semantically.                                                 | Phase 2
gik stack show       | Show aggregated stack/inventory view (folders, deps, tech).                               | Phase 2
gik stack rescan     | Force a full rescan of the stack base for the current branch.                             | Phase 2
gik doctor           | Run health checks on knowledge store and report issues and suggested fixes.               | Phase 2
```

---

*End of PRD v0.3.*
