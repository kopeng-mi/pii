# pii Experience

## Identity
Rust CLI application that ports the nushell scripts `pii.nu`, `llm-s.nu`, `llm-c.nu` to a fast, terminal-native Rust binary. Provides analytics for `pi` coding agent sessions, LLM model browsing, and interactive model comparisons.

## Stack & Versions
- **Language**: Rust (Edition 2024)
- **Database**: SQLite (via `rusqlite` with `bundled` feature)
- **TUI**: `crossterm` for the fzf-style inline picker, `ratatui` for full-screen rich views (spider charts, comparisons).
- **Fuzzy Match**: `nucleo` (parallel, helix-editor grade).
- **CLI**: `clap` with derive macros.

## Architecture & Data Flow
1. **Local State**: All state lives in `pii.db` in the current working directory.
2. **Sync on Boot**: When `pii` runs, it scans `~/.pi/agent/sessions/`.
   - Compares disk `file_size` to DB `file_size`.
   - Only parses `.jsonl` files that changed.
   - Inserted into DB `sessions` and `calls` tables.
   - Triggers keep the FTS5 virtual table (`sessions_fts`) in sync.
3. **Rendering**:
   - Lists and tables: Raw ANSI escape codes printed to stdout (keeps output pipe-friendly and scrollable).
   - Interactive pickers: `crossterm` raw mode inline (like `fzf --height`).
   - Rich dashboards (Phase 5): `ratatui`.

## Repo Layout
```
src/
├── main.rs              — Entry point, syncs DB, dispatches to commands
├── cli.rs               — `clap` definitions for all flags and subcommands
├── db.rs                — SQLite schema and FTS5 configuration
├── session/             — Parsing pi JSONL session logs
│   ├── parser.rs        — Sync logic and JSON line processing
│   └── types.rs         — Database row representations
├── models/              — (Future) API fetching and LLM stats
└── ui/                  — Rendering layer
    ├── heatmap.rs       — ANSI block grid rendering for session activity
    ├── picker.rs        — Raw crossterm inline fuzzy picker (Nucleo)
    ├── summary.rs       — Session/model aggregation view
    └── table.rs         — Custom raw-ANSI spark bars and truncation
```

## Gotchas & Hard-won Lessons

1. **Pi v3 Session JSONL Quirks**:
   - The JSON logs produced by `pi` are not uniform.
   - The `usage` and `model` objects used to be nested under `message`. In newer versions, they are top-level properties alongside `message`.
   - *Fix*: The parser checks `value.get("usage")` at the root. If not found, it falls back to looking inside `value.get("message")`. 
   - `total_tokens` inside `usage` is not always a reliable integer type in JSON. Always parse it using `.as_f64()` before casting to `u32`.

2. **Crate Feature Misconfigurations**:
   - `rusqlite` requires the `bundled` feature. FTS5 support is included by default in the bundled version (no need for `fts5` feature flag which doesn't exist).
   - `unicode-width` requires importing the `UnicodeWidthChar` trait to call `.width()` on a single `char`, and `UnicodeWidthStr` for strings.

3. **Ambiguous SQLite Columns**:
   - Because `sessions_fts` creates a virtual table with the same column names as `sessions`, joining them (`FROM sessions JOIN sessions_fts ON ...`) makes columns like `project` ambiguous. 
   - *Fix*: Always prefix selected columns with `sessions.` (e.g., `sessions.project`) when performing FTS matches.

4. **Terminal Formatting**:
   - Standard ANSI output requires resetting colors `\x1b[0m` correctly after every styled segment, especially around emojis, spark bars (`make_bar`), or truncated strings to prevent color leaking into the terminal background.
   - Spark bars use a fractional block mapping (eighths) for precise visual representation.

5. **Inline Picker (Raw Crossterm)**:
   - When using `crossterm` in raw mode without alternate screen, `\n` does not implicitly emit a carriage return. Using `Print("\r\n")` can cause ghosting/scrolling bugs if the render loop dynamically clears the cursor down. 
   - *Fix*: Reserve space initially by writing literal `b"\r\n"` bytes to stdout, move the cursor up, and strictly use `ClearType::CurrentLine` on every drawn line rather than `ClearType::FromCursorDown` to avoid viewport tearing and leftover stale content.
   - For multi-session/call detail views, moving cursor precisely with `\x1b[{}A` / `\x1b[{}B` is required to retroactively patch rows (e.g., updating a top summary line after reading call counts).

6. **Fuzzy Matching Heuristics**:
   - Model providers use wildly different ID formats (e.g., `openai/gpt-4o-2024-05-13` vs `chatgpt-4o-latest`). 
   - String `contains` checks cause incorrect overlapping merges. 
   - *Fix*: Using `nucleo` for API response merging. Strip vendor prefixes and dates first, then score. A `score > 60` threshold prevents unrelated models from clobbering each other.

## Build & Run
- **Build**: `cargo build`
- **Run**: `cargo run -- [args]`
  - `cargo run -- -t` (Today's sessions)
  - `cargo run -- -w -q "react"` (Past 7 days, fuzzy text search for "react")
  - `cargo run -- -c` or `cargo run` (Fuzzy session picker for continuation)
  - `cargo run -- -i` (Fuzzy session picker for inspection/stats breakdown)
- **Reset State**: `rm pii.db` (rebuilds cache from scratch on next run)
