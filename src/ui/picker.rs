use crate::ui::table::{compact_num, truncate};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{Color, Print, ResetColor, SetAttribute, SetForegroundColor},
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
    let mut stmt = conn.prepare(
        "SELECT id, file_path, project, date, time, prompt, total_calls, total_tokens, total_cost,
                COALESCE((SELECT model FROM calls WHERE session_id = sessions.id ORDER BY id DESC LIMIT 1), 'unknown')
         FROM sessions
         WHERE (?1 IS NULL OR date >= date('now', '-' || ?1 || ' days'))
         ORDER BY date, time",
    )?;
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
                    first_prompt.replace(['\n', '\r'], " ")
                ),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if items.is_empty() {
        println!("No sessions found.");
        return Ok(None);
    }

    pick(items, prompt).map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))
}

pub fn pick(items: Vec<(Selection, String)>, prompt: &str) -> io::Result<Option<Selection>> {
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

    let result = loop {
        let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
        let mut matches = items
            .iter()
            .enumerate()
            .filter_map(|(index, (_, text))| {
                let haystack = Utf32String::from(text.as_str());
                let mut positions = Vec::new();
                let score = pattern.indices(haystack.slice(..), &mut matcher, &mut positions)?;
                Some((index, score, positions))
            })
            .collect::<Vec<_>>();
        if !query.is_empty() {
            matches.sort_by_key(|item| Reverse(item.1));
        }
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
        for (match_index, (item_index, _, positions)) in matches
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
                &items[*item_index].1,
                positions,
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
            KeyCode::Enter if !matches.is_empty() => break Some(matches[selected].0),
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

fn draw_text(
    stdout: &mut impl Write,
    text: &str,
    positions: &[u32],
    max_width: usize,
    selected: bool,
) -> io::Result<()> {
    let positions: HashSet<u32> = positions.iter().copied().collect();
    let display = truncate(text, max_width);
    for (index, ch) in display.chars().enumerate() {
        let matched = positions.contains(&(index as u32));
        if matched {
            queue!(
                stdout,
                SetForegroundColor(Color::AnsiValue(43)),
                SetAttribute(crossterm::style::Attribute::Bold)
            )?;
        } else if selected {
            queue!(
                stdout,
                SetForegroundColor(Color::White),
                SetAttribute(crossterm::style::Attribute::Bold)
            )?;
        } else {
            queue!(
                stdout,
                SetForegroundColor(Color::AnsiValue(250)),
                SetAttribute(crossterm::style::Attribute::Reset)
            )?;
        }
        queue!(stdout, Print(ch))?;
    }
    queue!(
        stdout,
        ResetColor,
        SetAttribute(crossterm::style::Attribute::Reset)
    )?;
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
