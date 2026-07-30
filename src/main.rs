mod cli;
mod db;
mod models;
mod session;
mod ui;

use chrono::{Duration, Local};
use clap::Parser;
use cli::Cli;
use rusqlite::Connection;
use std::io::{self, Write};
use std::process::Command;

#[cfg(windows)]
fn which_exists(cmd: &str) -> bool {
    // Mirror std's Command search: walk PATH and check each dir for `cmd` with
    // PATHEXT extensions. If cmd already has an extension, only that one is tried.
    let has_ext = std::path::Path::new(cmd).extension().is_some();
    let exts: Vec<String> = if has_ext {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.BAT;.CMD".into())
            .split(';')
            .map(|s| s.to_string())
            .collect()
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for ext in &exts {
                let candidate = dir.join(format!("{}{}", cmd, ext));
                if candidate.is_file() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(not(windows))]
fn which_exists(_cmd: &str) -> bool {
    true
}

/// Print a styled prompt to stderr and read a single line from stdin.
fn prompt_input(question: &str, default: Option<&str>) -> io::Result<String> {
    eprint!("  \x1b[38;5;246m{}\x1b[0m ", question);
    if let Some(d) = default {
        eprint!("\x1b[38;5;242m[{}]\x1b[0m: ", d);
    } else {
        eprint!("\x1b[38;5;242m:\x1b[0m ");
    }
    io::stderr().flush().ok();
    let mut buf = String::new();
    let n = io::stdin().read_line(&mut buf)?;
    if n == 0 {
        // EOF (piped empty stdin). Treat as default if available.
        return Ok(default.unwrap_or("").to_string());
    }
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        Ok(default.unwrap_or("").to_string())
    } else {
        Ok(trimmed.to_string())
    }
}
use ratatui::style::Color;

fn inspect_session(
    conn: &rusqlite::Connection,
    session_id: &str,
    show_calls: bool,
) -> rusqlite::Result<()> {
    // Fetch session details
    let sql = "SELECT id, project, date, time, prompt, total_calls, total_tokens, total_cost, errors, ai_name FROM sessions WHERE id = ?";
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([session_id])?;

    if let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let project: String = row.get(1)?;
        let date: String = row.get(2)?;
        let time: String = row.get(3)?;
        let prompt: String = row.get(4)?;
        let calls: u32 = row.get(5)?;
        let mut tokens: u64 = row.get::<_, u32>(6)? as u64;
        let mut cost: f64 = row.get(7)?;
        let errs: u32 = row.get(8)?;
        let ai_name: String = row.get::<_, Option<String>>(9)?.unwrap_or_default();

        let mut call_rows = Vec::new();
        let mut max_call_tokens = 1.0;
        let mut total_in = 0;
        let mut total_out = 0;

        if show_calls {
            let call_sql = "SELECT id, model, input_tokens, output_tokens, cost, is_error FROM calls WHERE session_id = ? ORDER BY id ASC";
            let mut cstmt = conn.prepare(call_sql)?;
            call_rows = cstmt.query_map([session_id], |crow| {
                Ok((
                    crow.get::<_, String>(1)?, // provider
                    crow.get::<_, String>(2)?, // model
                    crow.get::<_, u32>(3)?,    // in
                    crow.get::<_, u32>(4)?,    // out
                    crow.get::<_, f64>(5)?,    // cost
                    crow.get::<_, bool>(6)?,   // err
                ))
            })?.collect::<Result<Vec<_>, _>>()?;

            if !call_rows.is_empty() {
                max_call_tokens = call_rows.iter().map(|&(_, _, i, o, _, _)| i + o).max().unwrap_or(1) as f64;
                total_in = call_rows.iter().map(|&(_, _, i, _, _, _)| i).sum();
                total_out = call_rows.iter().map(|&(_, _, _, o, _, _)| o).sum();
                // Override tokens and cost with precise sum from calls
                tokens = (total_in + total_out) as u64;
                cost = call_rows.iter().map(|&(_, _, _, _, c, _)| c).sum();
            }
        }

        println!("\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mSession Inspection\x1b[0m");
        println!("  \x1b[38;5;246mID:\x1b[0m      {}", id);
        println!("  \x1b[38;5;246mProject:\x1b[0m {}", project);
        println!("  \x1b[38;5;246mDate:\x1b[0m    {} {}", date, time);
        if !ai_name.is_empty() {
            println!("  \x1b[38;5;246mName:\x1b[0m    \x1b[1m\x1b[38;5;51m{}\x1b[0m", ai_name);
        }
        println!("  \x1b[38;5;246mCalls:\x1b[0m   {}", calls);
        
        if total_in > 0 || total_out > 0 {
            println!("  \x1b[38;5;246mTokens:\x1b[0m  {} \x1b[38;5;242m(in: {}, out: {})\x1b[0m", 
                crate::ui::table::compact_num(tokens), 
                crate::ui::table::compact_num(total_in as u64), 
                crate::ui::table::compact_num(total_out as u64)
            );
        } else {
            println!("  \x1b[38;5;246mTokens:\x1b[0m  {}", crate::ui::table::compact_num(tokens));
        }
        println!("  \x1b[38;5;246mCost:\x1b[0m    \x1b[38;5;220m${:.4}\x1b[0m", cost);
        if errs > 0 {
            println!("  \x1b[38;5;196mErrors:\x1b[0m  {}", errs);
        }
        
        println!("\n  \x1b[38;5;246mPrompt:\x1b[0m");
        for line in prompt.lines() {
            println!("    \x1b[38;5;250m{}\x1b[0m", line);
        }
        println!();

        if !call_rows.is_empty() {
            println!("  \x1b[38;5;246mCall Timeline\x1b[0m");
            println!(
                "  \x1b[38;5;237m────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
            );
            for (idx, (prov, m, it, ot, c, is_err)) in call_rows.into_iter().enumerate() {
                let err_str = if is_err {
                    "\x1b[38;5;196mERR\x1b[0m"
                } else {
                    "   "
                };
                let model_fmt = crate::ui::table::truncate(&m, 20);
                let prov_fmt = crate::ui::table::truncate(&prov, 14);
                let bar = crate::ui::table::make_bar((it + ot) as f64, max_call_tokens, 12);
                let total_t = crate::ui::table::compact_num((it + ot) as u64);

                println!(
                    "  \x1b[38;5;242m{:>2}\x1b[0m \x1b[38;5;43m{:<14}\x1b[0m \x1b[38;5;114m{:<20}\x1b[0m  {} {:>5}  \x1b[38;5;242min:\x1b[0m {:<5} \x1b[38;5;242mout:\x1b[0m {:<5} \x1b[38;5;220m${:>6.4}\x1b[0m  {}",
                    idx + 1, prov_fmt, model_fmt, bar, total_t, crate::ui::table::compact_num(it as u64), crate::ui::table::compact_num(ot as u64), c, err_str
                );
            }
            println!();
        }
    }

    Ok(())
}

fn print_custom_help() {
    let version = env!("CARGO_PKG_VERSION");
    let help_text = format!("
  \x1b[38;5;43m━━\x1b[0m \x1b[1mpii\x1b[0m \x1b[38;5;242mv{}\x1b[0m · session analytics & model explorer

  \x1b[38;5;246m┌─ USAGE ───────────────────────────────────────────────┐\x1b[0m
  \x1b[38;5;246m│\x1b[0m  pii [OPTIONS] [COMMAND]                              \x1b[38;5;246m│\x1b[0m
  \x1b[38;5;246m└───────────────────────────────────────────────────────┘\x1b[0m

  \x1b[38;5;246mCORE COMMANDS:\x1b[0m
    \x1b[38;5;114m<no args>\x1b[0m              \x1b[38;5;242mFuzzy-pick session, continue in pi\x1b[0m
    \x1b[38;5;114m-c, --continue-session\x1b[0m \x1b[38;5;242mInteractive picker to continue a session\x1b[0m
    \x1b[38;5;114m-i, --inspect\x1b[0m          \x1b[38;5;242mInteractive picker to inspect a session's details\x1b[0m

  \x1b[38;5;246mVIEWS:\x1b[0m
    \x1b[38;5;114m-t, --today\x1b[0m            \x1b[38;5;242mShow today's sessions\x1b[0m
    \x1b[38;5;114m-w, --week\x1b[0m             \x1b[38;5;242mShow past 7 days of sessions\x1b[0m
    \x1b[38;5;114m-m, --month\x1b[0m            \x1b[38;5;242mShow past 30 days of sessions\x1b[0m
    \x1b[38;5;114m-H, --heatmap\x1b[0m          \x1b[38;5;242mShow activity heatmap (150 days)\x1b[0m
    \x1b[38;5;114m-s, --summary\x1b[0m          \x1b[38;5;242mShow summary dashboard\x1b[0m

  \x1b[38;5;246mFILTERS:\x1b[0m
    \x1b[38;5;114m-q, --query <TEXT>\x1b[0m     \x1b[38;5;242mFilter sessions by text/model (FTS search)\x1b[0m
    \x1b[38;5;114m-d, --days <N>\x1b[0m         \x1b[38;5;242mScope picker to last N days\x1b[0m
    \x1b[38;5;114m--sort <COL>\x1b[0m           \x1b[38;5;242mSort by: cost, tokens, calls, time (default: time)\x1b[0m

  \x1b[38;5;246mSUBCOMMANDS:\x1b[0m
    \x1b[38;5;114mmodel [query]\x1b[0m          \x1b[38;5;242mModel detail card or interactive model picker\x1b[0m
    \x1b[38;5;114mcompare [m1 m2...]\x1b[0m     \x1b[38;5;242mCompare models side-by-side (interactive if no args) [--spider]\x1b[0m
    \x1b[38;5;114mrankings [category]\x1b[0m    \x1b[38;5;242mShow TrueSkill rankings · category: coding | math | general\x1b[0m
    \x1b[38;5;114msettings\x1b[0m               \x1b[38;5;242mInteractive settings picker\x1b[0m

  \x1b[38;5;246mOPTIONS:\x1b[0m
    \x1b[38;5;114m-h, --help\x1b[0m             \x1b[38;5;242mPrint this customized help message\x1b[0m
    \x1b[38;5;114m-V, --version\x1b[0m          \x1b[38;5;242mPrint version information\x1b[0m
", version);
    println!("{}", help_text);
}

fn main() -> rusqlite::Result<()> {
    // Load env vars, but ignore errors if .env is missing
    dotenvy::dotenv().ok();

    // Check for standard help flags before parser takes over to inject custom styled help
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_custom_help();
        return Ok(());
    }

    let cli = Cli::parse();

    let db_path = db::get_db_path();
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&db_path)?;
    db::init_db(&conn)?;

    // Load API keys from DB settings as fallback for env vars
    let fallback_keys = [
        ("LLM_STATS_API_KEY", "api.llm_stats_key"),
        ("ARTIFICIALANALYSIS_API_KEY", "api.artificial_analysis_key"),
    ];
    for (env_name, setting_key) in &fallback_keys {
        if std::env::var(env_name).is_ok() {
            continue;
        }
        let val = db::get_setting(&conn, setting_key, "")?;
        if !val.is_empty() {
            // SAFETY: single-threaded at this point, setting env vars is fine
            unsafe { std::env::set_var(env_name, &val); }
        }
    }

    // Force-resync: drop cached session rows, then re-parse every file.
    // The DELETE triggers on sessions/calls also wipe sessions_fts, so the
    // incremental sync that follows rebuilds everything from scratch.
    if cli.refresh_sessions {
        conn.execute("DELETE FROM calls", [])?;
        conn.execute("DELETE FROM sessions", [])?;
    }

    // Parse newly added sessions
    session::parser::sync_sessions(&conn)?;

    // -R forces a refresh of model data; otherwise it's a no-op when already
    // fetched today.
    models::api::refresh_if_needed(&conn, cli.refresh_api)?;

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
    if let Some(cmd) = &cli.command {
        match cmd {
            cli::Commands::Model { query, refresh } => {
                models::api::refresh_if_needed(&conn, *refresh)?;

                let target_id = if let Some(q) = query {
                    let mut stmt = conn.prepare("SELECT id FROM models WHERE id LIKE ? OR name LIKE ? ORDER BY LENGTH(id) ASC LIMIT 1")?;
                    let like_q = format!("%{}%", q);
                    let hit = stmt.query_row([&like_q, &like_q], |r| r.get::<_, String>(0)).ok();
                    if hit.is_none() {
                        println!(
                            "\n  \x1b[38;5;196m✗\x1b[0m No model matched \x1b[38;5;246m{}\x1b[0m. Opening picker...\n",
                            q
                        );
                        models::detail::run_model_picker(&conn)?
                    } else {
                        hit
                    }
                } else {
                    models::detail::run_model_picker(&conn)?
                };

                if let Some(id) = target_id {
                    models::detail::print_model_detail(&conn, &id)?;
                }
                return Ok(());
            }
            cli::Commands::Compare { models, spider } => {
                // Refresh quietly so we have data to pick from.
                models::api::refresh_if_needed(&conn, false)?;

                // If no models were given on the command line, prompt the user
                // to pick them via the same fuzzy picker used by `pii model`.
                let queries = if models.is_empty() {
                    println!("\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mModel Comparison\x1b[0m");
                    let count_str = prompt_input("How many models to compare?", Some("2"))
                        .unwrap_or_else(|_| "2".to_string());
                    let count: usize = count_str.trim().parse().unwrap_or(2);
                    if count < 2 {
                        println!("  \x1b[38;5;242mNeed at least 2 models to compare.\x1b[0m");
                        return Ok(());
                    }

                    let mut picked = Vec::new();
                    for i in 1..=count {
                        let prompt = format!("Select model {}/{}", i, count);
                        match models::detail::run_model_picker_with_prompt(&conn, &prompt)? {
                            Some(id) => picked.push(id),
                            None => {
                                println!("  \x1b[38;5;242mCancelled at model {}.\x1b[0m", i);
                                return Ok(());
                            }
                        }
                    }
                    picked
                } else {
                    models.clone()
                };

                let compare_data = crate::models::compare::print_compare_table(&conn, &queries)?;

                if *spider && !compare_data.is_empty() {
                    let core_categories = vec!["livecodebench".to_string(), "math_500".to_string(), "gpqa".to_string(), "mmlu_pro".to_string(), "aime".to_string()];
                    
                    let mut colors = vec![Color::Cyan, Color::Yellow, Color::Magenta, Color::Green, Color::Red];
                    let mut spider_models = Vec::new();

                    for d in compare_data {
                        let mut values = Vec::new();
                        for cat in &core_categories {
                            let val = d.evals.iter().find(|e| e.benchmark == *cat).map(|e| {
                                let m_score = e.max_score.unwrap_or(if e.score > 1.0 { 100.0 } else { 1.0 });
                                if m_score > 1.0 { e.score / m_score } else { e.score }
                            }).unwrap_or(0.0);
                            values.push(val);
                        }
                        spider_models.push(crate::ui::spider::SpiderData {
                            name: d.model.name.clone(),
                            values,
                            color: colors.pop().unwrap_or(Color::White),
                        });
                    }

                    crate::ui::spider::run_spider_chart(spider_models, core_categories).unwrap();
                }
                return Ok(());
            }
            cli::Commands::Rankings { category } => {
                // Make sure we have models scored — refresh quietly if needed.
                models::api::refresh_if_needed(&conn, false)?;
                models::rankings::print_rankings(&conn, category.as_deref())?;
                return Ok(());
            }
            cli::Commands::Settings => {
                let changed = ui::settings::run_settings_picker(&conn)?;
                if changed.is_empty() {
                    println!("  \x1b[38;5;242mNo changes.\x1b[0m");
                } else {
                    println!("  \x1b[38;5;43m✓\x1b[0m Saved \x1b[1m{}\x1b[0m setting(s):", changed.len());
                    for k in &changed {
                        println!("    \x1b[38;5;246m•\x1b[0m {}", k);
                    }
                }
                return Ok(());
            }
            cli::Commands::Tree => {
                ui::tree_picker::dump_tree(&conn)?;
                return Ok(());
            }
        }
    }

    // Default behavior or explicit continue/inspect
    let is_inspect = cli.inspect || cli.calls;
    let view = db::get_setting(&conn, "picker.view", "tree")?;
    let prompt_label = if is_inspect { "inspect" } else { "continue" };
    let selection = if view == "tree" {
        ui::tree_picker::run_tree_picker(&conn, cli.days, prompt_label)?
    } else {
        ui::picker::run_picker(&conn, cli.days, prompt_label)?
    };

    if let Some(selection) = selection {
        if is_inspect {
            // In Phase 3, -i explicitly implies we show calls if they exist.
            inspect_session(&conn, &selection.id, true)?;
        } else {
            println!(
                "  \x1b[38;5;43m▰\x1b[0m Resuming session \x1b[38;5;246m{}\x1b[0m",
                selection.file_path
            );
            // Try a few Windows-specific candidates first; fall back to bare "pi".
            let pi = ["pi.cmd", "pi.exe", "pi.bat", "pi"]
                .iter()
                .find(|c| which_exists(c))
                .copied()
                .unwrap_or("pi");
            let status = Command::new(pi)
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
