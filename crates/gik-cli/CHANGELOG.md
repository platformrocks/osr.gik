# Changelog

All notable changes to `gik-cli` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2024-12-09

### Added

- Initial release of the `gik` CLI binary
- Command dispatch to `GikEngine` for all operations
- Full command set: `init`, `status`, `bases`, `add`, `rm`, `commit`, `show`, `ask`, `stats`, `reindex`, `release`, `log`, `config`
- Global flags: `--verbose`, `--quiet`, `--config`, `--device`, `--color`
- JSON output mode (`--json`) for machine-readable output
- Styled output with colors, prefixes, and progress indicators
- UI module with `Style` helpers for consistent formatting
- Progress spinners for long-running operations via `indicatif`
- Color mode detection (auto/always/never)
- Integration test suite covering full workflows
- Feature flag propagation for `metal` and `cuda` GPU support

### Architecture

- Thin UX layer with zero business logic
- All domain operations delegated to `gik-core`
- Clap derive-based argument parsing
- Modular UI system (`color`, `format`, `progress`, `style`, `table`)
