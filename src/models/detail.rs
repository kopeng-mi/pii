use crate::models::types::UnifiedModel;
use crate::ui::picker::{pick, Selection};
use crate::ui::table::compact_num;

pub fn format_model_item(m: &UnifiedModel) -> (Selection, String) {
    let speed = m.speed_tok_s.map(|s| format!("{:>3.0}", s)).unwrap_or_else(|| " --".to_string());
    let context = m.context_window.map(|c| compact_num(c as u64)).unwrap_or_else(|| " -- ".to_string());
    let price_str = if m.input_price > 0.0 || m.output_price > 0.0 {
        format!("${:.2}/${:.2}", m.input_price, m.output_price)
    } else {
        "--/--".to_string()
    };
    
    let display = format!(
        "\x1b[38;5;114m{:<25}\x1b[0m  {:<15}  \x1b[38;5;220m{:<12}\x1b[0m  {:>4}k  {} t/s",
        crate::ui::table::truncate(&m.name, 25),
        crate::ui::table::truncate(&m.creator, 15),
        price_str,
        context,
        speed
    );

    (
        Selection { id: m.id.clone(), file_path: String::new() },
        display
    )
}

pub fn run_model_picker(conn: &rusqlite::Connection) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, creator, release_date, context_window, param_count, input_price, output_price, speed_tok_s, ttft_s, open_weight, source, raw_json 
         FROM models 
         ORDER BY name ASC"
    )?;
    
    let items = stmt.query_map([], |row| {
        let m = UnifiedModel {
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
        };
        Ok(format_model_item(&m))
    })?.collect::<rusqlite::Result<Vec<_>>>()?;

    if items.is_empty() {
        println!("No models found in database. Try running with --refresh.");
        return Ok(None);
    }

    match pick(items, "model") {
        Ok(Some(sel)) => Ok(Some(sel.id)),
        _ => Ok(None)
    }
}

pub fn print_model_detail(conn: &rusqlite::Connection, model_id: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, name, creator, release_date, context_window, param_count, input_price, output_price, speed_tok_s, ttft_s, open_weight, source
         FROM models WHERE id = ?1"
    )?;

    let mut rows = stmt.query([model_id])?;
    if let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        let creator: String = row.get(2)?;
        let release: String = row.get(3)?;
        let ctx: Option<u32> = row.get(4)?;
        let params: Option<u64> = row.get(5)?;
        let in_price: f64 = row.get(6)?;
        let out_price: f64 = row.get(7)?;
        let speed: Option<f64> = row.get(8)?;
        let open_weight: bool = row.get(10)?;
        let source: String = row.get(11)?;

        println!("\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mModel Profile\x1b[0m");
        println!("  \x1b[38;5;246mName:\x1b[0m       \x1b[1m{}\x1b[0m", name);
        println!("  \x1b[38;5;246mCreator:\x1b[0m    {}", if creator.is_empty() { "Unknown" } else { &creator });
        println!("  \x1b[38;5;246mOpen Weight:\x1b[0m {}", if open_weight { "\x1b[38;5;114mYes\x1b[0m" } else { "\x1b[38;5;196mNo\x1b[0m" });
        println!("  \x1b[38;5;246mReleased:\x1b[0m   {}", if release.is_empty() { "Unknown" } else { &release });
        
        let ctx_str = ctx.map(|c| compact_num(c as u64)).unwrap_or_else(|| "Unknown".into());
        let param_str = params.map(|p| compact_num(p)).unwrap_or_else(|| "Unknown".into());
        println!("  \x1b[38;5;246mContext:\x1b[0m    {} tokens", ctx_str);
        println!("  \x1b[38;5;246mParams:\x1b[0m     {}", param_str);
        
        println!("  \x1b[38;5;246mSpeed:\x1b[0m      {}", speed.map(|s| format!("{:.1} tok/s", s)).unwrap_or_else(|| "Unknown".into()));
        println!("  \x1b[38;5;246mPricing:\x1b[0m    \x1b[38;5;220m${:.2}\x1b[0m in / \x1b[38;5;220m${:.2}\x1b[0m out (per 1M tokens)", in_price, out_price);
        println!("  \x1b[38;5;237mSource: {}\x1b[0m", source);
        println!();

        let mut b_stmt = conn.prepare("SELECT benchmark, score, max_score, category FROM scores WHERE model_id = ? ORDER BY category, benchmark")?;
        let bench_rows = b_stmt.query_map([model_id], |brow| {
            Ok((
                brow.get::<_, String>(0)?,
                brow.get::<_, f64>(1)?,
                brow.get::<_, Option<f64>>(2)?,
                brow.get::<_, String>(3)?
            ))
        })?.collect::<Result<Vec<_>, _>>()?;

        if !bench_rows.is_empty() {
            println!("  \x1b[38;5;246mBenchmarks\x1b[0m");
            println!("  \x1b[38;5;237m──────────────────────────────────────────────────────────────────\x1b[0m");
            
            for (bench, score, max_score, _) in bench_rows {
                let m_score = max_score.unwrap_or(1.0);
                // Convert AA 100-basis points or normalized points
                let (norm_score, max_val) = if m_score > 1.0 { (score, m_score) } else { (score * 100.0, 100.0) };
                
                let bar = crate::ui::table::make_bar(norm_score, max_val, 12);
                let bench_fmt = crate::ui::table::truncate(&bench, 20);
                println!("  \x1b[38;5;114m{:<20}\x1b[0m {} {:>5.1}", bench_fmt, bar, norm_score);
            }
            println!();
        }

    } else {
        println!("Model not found: {}", model_id);
    }
    
    Ok(())
}
