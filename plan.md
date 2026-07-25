# pii — Implementation Plan

> Rust CLI: pi session analytics + LLM model explorer + interactive comparison.
> Port of nushell `pii.nu` / `llm-*.nu`. Stylish, minimal, terminal-native.

---

## Architecture

```
src/
├── main.rs              — dispatch
├── cli.rs               — clap derive
├── db.rs                — SQLite + FTS5 (single pii.db)
├── session/
│   ├── mod.rs
│   ├── parser.rs        — JSONL → rows
│   └── types.rs         — SessionRow, GroupedSession
├── models/
│   ├── mod.rs
│   ├── api.rs           — AA + LLM-Stats HTTP clients
│   ├── types.rs         — UnifiedModel, Evaluation
│   └── fuzzy.rs         — nucleo match + pricing lookup
├── picker.rs            — fzf-style interactive picker
├── ui/
│   ├── mod.rs
│   ├── table.rs         — styled tables (ratatui Table widget)
│   ├── heatmap.rs       — contribution grid
│   ├── bar.rs           — spark bars with embedded labels
│   ├── spider.rs        — radar chart (Canvas)
│   ├── compare.rs       — side-by-side model comparison
│   ├── detail.rs        — model card / session detail
│   └── theme.rs         — palette, model colors, box chars
└── util.rs              — compact_num, term_size
```

---

## Storage: SQLite + FTS5

Single file: `~/.pi/agent/pii.db`

### Schema

```sql
-- Sessions (parsed from JSONL)
CREATE TABLE sessions (
  id          TEXT PRIMARY KEY,  -- timestamp_uuid from filename
  project     TEXT NOT NULL,
  file_path   TEXT NOT NULL UNIQUE,
  file_size   INTEGER NOT NULL,  -- cache invalidation
  date        TEXT NOT NULL,      -- YYYY-MM-DD
  time        TEXT NOT NULL,      -- HH:MM
  prompt      TEXT DEFAULT '',
  total_calls INTEGER DEFAULT 0,
  total_tokens INTEGER DEFAULT 0,
  total_cost  REAL DEFAULT 0.0,
  errors      INTEGER DEFAULT 0
);

-- Per-call rows within sessions
CREATE TABLE calls (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions(id),
  model         TEXT NOT NULL,
  input_tokens  INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  tokens        INTEGER DEFAULT 0,
  cost          REAL DEFAULT 0.0,
  is_error      BOOLEAN DEFAULT 0
);

-- LLM model data (from APIs, refreshed daily)
CREATE TABLE models (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  creator       TEXT DEFAULT '',
  release_date  TEXT DEFAULT '',
  context_window INTEGER,
  param_count   INTEGER,
  input_price   REAL DEFAULT 0.0,
  output_price  REAL DEFAULT 0.0,
  speed_tok_s   REAL,
  ttft_s        REAL,
  open_weight   BOOLEAN DEFAULT 0,
  source        TEXT DEFAULT '',     -- 'llm-stats' | 'aa' | 'merged'
  raw_json      TEXT DEFAULT '{}'    -- full payload for detail views
);

-- Benchmark scores
CREATE TABLE scores (
  model_id      TEXT NOT NULL REFERENCES models(id),
  benchmark     TEXT NOT NULL,
  score         REAL NOT NULL,
  max_score     REAL,
  category      TEXT DEFAULT '',
  PRIMARY KEY (model_id, benchmark)
);

-- FTS5 for fast text search across sessions and models
CREATE VIRTUAL TABLE sessions_fts USING fts5(
  project, prompt, models,
  content=sessions, content_rowid=rowid
);

CREATE VIRTUAL TABLE models_fts USING fts5(
  name, creator, id,
  content=models, content_rowid=rowid
);

-- Cache metadata
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT
);
-- Keys: 'models_fetched_date', 'last_session_scan'
```

### Why SQLite over JSON files

- FTS5 gives instant search across hundreds of sessions/models
- SQL aggregation for summary/heatmap (GROUP BY date, SUM tokens)
- Single file, no dependency sprawl, `rusqlite` bundles SQLite
- Cache invalidation: compare file_size in DB vs disk, re-parse only changed files
- Model data refresh: check `meta.models_fetched_date` vs today

---

## Crates

| Purpose | Crate | Why |
|---------|-------|-----|
| CLI | `clap` derive | Standard |
| DB | `rusqlite` + `bundled` + FTS5 | Single file, FTS5, fast |
| HTTP | `ureq` | Sync, no tokio, small binary |
| JSON | `serde` + `serde_json` | Standard |
| TUI | `ratatui` + `crossterm` | ratatui for rich views (spider, compare). crossterm alone for picker |
| Fuzzy | `nucleo` | Fastest (helix-editor), parallel |
| Env | `dotenvy` | .env loading |
| Time | `chrono` | Date math |
| Unicode | `unicode-width` | Column alignment |
| Home dir | `dirs` | Cross-platform ~/.pi |

No tokio. No colored/owo-colors (ratatui owns styling). No extra chart crates.

---

## CLI Interface

```
pii                              → continue last session (= pii -c)
pii -c                           → fuzzy-pick session, continue in pi
pii -i [--calls]                 → fuzzy-pick session, inspect detail
pii -t                           → today's sessions
pii -w                           → past 7 days
pii -m                           → past 30 days
pii -H                           → activity heatmap (180 days)
pii -s                           → summary dashboard
pii -q <pattern>                 → filter by model name
pii --sort <cost|tokens|calls|time>
pii -d <N>                       → scope picker to last N days

pii model [query]                → model detail card
pii model --refresh              → force API re-fetch
pii compare [m1 m2 ...]          → side-by-side comparison
pii compare --spider             → + radar chart
pii rankings [category]          → TrueSkill rankings

pii -h                           → styled help
```

Default `pii` (no args) = `pii -c`.

---

## Picker Design (fzf-minimal)

Not a full-screen ratatui app. A **lean inline picker** like fzf's `--height` mode.
Renders below the cursor in the existing terminal. No borders, no chrome, no preview pane.
Just the list, the prompt, and the match count. Minimal and fast.

Use `crossterm` raw mode for input + inline rendering. No ratatui for the picker —
ratatui is reserved for the rich views (spider, compare, detail). The picker is
just cursor movement + ANSI redraws over N lines, exactly like fzf.

### Layout

```
  12/847
  07-23 14:20  pii      gpt-4o           2   8K   $0.04
  07-23 21:45  opto     claude-opus-4    11  62K   $0.38
  07-24 16:00  tri      deepseek-r2       4  91K   --
  07-24 18:30  haven    gemini-pro-agent  1   4K   $0.02
  07-24 22:15  bcore    gpt-5.6-luna      3  12K   $0.08
  07-25 05:22  pii      claude-opus-4    14  82K   $0.42
▸ 07-25 10:39  pii      claude-opus-4     6  24K   $0.18
  select▸ clau_
```

- Reverse layout (like `fzf --layout=reverse --height=40%`)
- Results above the prompt, newest at bottom (closest to cursor)
- Prompt at bottom with `▸` marker
- Match count top-right (`12/847`)
- Selected line gets `▸` pointer + bold/highlight
- No borders. No box drawing. Just text.
- Height: min(item_count + 2, terminal_height * 40%)

### Behaviors

- **Type to filter**: nucleo fuzzy matching, re-rank live
- **Match highlighting**: matched chars in teal on the selected + visible items
- **↑↓ / Ctrl-P/N / Ctrl-K/J**: move cursor
- **Enter**: select and return
- **Esc / Ctrl-C / Ctrl-G**: cancel
- **Tab** (compare mode only): toggle mark on current item, `+` prefix on marked
- **No preview pane** — detail comes after selection, not during

### Multi-select (for `pii compare`)

```
  4/200
  + gpt-4o             OpenAI       $2.50/$10   128K
    gemini-2.5-pro     Google       $1.25/$5    1M
  + claude-sonnet-4    Anthropic    $3.00/$15   200K
▸   deepseek-r2        DeepSeek     $0.55/$2.19 128K
  compare▸ deep_                              2 selected
```

`+` prefix = marked. Status shows count. Enter confirms all marked.

### Implementation

No ratatui. Crossterm raw mode:
1. Enter raw mode, hide cursor
2. Calculate height (40% of term or item count)
3. Render loop: clear N lines, print items + prompt, read key
4. On keystroke: update query → re-score with nucleo → re-render
5. On Enter: restore terminal, return selection
6. On Esc: restore terminal, return None

Same picker component for sessions and models — just different item formatters.

---

## Visual Design Language

### Principles
- **Dark terminal native** — looks good on any dark theme
- **Teal accent** (#00d7af / 256:43) — primary highlight, bars, headers
- **Gold for money** (#ffd700 / 256:220) — costs, prices
- **Rose for errors** (#ff0000 / 256:196) — error counts
- **White bold for emphasis** — project names, selected items
- **Gray gradients for hierarchy** — 255 > 246 > 242 > 237
- **Model-family colors** — instant visual identification

### Model Colors
| Family | ANSI 256 | Visual |
|--------|----------|--------|
| Claude | 215 | warm orange |
| GPT | 114 | soft green |
| Gemini | 75 | sky blue |
| DeepSeek | 147 | lilac |
| Kimi | 183 | pale violet |
| Qwen | 180 | sand |
| GLM | 109 | steel blue |
| MiMo | 174 | dusty rose |
| Other | 250 | light gray |

### Table Style

Unicode box-drawing, thin borders, header emphasis:

```
  ━━ Today [2026-07-25] · 4 sessions · 18 calls · $1.24

  when          project   model                  calls  usage            cost   err
  ─────────────────────────────────────────────────────────────────────────────────
  07-25 10:39   pii       claude-opus-4-6-thin…    14   ████████  82K   $0.42   ·
  07-25 08:12   bcore     gpt-5.6-luna              3   ██        12K   $0.08   ·
  07-25 06:30   haven     gemini-pro-agent          1   █          4K   $0.02   ·
  07-25 02:15   tri       deepseek-r2              --   ████████  91K   --      1
```

- Bars have value embedded inside (dark text on bright fill, dim text on empty)
- Cost color scales: green < yellow < orange < red by ratio to max
- Errors: rose numeral or dim `·` for zero
- Truncate model names to fit terminal, with `…`

### Heatmap Style

```
  ━━ Activity · 182d · 312 calls · 2.1M tok · $48.72

         May       Jun       Jul
    ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■
  M ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■
    ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■
  W ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■
    ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■
  F ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■
    ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■ ■

    Less ■ ■ ■ ■ ■ More
```

Grid columns adapt to terminal width. Intensity: token volume per day.

### Spider Chart

```
                    intelligence
                        ╱╲
                       ╱  ╲
              math ───╱────╲─── coding
                     ╱ ·····╲
                    ╱·········╲
              speed ───────────── price
                        │
                      context

  ■ claude-sonnet-4   ■ gpt-4o   ■ gemini-2.5-pro
```

Drawn with ratatui Canvas. Concentric guide polygons at 25/50/75/100%.
Each model = colored polygon overlay. Labels at spoke tips.
Normalized per-metric to max across compared models (or 100 for % benchmarks).

### Comparison Table

```
  ━━ Model Comparison · 3 models

                     claude-sonnet-4      gpt-4o              gemini-2.5-pro
  ─────────────────────────────────────────────────────────────────────────────
  Creator            Anthropic            OpenAI              Google
  Released           2025-05              2024-05             2025-03
  Context            200K                 128K                1M
  In ($/1M)          $3.00                $2.50               $1.25  ◀ best
  Out ($/1M)         $15.00               $10.00              $5.00  ◀ best
  Speed (tok/s)      82                   95  ◀ best          68
  ─ Benchmarks ──────────────────────────────────────────────────────────────
  MMLU-Pro           ████████████  89.1   ██████████  82.3    ████████████  88.7
  GPQA               ████████  72.1       ████████  71.5      █████████  76.3  ◀
  LiveCodeBench      ████████████  85.2   ██████████  78.1    ████████████  84.9
  MATH-500           ████████████  96.4   ██████████  91.0    ████████████  95.8
```

Best value per metric gets `◀ best` marker. Bars normalized to max.

---

## Data Sources

### Pi Sessions (`~/.pi/agent/sessions/`)

Directory per project (path-encoded), JSONL files per session.

**JSONL types (v3):**
| type | Useful fields |
|------|--------------|
| `session` | `id`, `timestamp`, `cwd` |
| `model_change` | `provider`, `modelId` |
| `message` (role=user) | `content[].text` → first user prompt |
| `message` (role=assistant) | `model`, `usage.{input,output,totalTokens}`, `usage.cost.total`, `stopReason` |

**Parse strategy:** scan first 50 lines for user prompt, all lines for assistant usage.
**Cache invalidation:** file_size in DB vs stat(). Re-parse only on mismatch.

### Artificial Analysis API
- Base: `https://artificialanalysis.ai/api/v2`
- Auth: `x-api-key: {ARTIFICI_ALANALYSIS_API_KEY}`
- Endpoint: `GET /data/llms/models`
- Rate limit: 1,000/day
- Fields: `id`, `name`, `model_creator`, `evaluations.*`, `pricing.*`, `median_output_tokens_per_second`, `median_time_to_first_token_seconds`

### LLM-Stats API
- Base: `https://api.llm-stats.com/stats/v1`
- Auth: `Authorization: Bearer {LLM_STATS_API_KEY}`
- Endpoints: `GET /v1/models?limit=200`, `GET /v1/models/{id}`, `GET /v1/rankings?category=...`
- Rate limits: 60/min (models), 120/min (detail/rankings)
- Fields: `id`, `name`, `organization`, `providers[].{input,output}_price_per_m`, `top_scores`, `context_window`, `param_count`, `open_weight`

**Merge strategy:** LLM-Stats primary (richer scores, rankings). AA fills speed/ttft/intelligence index. Dedupe by normalized ID.

### Fuzzy Model Matching (session→pricing)
1. Strip provider prefix: `anthropic/claude-sonnet-4-20250514` → `claude-sonnet-4-20250514`
2. Strip date suffix → `claude-sonnet-4`
3. Strip `-thinking` → `claude-sonnet-4`
4. Dots → dashes
5. Exact match → starts-with → nucleo fuzzy (threshold)

---

## Phases

### Phase 1 — Skeleton + DB + Session Parsing

**Delivers:** `pii -t`, `pii -w`, `pii -m`, `pii --sort`, `pii -q`

- Set up clap CLI with all flags/subcommands
- Create SQLite DB with schema (sessions, calls, meta tables)
- Parse JSONL files into DB (with file_size cache check)
- Session table output: styled ANSI tables with spark bars
- Terminal-width-aware column truncation

**Test:** `cargo run -- -t` shows today. `cargo run -- -w --sort cost` sorts by cost. Re-run is fast (cached).

**Deps:** clap, serde, serde_json, chrono, crossterm, dotenvy, rusqlite (bundled), unicode-width

### Phase 2 — Heatmap + Summary

**Delivers:** `pii -H`, `pii -s`

- Heatmap: SQL `GROUP BY date` → 7×N grid, terminal-width adaptive columns
- Summary: aggregate today/7d/30d/all via SQL
- Section headers with box-drawing accents

**Test:** `cargo run -- -H` renders heatmap. `cargo run -- -s` shows 4-row summary.

### Phase 3 — Picker + Inspect + Continue

**Delivers:** `pii -i`, `pii -c`, `pii` (default)

- Inline picker with crossterm raw mode (no ratatui) — like `fzf --height=40% --reverse`
- Nucleo fuzzy matching, match char highlighting
- Reverse layout: results above prompt, prompt at bottom
- Height adapts to item count and terminal size
- Session detail view: printed after selection (not in a preview pane)
- `-c` execs `pi --session <file>` after selection
- Default `pii` = `pii -c`
- Multi-select mode for compare (Tab to mark)

**New deps:** ratatui, nucleo

**Test:** `cargo run -- -i` opens picker. Type to filter. Enter shows detail. `cargo run -- -c` launches pi.

### Phase 4 — Model Data Layer

**Delivers:** `pii model [query]`

- HTTP fetch from both APIs, normalize into `models` + `scores` tables
- FTS5 tables for sessions and models
- Daily auto-refresh check (`meta.models_fetched_date`)
- `--refresh` forces re-fetch
- Model detail card: pricing, speed, benchmark bars
- Model picker: same picker component, different data

**New deps:** ureq

**Test:** `cargo run -- model claude` shows card. `cargo run -- model` opens picker.

### Phase 5 — Comparison + Spider Chart (Completed)

**Delivers:** `pii compare [models...]`, `pii compare --spider`

- Multi-select picker (Tab to mark models)
- Side-by-side metric table with `◀ best` markers
- Spark bars for benchmarks, normalized to max
- Spider chart via ratatui Canvas: guide polygons, colored model overlays
- Winner highlighting per metric

**Test:** `cargo run -- compare claude-sonnet-4 gpt-4o --spider` renders both views.

### Phase 6 — Cost Estimation + FTS Search

**Delivers:** cost column populated, fast search

- Match session model names → model pricing via fuzzy match
- Estimated cost when pi-reported cost is 0
- FTS5 search: `pii -q "claude coding"` uses FTS MATCH
- Search across prompts + project names + model names

**Test:** `cargo run -- -w` shows costs. `cargo run -- -q "refactor"` finds sessions by prompt text.

### Phase 7 — Rankings + Polish

**Delivers:** `pii rankings`, `-h` styled help, final polish

- Rankings view from LLM-Stats `/v1/rankings`
- Custom `--help` template (styled, compact)
- Consistent spacing at 80/120/200+ col widths
- Error handling: missing keys, network fails, empty results
- `PI_CODING_AGENT_SESSION_DIR` env var support
- DB migrations (version check in meta table)

**Test:** Full workflow. All views render cleanly.

---

## Cargo.toml (Final)

```toml
[package]
name = "pii"
version = "0.1.0"
edition = "2024"

[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
crossterm = "0.28"
ratatui = "0.29"
nucleo = "0.5"
ureq = { version = "3", features = ["json"] }
dotenvy = "0.15"
unicode-width = "0.2"
dirs = "6"
rusqlite = { version = "0.34", features = ["bundled", "fts5"] }
```

---

## Testing Checkpoints

| Phase | Command | Expected |
|-------|---------|----------|
| 1 | `cargo run -- -t` | Styled session table |
| 1 | `cargo run -- -w --sort cost` | Sorted by cost |
| 2 | `cargo run -- -H` | Heatmap grid |
| 2 | `cargo run -- -s` | Summary 4-row table |
| 3 | `cargo run -- -i` | Picker → session detail |
| 3 | `cargo run -- -c` | Picker → launches pi |
| 3 | `cargo run` | Same as -c |
| 4 | `cargo run -- model` | Model picker → card |
| 4 | `cargo run -- model gpt` | Direct model card |
| 5 | `cargo run -- compare gpt-4o claude-sonnet-4` | Side-by-side |
| 5 | `cargo run -- compare --spider` | + radar chart |
| 6 | `cargo run -- -w` | Cost column filled |
| 6 | `cargo run -- -q refactor` | FTS search |
| 7 | `cargo run -- rankings coding` | Ranking table |
| 7 | `cargo run -- -h` | Styled help |
