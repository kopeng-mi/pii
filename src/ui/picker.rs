use crate::ui::table::{compact_num, truncate};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use nucleo::{
    Config, Matcher, Utf32String,
    pattern::{CaseMatching, Normalization, Pattern},
};
use rusqlite::Connection;
use std::{
    cmp::Reverse,
    collections::HashSet,
    io::{self, Write},
};
pub struct Selection {
    pub id: String,
    pub file_path: String,
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter(stdout: &mut impl Write) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        queue!(stdout, cursor::Hide)?;
        stdout.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = queue!(stdout, ResetColor, cursor::Show);
        let _ = stdout.flush();
        let _ = terminal::disable_raw_mode();
    }
}

pub fn run_picker(
    conn: &Connection,
    days: Option<u32>,
    prompt: &str,
) -> rusqlite::Result<Option<Selection>> {
    // Pull a wider candidate set; the picker will refine via FTS + fuzzy at
    // keystroke time. Cap rows so we don't materialize tens of thousands on
    // startup. Increase this once `--all` lands.
    let sort = crate::db::get_setting(conn, "picker.default_sort", "time")?;
    let order = match sort.as_str() {
        "cost"   => "total_cost DESC, date DESC, time DESC",
        "tokens" => "total_tokens DESC, date DESC, time DESC",
        "calls"  => "total_calls DESC, date DESC, time DESC",
        _        => "date DESC, time DESC",
    };
    let sql = format!(
        "SELECT id, file_path, project, date, time, prompt, total_calls, total_tokens, total_cost, last_model, ai_name
         FROM sessions
         WHERE (?1 IS NULL OR date >= date('now', '-' || ?1 || ' days'))
         ORDER BY {}
         LIMIT 5000",
        order
    );
    let mut stmt = conn.prepare(&sql)?;
    let items = stmt
        .query_map([days], |row| {
            let date: String = row.get(3)?;
            let time: String = row.get(4)?;
            let project: String = row.get(2)?;
            let calls: u32 = row.get(6)?;
            let tokens: u64 = row.get(7)?;
            let cost: f64 = row.get(8)?;
            let model: String = row.get(9)?;
            let first_prompt: String = row.get(5)?;
            let ai_name: String = row.get::<_, Option<String>>(10)?.unwrap_or_default();
            // Name resolution: AI name > session_info name > first prompt
            let display_name = if !ai_name.is_empty() {
                ai_name
            } else {
                first_prompt
            };
            Ok((
                Selection {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                },
                format!(
                    "{} {}  {:<10} {:<20} {:>4} {:>7} {:>7}  {}",
                    date.get(5..).unwrap_or(&date),
                    time,
                    truncate(&project, 10),
                    truncate(&model, 20),
                    calls,
                    compact_num(tokens),
                    if cost > 0.0 {
                        format!("${cost:.2}")
                    } else {
                        "--".into()
                    },
                    display_name.replace(['\n', '\r'], " ")
                ),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if items.is_empty() {
        println!("No sessions found.");
        return Ok(None);
    }

    pick(conn, items, prompt).map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))
}

pub fn pick(
    conn: &Connection,
    items: Vec<(Selection, String)>,
    prompt: &str,
) -> io::Result<Option<Selection>> {
    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let max_rows = items
        .len()
        .min(((height as usize * 2) / 5).max(3).saturating_sub(2));
    // lines = counter line + item rows + prompt line
    let lines = max_rows + 2;

    // Reserve vertical space by scrolling, then park cursor at start
    for _ in 0..lines {
        stdout.write_all(b"\r\n")?;
    }
    stdout.flush()?;
    // Move back up to where we'll draw
    queue!(stdout, cursor::MoveUp(lines as u16))?;
    stdout.flush()?;
    let (_, start_row) = cursor::position().unwrap_or((0, 0));

    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut query = String::new();
    let mut selected = 0usize;

    // Pre-compute UTF-32 haystacks once (was rebuilt every keystroke).
    let haystacks: Vec<Utf32String> = items.iter().map(|(_, text)| Utf32String::from(text.as_str())).collect();

    // Last keystroke results — skip the whole pipeline if nothing changed.
    let mut last_query: String = String::new();
    let mut last_matches: Vec<usize> = (0..items.len()).collect();
    let mut last_positions: Vec<Vec<u32>> = vec![Vec::new(); items.len()];

    let result = loop {
        if query != last_query {
            last_query.clear();
            last_query.push_str(&query);

            // 1. FTS5 pre-filter — only scan when the query has 2+ chars and
            //    no FTS-special chars (so plain substring still works for "gpt").
            //    Returns an ordered list of candidate indices into `items`.
            let fts_candidates: Option<Vec<usize>> = if query.len() >= 2 && !query.contains(':') {
                fts_candidate_indices(conn, &items, &query)
            } else {
                None
            };

            // 2. Build a quick-lookup from id -> index for joining FTS hits.
            let id_to_idx: std::collections::HashMap<&str, usize> = items
                .iter()
                .enumerate()
                .map(|(i, (sel, _))| (sel.id.as_str(), i))
                .collect();

            let candidate_set: Vec<usize> = match fts_candidates {
                Some(v) if !v.is_empty() => v,
                // FTS had no hits (or wasn't queried) — fall back to fuzzy over all.
                _ => (0..items.len()).collect(),
            };

            // 3. Fuzzy-rank candidates, returning (index, score, positions).
            let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
            let mut scored: Vec<(usize, u32, Vec<u32>)> = Vec::with_capacity(candidate_set.len());
            if query.is_empty() {
                // Empty query: preserve the original date-DESC order, no scoring.
                scored = candidate_set.into_iter().map(|i| (i, 0, Vec::new())).collect();
            } else {
                for idx in candidate_set {
                    let haystack = &haystacks[idx];
                    let mut positions = Vec::new();
                    if let Some(score) = pattern.indices(haystack.slice(..), &mut matcher, &mut positions) {
                        scored.push((idx, score, positions));
                    }
                }
                scored.sort_by_key(|item| Reverse(item.1));
            }

            last_matches = scored.iter().map(|(i, _, _)| *i).collect();
            last_positions = vec![Vec::new(); items.len()];
            for (i, _, p) in scored {
                last_positions[i] = p;
            }
            // Suppress unused var warning if FTS brought no improvement.
            let _ = id_to_idx;
        }
        let matches = &last_matches;
        let positions_map = &last_positions;
        selected = selected.min(matches.len().saturating_sub(1));

        // Move to start and clear our entire region
        queue!(stdout, cursor::MoveTo(0, start_row))?;

        // Counter line
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::AnsiValue(246)),
            Print(format!("  {}/{}", matches.len(), items.len())),
            ResetColor,
            Print("\r\n")
        )?;

        // Item rows
        let visible_start = selected.saturating_add(1).saturating_sub(max_rows);
        let visible_end = matches.len().min(visible_start + max_rows);
        let rendered_rows = visible_end - visible_start;
        for (match_index, &item_index) in matches
            .iter()
            .enumerate()
            .take(visible_end)
            .skip(visible_start)
        {
            queue!(stdout, Clear(ClearType::CurrentLine))?;
            let marker = if match_index == selected {
                "▸ "
            } else {
                "  "
            };
            queue!(
                stdout,
                SetForegroundColor(if match_index == selected {
                    Color::AnsiValue(43)
                } else {
                    Color::AnsiValue(246)
                }),
                Print(marker),
                ResetColor
            )?;
            draw_text(
                &mut stdout,
                &items[item_index].1,
                &positions_map[item_index],
                width.saturating_sub(4) as usize,
                match_index == selected,
            )?;
            queue!(stdout, Print("\r\n"))?;
        }
        // Blank remaining rows
        for _ in rendered_rows..max_rows {
            queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;
        }

        // Prompt line
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::AnsiValue(43)),
            Print(format!("  {prompt}▸ ")),
            ResetColor,
            Print(&query)
        )?;
        stdout.flush()?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Enter if !matches.is_empty() => break Some(matches[selected]),
            KeyCode::Esc | KeyCode::Char('c' | 'g')
                if key.modifiers.contains(KeyModifiers::CONTROL) || key.code == KeyCode::Esc =>
            {
                break None;
            }
            KeyCode::Up | KeyCode::Char('p' | 'k')
                if key.code == KeyCode::Up || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                selected = selected.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('n' | 'j')
                if key.code == KeyCode::Down || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                selected = (selected + 1).min(matches.len().saturating_sub(1))
            }
            KeyCode::Backspace => {
                query.pop();
                selected = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                query.push(c);
                selected = 0;
            }
            _ => {}
        }
    };

    // Clean up: clear the picker region
    queue!(stdout, cursor::MoveTo(0, start_row))?;
    for _ in 0..lines {
        queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;
    }
    queue!(stdout, cursor::MoveTo(0, start_row))?;
    stdout.flush()?;
    Ok(result.map(|index| items.into_iter().nth(index).unwrap().0))
}

/// Run an FTS5 MATCH query against sessions_fts and return the matching
/// session IDs in bm25 order (best first). Returns Ok(None) on any DB error
/// so the caller can fall back to fuzzy.
fn fts_candidate_ids(conn: &Connection, query: &str) -> rusqlite::Result<Option<Vec<String>>> {
    // Escape FTS5 special chars; use prefix-match on each token for substring feel.
    let cleaned: String = query
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { ' ' })
        .collect();
    let fts_query = cleaned
        .split_whitespace()
        .map(|tok| format!("\"{}\"*", tok.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ");
    if fts_query.is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT s.id FROM sessions s
         JOIN sessions_fts f ON f.rowid = s.rowid
         WHERE sessions_fts MATCH ?1
         ORDER BY bm25(sessions_fts), s.date DESC, s.time DESC
         LIMIT 1000",
    )?;
    let rows = stmt.query_map([&fts_query], |row| row.get::<_, String>(0))?;
    let ids: Vec<String> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(ids))
}

/// Map FTS5-matched session IDs back to indices in the `items` slice, in
/// FTS-ranked order. Items not in the FTS hit set are dropped from the
/// candidate pool so fuzzy only scores likely matches.
fn fts_candidate_indices(
    conn: &Connection,
    items: &[(Selection, String)],
    query: &str,
) -> Option<Vec<usize>> {
    let ids = fts_candidate_ids(conn, query).ok().flatten()?;
    if ids.is_empty() {
        return Some(Vec::new());
    }
    let mut pos: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::with_capacity(items.len());
    for (i, (sel, _)) in items.iter().enumerate() {
        pos.insert(sel.id.as_str(), i);
    }
    let indices: Vec<usize> = ids
        .iter()
        .filter_map(|id| pos.get(id.as_str()).copied())
        .collect();
    Some(indices)
}

fn draw_text(
    stdout: &mut impl Write,
    text: &str,
    positions: &[u32],
    max_width: usize,
    selected: bool,
) -> io::Result<()> {
    let positions: HashSet<u32> = positions.iter().copied().collect();
    let display = truncate(text, max_width);
    // Build the entire line as one string with state-change escapes only.
    // Was: ~display.chars().count() * 3 queue! calls per line.
    let mut buf = String::with_capacity(display.len() * 2);
    let mut last_matched = false;
    let mut last_selected = false;
    for (index, ch) in display.chars().enumerate() {
        let matched = positions.contains(&(index as u32));
        if matched != last_matched || (matched == last_matched && selected != last_selected) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_results_rank_and_expose_match_positions() {
        let pattern = Pattern::parse("clsn", CaseMatching::Ignore, Normalization::Smart);
        let text = Utf32String::from("07-25 pii claude-sonnet".to_string());
        let mut positions = Vec::new();
        assert!(
            pattern
                .indices(
                    text.slice(..),
                    &mut Matcher::new(Config::DEFAULT),
                    &mut positions
                )
                .is_some()
        );
        assert_eq!(positions.len(), 4);
    }
}
