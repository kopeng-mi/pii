# pii Agent Instructions

## Developer Workflow
- **Run without args (Phase 3 default)**: `cargo run` (aliases `cargo run -- -c`)
- **Test Phase 1 table UI**: `cargo run -- -w` (Past 7 days) or `cargo run -- -t` (Today)
- **FTS Search test**: `cargo run -- -q "refactor"`
- **Clear DB cache**: `rm pii.db` in project root. Do this when you modify `src/session/parser.rs` or DB schema so the system re-parses all `~/.pi/agent/sessions/` JSONL files.

## Project Conventions
- **No external DBs**: We use a single local SQLite `pii.db` using `rusqlite` + `bundled` feature. No system SQLite installations are needed.
- **TUI Separation**: 
  - The interactive list picker uses **raw `crossterm` inline** (mimicking `fzf --height=40%`). It is *not* a full-screen app. Do not use `ratatui` for the picker component.
  - **`ratatui`** is strictly reserved for rich full-screen views (Spider charts, Model Comparisons, Rankings).
- **Console Output**: Non-interactive commands (like `pii -w` or `pii -H`) print direct raw ANSI escape codes to `stdout`. Do not use `ratatui` for these.
- **UI Colors**: Use teal (`#00d7af` / `256:43`), gold (`#ffd700` / `256:220`), rose (`#ff0000` / `256:196`), and white bold for emphasis. 

## Crate Constraints
- Do **not** add `tokio`. We use `ureq` for sync HTTP fetching to keep the binary small and execution fast.
- Do **not** use external chart crates. The radar/spider plot is built natively using `ratatui` Canvas widgets.
- Do **not** use `colored` or `owo-colors`. Print raw ANSI directly for standard outputs, or use `ratatui` styling features for TUI outputs.

## Development Phases
Refer to `plan.md` for the strict iterative sequence. You must implement and verify one phase at a time using the testing checkpoints table in `plan.md`.

- Phase 1: Skeleton + DB + Parsing + ANSI Tables (Completed)
- Phase 2: Heatmap + Summary
- Phase 3: Inline fzf-style Picker + Inspect + Continue
- Phase 4: Model API HTTP Fetching
- Phase 5: TUI Comparison + Spider Chart
- Phase 6: Cost Estimation + FTS integration
- Phase 7: Rankings + Polish