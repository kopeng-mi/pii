# pii

CLI for browsing your pi coding agent sessions and LLM model stats. Fast, terminal-native, no daemon.

## Install

```
cargo install --path .
```

Binary lands in `~/.cargo/bin/`. Make sure that's on your `PATH`.

## Usage

```
pii              # interactive picker to continue a session
pii -c           # same as above
pii -i           # inspect a session (use --calls for per-call rows)
pii -t           # today's sessions
pii -w           # past 7 days
pii -m           # past 30 days
pii -s           # summary dashboard
pii -H           # activity heatmap (180 days)
pii -q "query"   # FTS search across projects, prompts, models
pii model        # model detail card or picker
pii compare      # side-by-side model comparison (add --spider for radar)
pii rankings     # leaderboard by coding / math / general
pii -h           # help

# Refreshes
pii -r           # wipe session cache and reparse all JSONL files
pii -R           # force-refresh model data from APIs (LLM-Stats, Artificial Analysis)
```

By default `pii` only parses new or changed session files. `-r` forces a full resync.

API keys for live model data go in `~/.pi/.env` (or process env):

```
LLM_STATS_API_KEY=...
ARTIFICIALANALYSIS_API_KEY=...
```

Without keys `pii` still works on local session data.

## Data locations

- Sessions are read from `~/.pi/agent/sessions/` (where pi writes them).
- DB cache lives at `%LOCALAPPDATA%\pii\pii.db` on Windows, `~/.local/share/pii/pii.db` elsewhere.

## Notes

First run scans and parses all session JSONL files. After that it's incremental: only new or changed files get re-parsed. The picker is fuzzy on top of FTS5, so searching across thousands of sessions stays fast.

## Tree picker

`pii -i` and `pii -c` open a tree view by default. The first column is a list of project folders; the second column is the session tree inside the open folder. Move with the arrow keys.

- `↑` / `↓`: move the cursor. When the cursor lands on a folded project folder, the folder opens and any other open folder closes.
- `←`: collapse the current folder, or jump to the parent session.
- `→`: open the current folder, or step into the first child session.
- `Enter`: select the highlighted session.
- `Esc`: quit.

Each session shows a name, a cost column, and a relative age (`1d`, `2h`, `today`). Sessions without children drop the chevron so the column lines up.

To switch back to the flat fuzzy picker, set `pii settings` → `Picker view` to `list`.
