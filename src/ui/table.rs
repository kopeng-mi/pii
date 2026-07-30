use rusqlite::Connection;

pub fn print_sessions(
    conn: &Connection,
    limit: Option<u32>,
    title: &str,
    sort: &str,
    exact_date: Option<&str>,
    min_date: Option<&str>,
    search_query: Option<&str>,
) -> rusqlite::Result<()> {
    let order = match sort {
        "cost" => "total_cost DESC",
        "tokens" => "total_tokens DESC",
        "calls" => "total_calls DESC",
        _ => "date DESC, time DESC", // default time
    };

    let limit_clause = if let Some(l) = limit {
        format!("LIMIT {}", l)
    } else {
        "".to_string()
    };

    let mut where_clauses = Vec::new();
    if let Some(d) = exact_date {
        where_clauses.push(format!("date = '{}'", d));
    }
    if let Some(d) = min_date {
        where_clauses.push(format!("date >= '{}'", d));
    }

    let mut from_clause = "sessions".to_string();
    if let Some(query) = search_query {
        from_clause =
            "sessions JOIN sessions_fts ON sessions.rowid = sessions_fts.rowid".to_string();
        // Support searching across prompts, project names, and model names
        where_clauses.push(format!("sessions_fts MATCH '\"{}*\"'", query.replace("'", "''")));
    }

    let where_clause = if where_clauses.is_empty() {
        "".to_string()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let sql = format!("
        SELECT sessions.id, sessions.project, sessions.date, sessions.time, sessions.total_calls, sessions.total_tokens, sessions.total_cost, sessions.errors,
            COALESCE(sessions.last_model, '') as last_model
        FROM {}
        {}
        ORDER BY {} {}", from_clause, where_clause, order, limit_clause);

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?, // id
            row.get::<_, String>(1)?, // project
            row.get::<_, String>(2)?, // date
            row.get::<_, String>(3)?, // time
            row.get::<_, u32>(4)?,    // calls
            row.get::<_, u32>(5)?,    // tokens
            row.get::<_, f64>(6)?,    // cost
            row.get::<_, u32>(7)?,    // errors
            row.get::<_, Option<String>>(8)?
                .unwrap_or_else(|| "".to_string()), // model
        ))
    })?;

    let mut sessions = Vec::new();
    let mut total_calls = 0;
    let mut total_tokens = 0;
    let mut total_cost = 0.0;

    for row in rows {
        let (id, project, date, time, calls, tokens, cost, errors, model) = row?;
        total_calls += calls;
        total_tokens += tokens;
        total_cost += cost;
        sessions.push((id, project, date, time, calls, tokens, cost, errors, model));
    }

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    let max_tokens = sessions.iter().map(|s| s.5).max().unwrap_or(1) as f64 / 1000.0;

    // Print Header
    println!(
        "\n  \x1b[38;5;43m━━\x1b[0m \x1b[1m{}\x1b[0m · {} sessions · {} calls · {} tokens · ${:.2}\n",
        title,
        sessions.len(),
        total_calls,
        compact_num(total_tokens as u64),
        total_cost
    );

    let header =
        "  \x1b[38;5;246mwhen          project   model                 calls  usage           cost   err\x1b[0m";
    println!("{}", header);
    println!(
        "  \x1b[38;5;237m─────────────────────────────────────────────────────────────────────────────────\x1b[0m"
    );

    for (_, project, date, time, calls, tokens, cost, errors, model) in sessions {
        let date_short = &date[5..]; // MM-DD
        let when = format!("{} {}", date_short, time);

        let proj_fmt = format!("{:width$}", truncate(&project, 9), width = 9);
        let model_fmt = format!("{:width$}", truncate(&model, 20), width = 20);
        let calls_fmt = if calls == 0 {
            " -- ".to_string()
        } else {
            format!("{:>4}", calls)
        };
        let cost_fmt = if cost == 0.0 {
            "   -- ".to_string()
        } else {
            // 7-char column: $X.XX / $XX.XX / $XXX.XX all fit, left-padded with spaces.
            format!("${:>6.2}", cost)
        };
        let err_fmt = if errors == 0 {
            "\x1b[38;5;242m·\x1b[0m".to_string()
        } else {
            format!("\x1b[38;5;196m{}\x1b[0m", errors)
        };

        let tokens_k = (tokens as f64) / 1000.0;
        let bar = make_bar(tokens_k, max_tokens, 8);

        let usage_label = if tokens >= 1_000_000 {
            format!("{:>4.1}M", tokens as f64 / 1_000_000.0)
        } else if tokens >= 1_000 {
            format!("{:>4.0}K", tokens_k)
        } else {
            format!("{:>5}", tokens) // raw tokens
        };
        let usage_fmt = format!("{} {}", bar, usage_label);

        println!(
            "  \x1b[38;5;250m{when}\x1b[0m   \x1b[1m{proj_fmt}\x1b[0m   \x1b[38;5;114m{model_fmt}\x1b[0m  {calls_fmt}  {usage_fmt}  \x1b[38;5;220m{cost_fmt}\x1b[0m   {err_fmt}"
        );
    }
    
    println!(
        "  \x1b[38;5;237m───────────────────────────────────── end ─────────────────────────────────────\x1b[0m"
    );

    // ── Provider breakdown (time-scoped) ──
    if let Some(date_filter) = exact_date.or(min_date) {
        let op = if exact_date.is_some() { "=" } else { ">=" };
        let psql = format!(
            "SELECT c.provider, COUNT(*) AS calls, SUM(c.tokens) AS tokens, SUM(c.cost) AS cost
             FROM calls c JOIN sessions s ON s.id = c.session_id
             WHERE s.date {} ?1 AND c.provider != ''
             GROUP BY c.provider ORDER BY calls DESC", op);
        let mut pstmt = conn.prepare(&psql)?;
        let mut provs = Vec::new();
        let mut rows = pstmt.query([date_filter])?;
        while let Some(row) = rows.next()? {
            let p: String = row.get(0)?;
            let c: u32 = row.get(1)?;
            let t: u32 = row.get(2)?;
            let cost: f64 = row.get(3)?;
            provs.push((p, c, t, cost));
        }
        if !provs.is_empty() {
            let max_c = provs.first().map(|x| x.1).unwrap_or(1);
            println!();
            println!("  \x1b[38;5;51mProviders\x1b[0m");
            for (p, calls, tokens, cost) in &provs {
                let bar = make_bar(*calls as f64, max_c as f64, 6);
                let cost_fmt = if *cost == 0.0 { "     --".to_string() } else { format!("${:>6.2}", cost) };
                println!(
                    "    \x1b[38;5;43m{:<16}\x1b[0m  {} {:>4} calls  {:>7}  \x1b[38;5;220m{}\x1b[0m",
                    truncate(p, 16), bar, calls, compact_num(*tokens as u64), cost_fmt
                );
            }
        }
    }

    println!();
    Ok(())
}


pub fn compact_num(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let mut truncated = String::new();
        let mut width = 0;
        for c in s.chars() {
            let w = 1;
            if width + w > max - 1 {
                break;
            }
            truncated.push(c);
            width += w;
        }
        truncated.push('…');
        truncated
    } else {
        s.to_string()
    }
}

pub fn make_bar(value: f64, max: f64, width: usize) -> String {
    let blocks = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];
    if max <= 0.0 || value <= 0.0 {
        return "\x1b[38;5;237m".to_string() + &" ".repeat(width) + "\x1b[0m";
    }

    let ratio = (value / max).clamp(0.0, 1.0);
    let filled_len = ratio * (width as f64);
    let full_blocks = filled_len.floor() as usize;
    let remainder = (filled_len.fract() * 8.0).round() as usize;

    let mut bar = "\x1b[38;5;43m".to_string(); // Teal
    for _ in 0..full_blocks {
        bar.push_str("█");
    }
    if full_blocks < width && remainder > 0 {
        bar.push_str(blocks[remainder]);
    }

    let space = width.saturating_sub(full_blocks + if remainder > 0 { 1 } else { 0 });
    bar.push_str("\x1b[0m\x1b[38;5;237m");
    for _ in 0..space {
        bar.push_str("█");
    }
    bar.push_str("\x1b[0m");
    bar
}
