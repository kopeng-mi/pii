use crate::ui::table::compact_num;
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

    let mut total_sess = 0;
    let mut total_call = 0;
    let mut total_toks = 0;
    let mut total_c = 0.0;
    let mut total_errs = 0;

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

    // Top models
    let top_models_sql = "
        SELECT model, COUNT(*) as call_count, SUM(tokens) as token_count, SUM(cost) as model_cost
        FROM calls
        GROUP BY model
        ORDER BY call_count DESC
        LIMIT 3
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

    println!("\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mSummary Dashboard\x1b[0m\n");

    println!("  \x1b[38;5;246mAll Time Totals\x1b[0m");
    println!("  \x1b[38;5;237m────────────────────────────────────────────────────\x1b[0m");
    println!(
        "  \x1b[1mSessions:\x1b[0m {:<10} \x1b[1mCalls:\x1b[0m {:<10}",
        total_sess, total_call
    );
    println!(
        "  \x1b[1mTokens:  \x1b[0m {:<10} \x1b[1mCost: \x1b[0m ${:<9.2}",
        compact_num(total_toks as u64),
        total_c
    );
    println!("  \x1b[1mErrors:  \x1b[0m {:<10}", total_errs);

    println!("\n  \x1b[38;5;246mTop Models\x1b[0m");
    println!("  \x1b[38;5;237m────────────────────────────────────────────────────\x1b[0m");
    for (m, calls, tokens, cost) in top_models {
        let model_fmt = crate::ui::table::truncate(&m, 20);
        println!(
            "  \x1b[38;5;114m{:<20}\x1b[0m │ {:>5} calls │ {:>6} tk │ \x1b[38;5;220m${:>5.2}\x1b[0m",
            model_fmt,
            calls,
            compact_num(tokens as u64),
            cost
        );
    }
    println!();

    Ok(())
}
