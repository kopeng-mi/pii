use crate::models::types::{Evaluation, UnifiedModel};
use crate::ui::table::{compact_num, truncate, make_bar};
use std::collections::HashMap;

pub struct CompareData {
    pub model: UnifiedModel,
    pub evals: Vec<Evaluation>,
}

pub fn print_compare_table(conn: &rusqlite::Connection, models: &[String]) -> rusqlite::Result<Vec<CompareData>> {
    let mut data = Vec::new();

    for id in models {
        let mut m_stmt = conn.prepare("SELECT id, name, creator, release_date, context_window, param_count, input_price, output_price, speed_tok_s, ttft_s, open_weight, source, raw_json FROM models WHERE id LIKE ?1 OR name LIKE ?1 ORDER BY LENGTH(id) ASC LIMIT 1")?;
        let like_q = format!("%{}%", id.replace("-", "%"));
        
        let mut model = None;
        let mut m_rows = m_stmt.query([&like_q])?;
        if let Some(row) = m_rows.next()? {
            model = Some(UnifiedModel {
                id: row.get(0)?,
                name: row.get(1)?,
                creator: row.get(2)?,
                release_date: row.get(3)?,
                context_window: row.get(4)?,
                param_count: row.get(5)?,
                input_price: row.get(6)?,
                output_price: row.get(7)?,
                speed_tok_s: row.get(8)?,
                ttft_s: row.get(9)?,
                open_weight: row.get(10)?,
                source: row.get(11)?,
                raw_json: row.get(12)?,
            });
        }

        if let Some(m) = model {
            let mut evals = Vec::new();
            let mut b_stmt = conn.prepare("SELECT benchmark, score, max_score, category FROM scores WHERE model_id = ? ORDER BY category, benchmark")?;
            let mut b_rows = b_stmt.query([&m.id])?;
            while let Some(row) = b_rows.next()? {
                evals.push(Evaluation {
                    model_id: m.id.clone(),
                    benchmark: row.get(0)?,
                    score: row.get(1)?,
                    max_score: row.get(2)?,
                    category: row.get(3)?,
                });
            }
            data.push(CompareData { model: m, evals });
        } else {
            println!("  \x1b[38;5;196m✗\x1b[0m Model not found: {}", id);
        }
    }

    if data.is_empty() {
        return Ok(data);
    }

    // Print table side-by-side
    println!("\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mCompare\x1b[0m");

    // Name row
    print!("  {:<18}", "");
    for d in &data {
        print!(" \x1b[1m{:<28}\x1b[0m", truncate(&d.model.name, 28));
    }
    println!();

    // Context
    print!("  \x1b[38;5;246m{:<18}\x1b[0m", "Context");
    for d in &data {
        let ctx = d.model.context_window.map(|c| compact_num(c as u64)).unwrap_or_else(|| "--".into());
        let pad = 28_usize.saturating_sub(ctx.len());
        print!(" {}{}", ctx, " ".repeat(pad));
    }
    println!();

    // Speed
    print!("  \x1b[38;5;246m{:<18}\x1b[0m", "Speed");
    let best_speed = data.iter().filter_map(|d| d.model.speed_tok_s).fold(0.0, f64::max);
    for d in &data {
        if let Some(s) = d.model.speed_tok_s {
            let marker = if s > 0.0 && s >= best_speed { "\x1b[38;5;220m◀\x1b[0m" } else { " " };
            let s_str = format!("{:.1} t/s", s);
            
            if marker == " " {
                let pad = 28_usize.saturating_sub(s_str.len());
                print!(" {}{}", s_str, " ".repeat(pad));
            } else {
                let pad = 28_usize.saturating_sub(s_str.len() + 2); // 2 for " ◀"
                print!(" {} {}{}", s_str, marker, " ".repeat(pad));
            }
        } else {
            let pad = 28_usize.saturating_sub(2);
            print!(" --{}", " ".repeat(pad));
        }
    }
    println!();

    // Price
    print!("  \x1b[38;5;246m{:<18}\x1b[0m", "Price (In/Out)");
    for d in &data {
        if d.model.input_price == 0.0 && d.model.output_price == 0.0 {
            let pad = 28_usize.saturating_sub(7); // "-- / --"
            print!(" -- / --{}", " ".repeat(pad));
        } else {
            let p_str = format!("${:.2} / ${:.2}", d.model.input_price, d.model.output_price);
            let display = format!("\x1b[38;5;220m${:.2}\x1b[0m / \x1b[38;5;220m${:.2}\x1b[0m", d.model.input_price, d.model.output_price);
            let pad = 28_usize.saturating_sub(p_str.len());
            print!(" {}{}", display, " ".repeat(pad));
        }
    }
    println!();
    println!();

    // Benchmarks
    // Collect union of benchmarks across all selected models
    let mut benchmarks = Vec::new();
    let mut bench_maxes = HashMap::new();
    for d in &data {
        for e in &d.evals {
            if !benchmarks.contains(&e.benchmark) {
                benchmarks.push(e.benchmark.clone());
            }
            let max = bench_maxes.entry(e.benchmark.clone()).or_insert(0.0_f64);
            // AA data is mixed, assume if score is > 1.0, max is 100. If <= 1.0, max is 1.0
            let s_max = e.max_score.unwrap_or(if e.score > 1.0 { 100.0 } else { 1.0 });
            if s_max > *max { *max = s_max; }
        }
    }
    benchmarks.sort();

    if !benchmarks.is_empty() {
        println!("  \x1b[38;5;246mBenchmarks\x1b[0m");
        println!("  \x1b[38;5;237m────────────────────────────────────────────────────────────────────────────\x1b[0m");
        
        for b in benchmarks {
            let label = truncate(&b, 18);
            print!("  \x1b[38;5;114m{:<18}\x1b[0m", label);
            
            // Find best score for this bench
            let mut best_score = 0.0;
            for d in &data {
                if let Some(e) = d.evals.iter().find(|e| e.benchmark == b) {
                    let m_score = e.max_score.unwrap_or(if e.score > 1.0 { 100.0 } else { 1.0 });
                    let norm = if m_score <= 1.0 { e.score * 100.0 } else { e.score };
                    if norm > best_score { best_score = norm; }
                }
            }

            for d in &data {
                if let Some(e) = d.evals.iter().find(|e| e.benchmark == b) {
                    let m_score = e.max_score.unwrap_or(if e.score > 1.0 { 100.0 } else { 1.0 });
                    let (norm_score, max_val) = if m_score > 1.0 { (e.score, m_score) } else { (e.score * 100.0, 100.0) };
                    
                    let marker = if norm_score > 0.0 && (norm_score - best_score).abs() < 0.001 { "\x1b[38;5;220m◀\x1b[0m" } else { " " };
                    let bar = make_bar(norm_score, max_val, 10);
                    // bar visually: 10 chars
                    // " " (1 char)
                    // norm_score format: 5 chars (e.g., " 10.3")
                    // " " (1 char)
                    // marker: 1 char
                    // bar (10) + space (1) + score (5) + space (1) + marker (1) + 8 spaces = 26 physical layout characters
                    let score_str = format!("{:>5.1}", norm_score);
                    // bar is visually 10 chars, score format is 5 chars.
                    // total string width when printed without marker: 1(space) + 10(bar) + 1(space) + 5(score) = 17
                    // Cell target width: 29 physical characters (" " + 28 chars width)
                    if marker == " " {
                        print!(" {} {}            ", bar, score_str); // 12 spaces total pad (17 + 12 = 29)
                    } else {
                        print!(" {} {} {}         ", bar, score_str, marker); // 9 spaces total pad (17 + 2 + 10 = 29)
                    }
                } else {
                    print!(" {:<28}", "--");
                }
            }
            println!();
        }
        println!();
    }

    Ok(data)
}