mod cli;
mod db;
mod models;
mod session;
mod ui;

use chrono::{Duration, Local};
use clap::Parser;
use cli::Cli;
use rusqlite::Connection;
use std::process::Command;

fn inspect_session(
    conn: &rusqlite::Connection,
    session_id: &str,
    show_calls: bool,
) -> rusqlite::Result<()> {
    // Fetch session details
    let sql = "SELECT id, project, date, time, prompt, total_calls, total_tokens, total_cost, errors FROM sessions WHERE id = ?";
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([session_id])?;

    if let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let project: String = row.get(1)?;
        let date: String = row.get(2)?;
        let time: String = row.get(3)?;
        let prompt: String = row.get(4)?;
        let calls: u32 = row.get(5)?;
        let tokens: u32 = row.get(6)?;
        let cost: f64 = row.get(7)?;
        let errs: u32 = row.get(8)?;

        println!("\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mSession Inspection\x1b[0m");
        println!("  \x1b[38;5;246mID:\x1b[0m {}", id);
        println!("  \x1b[38;5;246mProject:\x1b[0m {}", project);
        println!("  \x1b[38;5;246mDate:\x1b[0m {} {}", date, time);
        println!("  \x1b[38;5;246mCalls:\x1b[0m {}", calls);
        println!("  \x1b[38;5;246mTokens:\x1b[0m {} (in: {}, out: {})", tokens, "—", "—"); // We will aggregate this
        println!("  \x1b[38;5;246mCost:\x1b[0m ${:.2}", cost);
        if errs > 0 {
            println!("  \x1b[38;5;196mErrors:\x1b[0m {}", errs);
        }
        println!("\n  \x1b[38;5;246mPrompt:\x1b[0m");

        for line in prompt.lines() {
            println!("    \x1b[38;5;250m{}\x1b[0m", line);
        }
        println!();

        if show_calls {
            let call_sql = "SELECT id, model, input_tokens, output_tokens, cost, is_error FROM calls WHERE session_id = ? ORDER BY id ASC";
            let mut cstmt = conn.prepare(call_sql)?;
            // Collect rows to compute max tokens for the graph
            let call_rows = cstmt.query_map([session_id], |crow| {
                Ok((
                    crow.get::<_, String>(1)?, // model
                    crow.get::<_, u32>(2)?,    // in
                    crow.get::<_, u32>(3)?,    // out
                    crow.get::<_, f64>(4)?,    // cost
                    crow.get::<_, bool>(5)?,   // err
                ))
            })?.collect::<Result<Vec<_>, _>>()?;

            if call_rows.is_empty() {
                return Ok(());
            }

            let max_call_tokens = call_rows.iter().map(|&(_, i, o, _, _)| i + o).max().unwrap_or(1) as f64;
            let total_in: u32 = call_rows.iter().map(|&(_, i, _, _, _)| i).sum();
            let total_out: u32 = call_rows.iter().map(|&(_, _, o, _, _)| o).sum();

            // Re-print Token summary with accurate in/out if available
            print!("\x1b[{}A", 7 + prompt.lines().count()); // Move up to Tokens line (approx)
            println!("  \x1b[38;5;246mTokens:\x1b[0m {} (in: {}, out: {}) \x1b[K", crate::ui::table::compact_num(tokens as u64), crate::ui::table::compact_num(total_in as u64), crate::ui::table::compact_num(total_out as u64));
            print!("\x1b[{}B", 6 + prompt.lines().count()); // Move back down

            println!("  \x1b[38;5;246mCall Timeline\x1b[0m");
            println!(
                "  \x1b[38;5;237m────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
            );
            for (idx, (m, it, ot, c, is_err)) in call_rows.into_iter().enumerate() {
                let err_str = if is_err {
                    "\x1b[38;5;196mERR\x1b[0m"
                } else {
                    "   "
                };
                let model_fmt = crate::ui::table::truncate(&m, 20);
                let bar = crate::ui::table::make_bar((it + ot) as f64, max_call_tokens, 12);
                let total_t = crate::ui::table::compact_num((it + ot) as u64);

                println!(
                    "  \x1b[38;5;242m{:>2}\x1b[0m │ \x1b[38;5;114m{:<20}\x1b[0m  {} {:>5}  \x1b[38;5;246min:\x1b[0m {:<5} \x1b[38;5;246mout:\x1b[0m {:<5} \x1b[38;5;220m${:>6.4}\x1b[0m  {}",
                    idx + 1, model_fmt, bar, total_t, crate::ui::table::compact_num(it as u64), crate::ui::table::compact_num(ot as u64), c, err_str
                );
            }
            println!();
        }
    }

    Ok(())
}

fn main() -> rusqlite::Result<()> {
    // Load env vars, but ignore errors if .env is missing
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    let db_path = db::get_db_path();
    let conn = Connection::open(&db_path)?;
    db::init_db(&conn)?;

    // Parse newly added sessions
    session::parser::sync_sessions(&conn)?;

    let today = Local::now();

    if cli.heatmap {
        ui::heatmap::print_heatmap(&conn)?;
        return Ok(());
    }

    if cli.summary {
        ui::summary::print_summary(&conn)?;
        return Ok(());
    }

    if cli.today {
        let date_str = today.format("%Y-%m-%d").to_string();
        ui::table::print_sessions(
            &conn,
            Some(100),
            &format!("Today [{}]", date_str),
            &cli.sort,
            Some(&date_str),
            None,
            cli.query.as_deref(),
        )?;
        return Ok(());
    }

    if cli.week {
        let start_date = (today - Duration::days(7)).format("%Y-%m-%d").to_string();
        ui::table::print_sessions(
            &conn,
            Some(100),
            "Past 7 days",
            &cli.sort,
            None,
            Some(&start_date),
            cli.query.as_deref(),
        )?;
        return Ok(());
    }

    if cli.month {
        let start_date = (today - Duration::days(30)).format("%Y-%m-%d").to_string();
        ui::table::print_sessions(
            &conn,
            Some(200),
            "Past 30 days",
            &cli.sort,
            None,
            Some(&start_date),
            cli.query.as_deref(),
        )?;
        return Ok(());
    }

    if cli.query.is_some() {
        ui::table::print_sessions(
            &conn,
            Some(100),
            &format!("Search: {}", cli.query.as_ref().unwrap()),
            &cli.sort,
            None,
            None,
            cli.query.as_deref(),
        )?;
        return Ok(());
    }

    // Process Models Subcommand
    if let Some(cli::Commands::Model { query, refresh }) = &cli.command {
        models::api::refresh_if_needed(&conn, *refresh)?;
        
        let target_id = if let Some(q) = query {
            // Very basic matching for direct query logic
            let mut stmt = conn.prepare("SELECT id FROM models WHERE id LIKE ? OR name LIKE ? LIMIT 1")?;
            let like_q = format!("%{}%", q);
            stmt.query_row([&like_q, &like_q], |r| r.get::<_, String>(0)).ok()
        } else {
            // Interactive picker
            models::detail::run_model_picker(&conn)?
        };

        if let Some(id) = target_id {
            models::detail::print_model_detail(&conn, &id)?;
        }
        return Ok(());
    }

    // Default behavior or explicit continue/inspect
    let is_inspect = cli.inspect || cli.calls;

    if let Some(selection) = ui::picker::run_picker(
        &conn,
        cli.days,
        if is_inspect { "inspect" } else { "continue" },
    )? {
        if is_inspect {
            // In Phase 3, -i explicitly implies we show calls if they exist.
            inspect_session(&conn, &selection.id, true)?;
        } else {
            println!(
                "  \x1b[38;5;43m▰\x1b[0m Resuming session \x1b[38;5;246m{}\x1b[0m",
                selection.file_path
            );
            let status = Command::new("pi")
                .arg("--session")
                .arg(&selection.file_path)
                .status();

            match status {
                Ok(s) => std::process::exit(s.code().unwrap_or(1)),
                Err(e) => {
                    eprintln!(
                        "Failed to launch `pi --session {}`: {}",
                        selection.file_path, e
                    );
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
