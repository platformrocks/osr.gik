# gik-cli

> Thin CLI layer for GIK—handles user interaction with zero business logic.

## Overview

`gik-cli` is the binary crate that provides the `gik` command-line interface. It handles argument parsing with `clap`, formats output using `Style` helpers, and dispatches all commands to `GikEngine` in `gik-core`. This crate contains **no business logic**—it is purely a UX layer.

## Goals

- **Separation of concerns**: Keep CLI parsing and output formatting isolated from domain logic
- **Consistent UX**: Provide unified styling, progress indicators, and error formatting across all commands
- **Zero logic leakage**: All domain decisions happen in `gik-core`; CLI only translates user intent

## Features

- Parses CLI arguments and flags using `clap` with derive macros
- Dispatches commands to `GikEngine` methods
- Formats output with consistent styling (colors, prefixes, tables)
- Displays progress spinners for long-running operations
- Supports `--json` output mode for machine-readable responses
- Handles global flags (`--verbose`, `--quiet`, `--config`, `--device`, `--color`)

## Architecture

### Module Overview

```
src/
├── main.rs          # Entry point, tracing setup, command dispatch
├── cli.rs           # Cli struct, Command enum, all command handlers (~2000 lines)
└── ui/
    ├── mod.rs       # Re-exports
    ├── color.rs     # ColorMode detection (auto/always/never)
    ├── format.rs    # Utility formatters (bytes, duration)
    ├── progress.rs  # Spinner and progress bar helpers
    ├── style.rs     # Style struct, MessageType, prefixes
    └── table.rs     # Table rendering utilities
```

### Key Types

- `Cli` — Root clap struct with global flags
- `Command` — Enum of all subcommands (Init, Add, Commit, Ask, etc.)
- `Style` — Output formatting helper with color and prefix support
- `MessageType` — Ok, Warn, Error, Info message categories

### Command Flow

```
User Input → clap parsing → Cli/Command structs
                              ↓
                         main.rs dispatch
                              ↓
                    GikEngine method call
                              ↓
                    Result<T, GikError>
                              ↓
                    Style-formatted output
```

## Commands

| Command | Description |
|---------|-------------|
| `init` | Initialize GIK workspace |
| `status` | Show workspace status |
| `bases` | List knowledge bases |
| `add <PATH>` | Stage sources for indexing |
| `rm <PATH>` | Remove from staging |
| `commit` | Index staged sources |
| `show [REV]` | Inspect revision (like `git show`) |
| `ask <QUERY>` | Query knowledge (RAG) |
| `stats` | Show base statistics |
| `reindex` | Rebuild embeddings |
| `release` | Generate CHANGELOG |
| `log` | Show revision history |
| `config check` | Validate configuration |
| `config show` | Show resolved config |

## Dependencies

### Internal (Workspace)

| Crate | Purpose |
|-------|---------|
| `gik-core` | All business logic via `GikEngine` |

### External

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing with derive |
| `owo-colors` | Terminal color output |
| `indicatif` | Progress bars and spinners |
| `tracing` | Structured logging |
| `tracing-subscriber` | Log output formatting |
| `anyhow` | Error context in main |

## Usage

### Basic Commands

```bash
# Initialize a GIK workspace
gik init

# Add sources and commit
gik add src/
gik commit -m "Index source code"

# Query the knowledge base
gik ask "How does the config system work?"

# Show workspace status
gik status
```

### Global Flags

```bash
# Enable verbose logging
gik -v ask "query"

# Suppress progress output
gik -q commit -m "Silent"

# Use custom config file
gik -c custom.yaml status

# Force GPU or CPU
gik --device gpu ask "query"

# Control color output
gik --color never status
```

### JSON Output

```bash
# Machine-readable output
gik status --json
gik ask "query" --json
gik stats --json
```

## Configuration

CLI flags override environment variables which override config file values:

| Flag | Environment | Config Key | Default |
|------|-------------|------------|---------|
| `-v, --verbose` | `GIK_VERBOSE` | — | `false` |
| `-q, --quiet` | — | — | `false` |
| `-c, --config` | `GIK_CONFIG` | — | `gik.yaml` |
| `--device` | `GIK_DEVICE` | `device` | `auto` |
| `--color` | — | — | `auto` |

## Feature Flags

| Flag | Effect | Propagates To |
|------|--------|---------------|
| `metal` | Enable macOS GPU acceleration | `gik-core/metal` → `gik-model/metal` |
| `cuda` | Enable NVIDIA GPU acceleration | `gik-core/cuda` → `gik-model/cuda` |

Build with GPU support:

```bash
cargo build -p gik-cli --features cuda    # NVIDIA
cargo build -p gik-cli --features metal   # macOS
```

## Testing

```bash
# Run CLI integration tests (requires models)
cargo test -p gik-cli

# Run tests that need real ML models
cargo test -p gik-cli -- --ignored
```

### Integration Test Files

| Test | Coverage |
|------|----------|
| `integration_flow.rs` | Full init→add→commit→ask flow |
| `memory_ask_and_status.rs` | Memory base ingestion/retrieval |
| `kg_extraction_from_bases.rs` | Knowledge graph extraction |
| `release_and_changelog.rs` | Release generation |
| `show_cli.rs` | Show command variants |

## Versioning

This crate follows the workspace version defined in the root `Cargo.toml`.
See [CHANGELOG.md](./CHANGELOG.md) for version history.

## Related Documentation

- [Crates Overview](../../.guided/architecture/crates-overview.md) — All crates in the workspace
- [Architecture Document](../../docs/5-ARCH.md) — Global architecture view
- [Command Reference](../../docs/4-COMMANDS.md) — Detailed command documentation
