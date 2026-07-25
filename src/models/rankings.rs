use crate::ui::table::{make_bar, truncate};
use rusqlite::Connection;

#[derive(Debug)]
pub struct RankedModel {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    pub creator: String,
    pub source: String,
    pub avg_score: f64,
    #[allow(dead_code)]
    pub benchmark_count: u32,
}

pub fn print_rankings(conn: &Connection, category: Option<&str>) -> rusqlite::Result<()> {
    match category {
        None => print_overall_leaderboard(conn),
        Some(cat) => print_category_leaderboard(conn, cat),
    }
}

fn print_overall_leaderboard(conn: &Connection) -> rusqlite::Result<()> {
    println!(
        "\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mModel Rankings\x1b[0m · Overall Average Score\n"
    );

    let models = query_ranked_models(conn, None, 25)?;
    if models.is_empty() {
        println!(
            "  \x1b[38;5;242mNo scores in database yet. Run `pii model --refresh` first.\x1b[0m\n"
        );
        return Ok(());
    }

    print_rows(&models);
    println!();
    println!("  \x1b[38;5;246mFilter:\x1b[0m \x1b[38;5;114mpii rankings [coding|math|general]\x1b[0m");
    Ok(())
}

fn print_category_leaderboard(conn: &Connection, category: &str) -> rusqlite::Result<()> {
    let category_lower = category.to_lowercase();
    let title = titleize(category);

    println!(
        "\n  \x1b[38;5;43m━━\x1b[0m \x1b[1m{}\x1b[0m Rankings · by Average Score\n",
        title
    );

    let models = query_ranked_models(conn, Some(&category_lower), 25)?;
    if models.is_empty() {
        println!(
            "  \x1b[38;5;242mNo models scored under category \x1b[0m\x1b[38;5;114m{}\x1b[0m\x1b[38;5;242m. Try: coding | math | general.\x1b[0m\n",
            category_lower
        );
        return Ok(());
    }

    print_rows(&models);
    println!();

    println!(
        "  \x1b[38;5;246mTop Per Benchmark\x1b[0m · {} category\n",
        category_lower
    );
    let sql = "
        SELECT s.benchmark,
               (SELECT m.name FROM models m WHERE m.id = s.model_id) AS winner,
               MAX(CASE WHEN s.max_score IS NULL OR s.max_score <= 1.0
                        THEN s.score * 100.0 ELSE s.score END) AS best
        FROM scores s
        WHERE s.category = ?1
        GROUP BY s.benchmark
        ORDER BY best DESC
        LIMIT 8
    ";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([&category_lower], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    let bench_winners: Vec<(String, String, f64)> = rows
        .map(|r| {
            let (b, w_opt, score) = r?;
            Ok((b, w_opt.unwrap_or_else(|| "--".into()), score))
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for (bench, winner, score) in bench_winners {
        let bar = make_bar(score, 100.0, 10);
        let label = format!("{:<24}", truncate(&bench, 24));
        let winner_fmt = format!("{:<28}", truncate(&winner, 28));
        println!(
            "  \x1b[38;5;114m{label}\x1b[0m  \x1b[38;5;250m{winner_fmt}\x1b[0m  {bar}  \x1b[38;5;220m{:>5.1}\x1b[0m",
            score
        );
    }
    println!();
    Ok(())
}

fn query_ranked_models(
    conn: &Connection,
    category: Option<&str>,
    limit: u32,
) -> rusqlite::Result<Vec<RankedModel>> {
    let sql = "
        SELECT m.id, m.name, m.creator, m.source,
               AVG(CASE WHEN s.max_score IS NULL OR s.max_score <= 1.0
                        THEN s.score * 100.0 ELSE s.score END) AS avg_score,
               COUNT(s.benchmark) AS bench_count
        FROM models m
        JOIN scores s ON s.model_id = m.id
        WHERE (?1 IS NULL OR s.category = ?1)
        GROUP BY m.id, m.name, m.creator, m.source
        HAVING bench_count >= 1
        ORDER BY avg_score DESC
        LIMIT ?2
    ";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params![category, limit], |row| {
        Ok(RankedModel {
            id: row.get(0)?,
            name: row.get(1)?,
            creator: row.get(2)?,
            source: row.get(3)?,
            avg_score: row.get(4)?,
            benchmark_count: row.get(5)?,
        })
    })?;
    rows.collect::<Result<_, _>>()
}

fn print_rows(models: &[RankedModel]) {
    println!(
        "  \x1b[38;5;246m     {:<27}  {:<14}  {:<14}  {:>5}  {:<10}\x1b[0m",
        "model", "creator", "score", "avg", "source"
    );
    println!(
        "  \x1b[38;5;237m──────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
    );
    for (idx, m) in models.iter().enumerate() {
        let rank = idx + 1;
        let marker = match rank {
            1 => "\x1b[38;5;220m◆\x1b[0m",
            2 => "\x1b[38;5;243m◆\x1b[0m",
            3 => "\x1b[38;5;130m◆\x1b[0m",
            _ => "\x1b[38;5;242m◆\x1b[0m",
        };
        let name_fmt = format!("{:<27}", truncate(&m.name, 27));
        let creator_fmt = format!("{:<14}", truncate(&m.creator, 14));
        let bar = make_bar(m.avg_score, 100.0, 14);
        let tag = source_tag(&m.source);
        println!(
            "  \x1b[38;5;246m{:>2}.\x1b[0m {marker} \x1b[1m{name_fmt}\x1b[0m  \x1b[38;5;250m{creator_fmt}\x1b[0m  {bar}  \x1b[38;5;43m{:>5.1}\x1b[0m  {tag}",
            rank,
            m.avg_score,
        );
    }
}

fn source_tag(source: &str) -> String {
    match source {
        "merged" => "\x1b[38;5;114m[merged]\x1b[0m".to_string(),
        "llm-stats" => "\x1b[38;5;246m[llm-stats]\x1b[0m".to_string(),
        "aa" => "\x1b[38;5;246m[aa]\x1b[0m".to_string(),
        other => format!("\x1b[38;5;242m[{}]\x1b[0m", other),
    }
}

fn titleize(s: &str) -> String {
    let mut chars = s.chars();
    let first = chars.next().unwrap_or(' ').to_uppercase().to_string();
    first + chars.as_str()
}
