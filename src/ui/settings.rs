use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use rusqlite::Connection;
use std::io::{self, Write};

struct SettingDef {
    key: &'static str,
    label: &'static str,
    /// When non-empty, the value cycles through these. When empty, the row
    /// accepts free-form text input.
    options: &'static [&'static str],
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

/// All known settings, in display order. Add new ones here.
const DEFS: &[SettingDef] = &[
    SettingDef {
        key: "picker.view",
        label: "Picker view (-i, -c)",
        options: &["tree", "list"],
    },
    SettingDef {
        key: "picker.default_sort",
        label: "Default sort (picker)",
        options: &["time", "cost", "tokens", "calls"],
    },
    SettingDef {
        key: "heatmap.palette",
        label: "Heatmap palette",
        options: &["teal", "green", "blue", "amber"],
    },
    SettingDef {
        key: "ui.theme",
        label: "UI theme",
        options: &["dark", "light"],
    },
];

/// Render the settings picker. Changes are written to the DB only on confirm
/// (Enter), so Esc discards. Returns the keys that changed.
pub fn run_settings_picker(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut values: Vec<String> = DEFS
        .iter()
        .map(|d| crate::db::get_setting(conn, d.key, "").unwrap_or_default())
        .collect();

    let confirmed = run_inner(&mut values)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    if !confirmed {
        return Ok(Vec::new());
    }
    let mut changed = Vec::new();
    for (i, def) in DEFS.iter().enumerate() {
        if values[i].is_empty() {
            continue;
        }
        let prev = crate::db::get_setting(conn, def.key, "")?;
        if prev != values[i] {
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![def.key, values[i]],
            )?;
            changed.push(def.key.to_string());
        }
    }
    Ok(changed)
}

/// Inner picker loop. Returns Ok(true) on Enter (commit), Ok(false) on Esc.
fn run_inner(values: &mut [String]) -> io::Result<bool> {
    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;
    let (width, _) = terminal::size().unwrap_or((80, 24));
    let max_rows = DEFS.len();

    // Reserve vertical space: header + setting rows + hints + bottom blank
    let lines = 1 + max_rows + 2 + 1;
    for _ in 0..lines {
        stdout.write_all(b"\r\n")?;
    }
    stdout.flush()?;
    queue!(stdout, cursor::MoveUp(lines as u16))?;
    stdout.flush()?;
    let (_, start_row) = cursor::position().unwrap_or((0, 0));

    let mut selected: usize = 0;
    let mut editing: Option<String> = None; // Some(buf) when in text-input mode

    loop {
        // Header
        queue!(stdout, cursor::MoveTo(0, start_row), Clear(ClearType::CurrentLine))?;
        queue!(
            stdout,
            SetForegroundColor(Color::AnsiValue(43)),
            Print("  "),
            Print("\x1b[1mSettings\x1b[0m"),
            ResetColor,
            Print("  "),
            SetForegroundColor(Color::AnsiValue(242)),
            Print("↑↓ navigate · ←→ change · enter save · esc cancel"),
            ResetColor,
            Print("\r\n"),
        )?;

        // Setting rows
        for (i, def) in DEFS.iter().enumerate() {
            queue!(stdout, Clear(ClearType::CurrentLine))?;
            let is_sel = i == selected;
            if is_sel {
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(43)),
                    Print("▸ "),
                    ResetColor,
                )?;
            } else {
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(246)),
                    Print("  "),
                    ResetColor,
                )?;
            }

            // Label
            let label_color = if is_sel { 250 } else { 246 };
            queue!(
                stdout,
                SetForegroundColor(Color::AnsiValue(label_color)),
                Print(format!("{:<24}", def.label)),
                ResetColor,
            )?;

            if def.options.is_empty() {
                // Free-form text
                let is_editing = editing.is_some() && i == selected;
                let display = if is_editing {
                    format!("[ {}▏", editing.as_deref().unwrap_or(""))
                } else if values[i].is_empty() {
                    "[ empty ]".to_string()
                } else {
                    format!("[ {} ]", values[i])
                };
                let color = if is_editing { 43 } else { 250 };
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(color)),
                    Print(display),
                    ResetColor,
                )?;
            } else {
                // Cycled options: < prev | current* | next >
                let opts = def.options;
                let cur = if values[i].is_empty() {
                    opts[0]
                } else {
                    values[i].as_str()
                };
                let idx = opts.iter().position(|o| *o == cur).unwrap_or(0);
                let prev = opts[(idx + opts.len() - 1) % opts.len()];
                let next = opts[(idx + 1) % opts.len()];

                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(242)),
                    Print("< "),
                    ResetColor,
                )?;
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(242)),
                    Print(prev),
                    ResetColor,
                )?;
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(242)),
                    Print(" | "),
                    ResetColor,
                )?;
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(43)),
                    Print("\x1b[1m"),
                    Print(opts[idx]),
                    Print("\x1b[0m"),
                    ResetColor,
                )?;
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(242)),
                    Print(" | "),
                    ResetColor,
                )?;
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(250)),
                    Print(next),
                    ResetColor,
                )?;
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(242)),
                    Print(" >"),
                    ResetColor,
                )?;
            }
            queue!(stdout, Print("\r\n"))?;
        }

        // Hint footer
        queue!(stdout, Clear(ClearType::CurrentLine))?;
        let hint_color = if editing.is_some() { 220 } else { 242 };
        let hint = if editing.is_some() {
            "  typing — enter to commit, esc to cancel input"
        } else if DEFS[selected].options.is_empty() {
            "  press 'e' to edit text, ←→ to skip"
        } else {
            "  ← → to change, enter to save, esc to cancel"
        };
        queue!(
            stdout,
            SetForegroundColor(Color::AnsiValue(hint_color)),
            Print(hint),
            ResetColor,
            Print("\r\n"),
        )?;

        // Bottom margin
        queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;

        let _ = width;
        stdout.flush()?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }

        // Editing mode: capture text, esc cancels, enter commits
        if let Some(buf) = editing.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    editing = None;
                }
                KeyCode::Enter => {
                    values[selected] = buf.clone();
                    editing = None;
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    buf.push(c);
                }
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                cleanup(&mut stdout, start_row, lines)?;
                return Ok(false);
            }
            KeyCode::Enter => {
                cleanup(&mut stdout, start_row, lines)?;
                return Ok(true);
            }
            KeyCode::Up | KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                selected = selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if selected + 1 < DEFS.len() {
                    selected += 1;
                }
            }
            KeyCode::Left | KeyCode::Char('h') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                cycle(&DEFS[selected], values, selected, -1);
            }
            KeyCode::Right | KeyCode::Char('l') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                cycle(&DEFS[selected], values, selected, 1);
            }
            KeyCode::Char('e') if DEFS[selected].options.is_empty() => {
                editing = Some(values[selected].clone());
            }
            _ => {}
        }
    }
}

fn cycle(def: &SettingDef, values: &mut [String], idx: usize, dir: i32) {
    if def.options.is_empty() {
        return;
    }
    let opts = def.options;
    let cur = if values[idx].is_empty() {
        opts[0]
    } else {
        values[idx].as_str()
    };
    let pos = opts.iter().position(|o| *o == cur).unwrap_or(0);
    let n = opts.len() as i32;
    let next = ((pos as i32 + dir).rem_euclid(n)) as usize;
    values[idx] = opts[next].to_string();
}

fn cleanup(stdout: &mut impl Write, start_row: u16, lines: usize) -> io::Result<()> {
    queue!(stdout, cursor::MoveTo(0, start_row))?;
    for _ in 0..lines {
        queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;
    }
    queue!(stdout, cursor::MoveTo(0, start_row))?;
    stdout.flush()?;
    Ok(())
}
