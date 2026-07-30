use crate::ui::table::{compact_num, make_bar};
use rusqlite::Connection;

pub fn print_summary(conn: &Connection) -> rusqlite::Result<()> {
    let sql = "
        SELECT
            COUNT(id) as total_sessions,
            SUM(total_calls) as total_calls,
            SUM(total_tokens) as total_tokens,
            SUM(total_cost) as total_cost,
            SUM(errors) as total_errors
        FROM sessions
    ";

    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;

    let mut total_sess: u32 = 0;
    let mut total_call: u32 = 0;
    let mut total_toks: u32 = 0;
    let mut total_c: f64 = 0.0;
    let mut total_errs: u32 = 0;

    if let Some(row) = rows.next()? {
        total_sess = row.get::<_, u32>(0).unwrap_or(0);
        total_call = row.get::<_, u32>(1).unwrap_or(0);
        total_toks = row.get::<_, u32>(2).unwrap_or(0);
        total_c = row.get::<_, f64>(3).unwrap_or(0.0);
        total_errs = row.get::<_, u32>(4).unwrap_or(0);
    }

    if total_sess == 0 {
        println!("No sessions found.");
        return Ok(());
    }

    // Today's stats
    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();
    let today_sql = "SELECT COUNT(id), COALESCE(SUM(total_calls),0), COALESCE(SUM(total_tokens),0), COALESCE(SUM(total_cost),0) FROM sessions WHERE date = ?1";
    let mut today_stmt = conn.prepare(today_sql)?;
    let (today_sess, today_calls, today_toks, today_cost): (u32, u32, u32, f64) =
        today_stmt.query_row([&today_str], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;

    // Active days count for averages
    let active_days: u32 = conn.query_row(
        "SELECT COUNT(DISTINCT date) FROM sessions", [], |r| r.get(0),
    )?;
    let avg_daily_sess = if active_days > 0 { total_sess as f64 / active_days as f64 } else { 0.0 };
    let avg_daily_cost = if active_days > 0 { total_c / active_days as f64 } else { 0.0 };

    // Most active project
    let top_project: Option<(String, u32)> = conn.prepare(
        "SELECT project, COUNT(*) as cnt FROM sessions GROUP BY project ORDER BY cnt DESC LIMIT 1"
    )?.query_row([], |r| Ok((r.get(0)?, r.get(1)?))).ok();

    // Top models
    let top_models_sql = "
        SELECT model, COUNT(*) as call_count, SUM(tokens) as token_count, SUM(cost) as model_cost
        FROM calls
        GROUP BY model
        ORDER BY call_count DESC
        LIMIT 5
    ";

    let mut stmt_m = conn.prepare(top_models_sql)?;
    let mut models_iter = stmt_m.query([])?;

    let mut top_models = Vec::new();
    while let Some(row) = models_iter.next()? {
        let m: String = row.get(0)?;
        let c: u32 = row.get(1)?;
        let t: u32 = row.get(2)?;
        let mc: f64 = row.get(3)?;
        top_models.push((m, c, t, mc));
    }

    let max_model_calls = top_models.first().map(|m| m.1).unwrap_or(1);

    // ── Header ──
    let h = "Summary Dashboard";
    println!("\n  \x1b[1m{}\x1b[0m", h);
    println!("  \x1b[38;5;51m{}\x1b[0m\n", "━".repeat(h.chars().count()));

    // ── All Time ──
    let h = "All Time";
    println!("  \x1b[38;5;51m{}\x1b[0m", h);
    println!("  \x1b[38;5;237m{}\x1b[0m", "─".repeat(h.chars().count()));
    println!(
        "    {:8} \x1b[1m{:>6}\x1b[0m    {:6} \x1b[1m{:>8}\x1b[0m",
        "Sessions", total_sess, "Calls", total_call
    );
    println!(
        "    {:8} \x1b[1m{:>6}\x1b[0m    {:6} \x1b[1m\x1b[38;5;220m${:.2}\x1b[0m",
        "Tokens", compact_num(total_toks as u64), "Cost", total_c
    );
    if total_errs > 0 {
        println!(
            "    {:8} \x1b[38;5;196m{}\x1b[0m",
            "Errors", total_errs
        );
    }
    if let Some((proj, cnt)) = &top_project {
        println!(
            "    {:8} \x1b[1m{}\x1b[0m \x1b[38;5;242m({} sessions)\x1b[0m",
            "Top Proj", crate::ui::table::truncate(proj, 18), cnt
        );
    }
    println!();

    // ── Today vs Average ──
    let h = "Today vs Daily Avg";
    println!("  \x1b[38;5;51m{}\x1b[0m", h);
    println!("  \x1b[38;5;237m{}\x1b[0m", "─".repeat(h.chars().count()));
    println!(
        "    {:8} \x1b[1m{}\x1b[0m \x1b[38;5;242mtoday\x1b[0m   \x1b[38;5;242m/ {:.1} avg/day\x1b[0m",
        "Sessions", today_sess, avg_daily_sess
    );
    println!(
        "    {:8} \x1b[1m{}\x1b[0m \x1b[38;5;242mtoday\x1b[0m   \x1b[38;5;242m/ {} tokens\x1b[0m",
        "Calls", today_calls, compact_num(today_toks as u64)
    );
    println!(
        "    {:8} \x1b[38;5;220m${:.2}\x1b[0m \x1b[38;5;242mtoday\x1b[0m \x1b[38;5;242m/ ${:.2} avg/day\x1b[0m",
        "Cost", today_cost, avg_daily_cost
    );
    println!();

    // ── Top Models ──
    let h = "Top Models";
    println!("  \x1b[38;5;51m{}\x1b[0m", h);
    println!("  \x1b[38;5;237m{}\x1b[0m", "─".repeat(h.chars().count()));
    for (m, calls, tokens, cost) in top_models.iter() {
        let model_fmt = crate::ui::table::truncate(m, 20);
        let bar = make_bar(*calls as f64, max_model_calls as f64, 8);
        let cost_fmt = format!("${:>6.2}", cost);
        let tokens_str = compact_num(*tokens as u64);
        println!(
            "    \x1b[38;5;114m{:<20}\x1b[0m  {} {:>5} calls  {:>6} tk  \x1b[38;5;220m{}\x1b[0m",
            model_fmt, bar, calls, tokens_str, cost_fmt
        );
    }
    println!();

    // ── Provider Breakdown ──
    let prov_sql = "
        SELECT c.provider, COUNT(*) AS call_count, SUM(c.tokens) AS token_count, SUM(c.cost) AS total_cost
        FROM calls c
        WHERE c.provider != ''
        GROUP BY c.provider
        ORDER BY call_count DESC
    ";
    let mut stmt_p = conn.prepare(prov_sql)?;
    let mut prov_iter = stmt_p.query([])?;

    let mut providers = Vec::new();
    while let Some(row) = prov_iter.next()? {
        let p: String = row.get(0)?;
        let c: u32 = row.get(1)?;
        let t: u32 = row.get(2)?;
        let cost: f64 = row.get(3)?;
        providers.push((p, c, t, cost));
    }

    if !providers.is_empty() {
        let max_prov_calls = providers.first().map(|p| p.1).unwrap_or(1);
        let h = "Provider Breakdown";
        println!("  \x1b[38;5;51m{}\x1b[0m", h);
        println!("  \x1b[38;5;237m{}\x1b[0m", "─".repeat(h.chars().count()));
        for (p, calls, tokens, cost) in providers.iter() {
            let bar = make_bar(*calls as f64, max_prov_calls as f64, 8);
            let cost_fmt = format!("${:>6.2}", cost);
            let tokens_str = compact_num(*tokens as u64);
            println!(
                "    \x1b[38;5;43m{:<20}\x1b[0m  {} {:>5} calls  {:>6} tk  \x1b[38;5;220m{}\x1b[0m",
                crate::ui::table::truncate(p, 20), bar, calls, tokens_str, cost_fmt
            );
        }
        println!();
    }

    Ok(())
}
