# pii Optimization Plan

> Comprehensive plan for performance, caching, rendering speed, and visual polish.
> Each section is self-contained. Implement top-to-bottom; earlier items have highest impact.

---

## Current Stats
- **DB**: 5.7MB SQLite — 483 sessions, 21K calls, 699 models
- **Source**: ~2,877 lines Rust across 19 files
- **Codebase**: no PRAGMAs, no indexes, no transaction batching, correlated subqueries on every view, picker re-allocates fuzzy haystacks every keystroke

---

## 1. SQLite PRAGMA Tuning (db.rs — `init_db`)

**Problem**: Default SQLite settings (rollback journal, synchronous=FULL, tiny cache). Every write does an fsync.

**Fix**: Add these PRAGMAs at the top of `init_db`'s `execute_batch`, before any CREATE TABLE:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -16000;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 268435456;
PRAGMA busy_timeout = 5000;
```

**Why**: WAL mode allows concurrent reads during writes. `synchronous=NORMAL` skips redundant fsyncs in WAL mode. `cache_size=-16000` gives ~16MB page cache (default is 2MB). `mmap_size` memory-maps 256MB of the DB for zero-copy reads. `temp_store=MEMORY` keeps temp tables in RAM.

**Sources**:
- https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/
- https://phiresky.github.io/blog/2020/sqlite-performance-tuning/
- https://sqlite.org/wal.html

---

## 2. Add Missing Indexes (db.rs — `init_db`)

**Problem**: No indexes on `sessions.date` or `calls.session_id`. Every date-filtered query and every call lookup does a full table scan.

**Fix**: Add at the end of the `execute_batch` block:

```sql
CREATE INDEX IF NOT EXISTS idx_sessions_date ON sessions(date);
CREATE INDEX IF NOT EXISTS idx_calls_session ON calls(session_id);
CREATE INDEX IF NOT EXISTS idx_scores_model ON scores(model_id);
```

**Why**: `sessions.date` is filtered in `-t`, `-w`, `-m`, heatmap, picker. `calls.session_id` is used in every correlated subquery and in inspect. `scores.model_id` is joined in rankings/compare/detail.

---

## 3. Transaction Batching for Sync (session/parser.rs — `sync_sessions`)

**Problem**: Each `insert_session` call auto-commits (implicit transaction per INSERT). With 483 sessions × (1 session INSERT + N call INSERTs + 1 DELETE), that's thousands of individual fsyncs.

**Fix**: Wrap the entire file-scanning loop in a single transaction:

```rust
// Before the entries loop:
conn.execute_batch("BEGIN")?;

// ... existing loop body ...

// After the loop:
conn.execute_batch("COMMIT")?;
```

That's it. Two lines. The `insert_session` function stays unchanged.

**Why**: SQLite batches all writes into a single WAL frame. Benchmarks show 50-100x speedup for bulk inserts. This is the single biggest win.

---

## 4. Stream-Parse JSONL with BufReader (session/parser.rs — `parse_session_file`)

**Problem**: `fs::read_to_string(path)` loads entire JSONL file into one String. Large sessions can be several MB.

**Fix**: Replace in `parse_session_file`:

```rust
// Old:
let content = fs::read_to_string(path).ok()?;
for line in content.lines() { ... }

// New:
use std::io::{BufRead, BufReader};
let file = fs::File::open(path).ok()?;
let reader = BufReader::new(file);
for line in reader.lines() {
    let line = match line { Ok(l) => l, Err(_) => continue };
    // ... rest of parsing unchanged, just use &line instead of line ...
}
```

**Why**: Streams line-by-line, peak memory drops from file-size to single-line size. No logic changes needed inside the loop.

---

## 5. Denormalize `last_model` into Sessions Table

**Problem**: Picker SQL and table SQL both do a correlated subquery per row:
```sql
(SELECT model FROM calls WHERE session_id = sessions.id ORDER BY id DESC LIMIT 1)
```
This fires once per session row. With 483 sessions, that's 483 subqueries.

**Fix** (3 parts):

### 5a. Add column (db.rs `init_db`)
```sql
ALTER TABLE sessions ADD COLUMN last_model TEXT DEFAULT '';
```
Use `IF NOT EXISTS` pattern or just ignore the error if column already exists (SQLite doesn't support `ADD COLUMN IF NOT EXISTS`, so wrap in an `execute` that ignores `duplicate column` errors).

### 5b. Populate during sync (session/parser.rs `insert_session`)
Set `last_model` to the model from the last call in `call_rows`:
```rust
let last_model = calls.last().map(|c| c.model.as_str()).unwrap_or("");
```
Add it to the INSERT statement's column list and values.

### 5c. Remove subqueries (ui/picker.rs, ui/table.rs)
Replace `(SELECT model FROM calls WHERE ...)` with just `sessions.last_model` in all SQL.

**Why**: Eliminates N correlated subqueries. The column is ~30 bytes per row, negligible.

---

## 6. Pre-Compute Picker Fuzzy Haystacks (ui/picker.rs — `pick`)

**Problem**: Inside the render loop (runs every keypress), every item creates a new `Utf32String`:
```rust
let haystack = Utf32String::from(text.as_str());
```
With 483 items, that's 483 allocations per keystroke.

**Fix**: Build the haystacks once before the loop:

```rust
// Before the loop:
let haystacks: Vec<Utf32String> = items.iter().map(|(_, text)| Utf32String::from(text.as_str())).collect();

// Inside the loop, replace:
//   let haystack = Utf32String::from(text.as_str());
// with:
//   let haystack = &haystacks[index];
// And use haystack.slice(..) as before.
```

**Why**: Moves O(N) allocations from per-keypress to one-time init. Nucleo's `Utf32String` stores UTF-32 data; re-encoding per keystroke is pure waste.

---

## 7. Reduce Picker Write Syscalls (ui/picker.rs — `draw_text` + render loop)

**Problem**: Each character in `draw_text` calls `queue!` 2-3 times (SetForegroundColor + SetAttribute + Print). For a 80-char line × 15 rows = ~3600 queue calls per frame.

**Fix**: Build the entire line as a single `String` with embedded ANSI escape codes, then write once:

```rust
fn draw_text(...) -> io::Result<()> {
    let mut buf = String::with_capacity(text.len() * 2);
    let mut last_matched = false;
    let mut last_selected = false;
    for (index, ch) in display.chars().enumerate() {
        let matched = positions.contains(&(index as u32));
        // Only emit escape code when state changes
        if matched != last_matched || (!matched && selected != last_selected) {
            if matched {
                buf.push_str("\x1b[38;5;43;1m");
            } else if selected {
                buf.push_str("\x1b[37;1m");
            } else {
                buf.push_str("\x1b[38;5;250;0m");
            }
            last_matched = matched;
            last_selected = selected;
        }
        buf.push(ch);
    }
    buf.push_str("\x1b[0m");
    queue!(stdout, Print(buf))?;
    Ok(())
}
```

**Why**: One write per line instead of per-character. Fewer syscalls, smoother rendering.

---

## 8. Visual Polish

### 8a. Summary Dashboard (ui/summary.rs)
- Add unicode metric icons: `◈ Sessions`, `⚡ Calls`, `◆ Tokens`, `$ Cost`
- Add a "Today vs Avg" comparison row (query today's counts vs all-time average)
- Add "Most Active Project" row: `SELECT project, COUNT(*) ... GROUP BY project ORDER BY ... LIMIT 1`

### 8b. Heatmap (ui/heatmap.rs)
- Add month labels above the grid. Walk the weeks, print month abbreviation when the month changes.
- Render format: `      Jan         Feb         Mar ...` above the grid rows.

### 8c. Table Output (ui/table.rs)
- Right-align numeric columns properly with consistent fixed widths
- Add a dim footer line: `  ─── end ─── ` after the last row

### 8d. Inspect View (main.rs `inspect_session`)
- Remove the cursor-moving hack (`\x1b[{}A` / `\x1b[{}B`) for re-printing token summary. Instead, compute `total_in`/`total_out` in a first pass before printing any output.
- Add a mini-sparkline showing token usage trend across calls

### 8e. Help Screen (main.rs `print_custom_help`)
- Add version number dynamically: `env!("CARGO_PKG_VERSION")`
- Group commands with subtle dim box-drawing borders

---

## 9. API Fetch Optimization (models/api.rs)

**Problem**: `fetch_models` downloads all models from both APIs sequentially. If one is slow, everything blocks.

**Fix**: Since we can't use tokio (per AGENTS.md), use `std::thread` to fetch both APIs concurrently:

```rust
use std::thread;

let llm_handle = thread::spawn(|| fetch_llm_stats());
let aa_handle = thread::spawn(|| fetch_aa());

let llm_result = llm_handle.join().ok().flatten();
let aa_result = aa_handle.join().ok().flatten();
// ... merge as before
```

Extract the LLM-Stats and AA fetch logic into separate functions that return their results. Merge after both complete.

**Why**: Two HTTP calls run in parallel. Total time = max(llm, aa) instead of sum.

---

## 10. Fuzzy Match Cost Estimation Cache (models/fuzzy.rs)

**Problem**: `estimate_cost` runs nucleo fuzzy matching for every call row during sync. With 21K calls, that's 21K × 699 candidates = 14.7M comparisons.

**Fix**: Add a simple `HashMap<String, Option<usize>>` cache mapping model names to their matched candidate index:

```rust
pub fn estimate_cost_cached(
    model_name: &str,
    in_tokens: u32,
    out_tokens: u32,
    candidates: &[UnifiedModel],
    cache: &mut HashMap<String, Option<usize>>,
) -> Option<f64> {
    let idx = cache.entry(model_name.to_string()).or_insert_with(|| {
        // ... existing matching logic, but return Some(index) instead of Some(candidate)
    });
    idx.and_then(|i| {
        let m = &candidates[i];
        let cost = (in_tokens as f64 / 1e6) * m.input_price + (out_tokens as f64 / 1e6) * m.output_price;
        if cost > 0.0 { Some(cost) } else { None }
    })
}
```

Pass `&mut cache` from `sync_sessions`. The cache persists across all calls within one sync run.

**Why**: Typical sessions use 2-3 distinct models. After the first match, subsequent calls with the same model name are O(1) HashMap lookups instead of O(N) fuzzy scans.

---

## Implementation Order (by impact)

| Priority | Task | File(s) | Impact |
|----------|------|---------|--------|
| 🔴 1 | Transaction batching | parser.rs | ~50x faster sync |
| 🔴 2 | SQLite PRAGMAs | db.rs | ~5-10x faster I/O |
| 🔴 3 | Add indexes | db.rs | ~10x faster filtered queries |
| 🟡 4 | Denormalize last_model | db.rs, parser.rs, picker.rs, table.rs | Eliminates 483 subqueries |
| 🟡 5 | Stream-parse BufReader | parser.rs | Lower memory, same speed |
| 🟡 6 | Pre-compute haystacks | picker.rs | Smoother picker typing |
| 🟡 7 | Fuzzy match cache | fuzzy.rs, parser.rs | ~1000x fewer comparisons |
| 🟢 8 | Reduce write syscalls | picker.rs | Snappier picker rendering |
| 🟢 9 | Parallel API fetch | api.rs | Halves model sync time |
| 🟢 10 | Visual polish | summary.rs, heatmap.rs, table.rs, main.rs | Aesthetics |

---

## Sources
- [SQLite PRAGMA Cheatsheet](https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/)
- [SQLite Performance Tuning](https://phiresky.github.io/blog/2020/sqlite-performance-tuning/)
- [SQLite WAL Overview](https://sqlite.org/wal.html)
- [rusqlite GitHub](https://github.com/rusqlite/rusqlite)
- [Nucleo fuzzy matcher](https://docs.rs/nucleo/latest/nucleo/)
