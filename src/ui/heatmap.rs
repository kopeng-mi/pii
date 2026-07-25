use chrono::{Datelike, Duration, NaiveDate, Utc};
use rusqlite::Connection;
use std::collections::HashMap;

const DAYS_TO_SHOW: i64 = 150;

pub fn print_heatmap(conn: &Connection) -> rusqlite::Result<()> {
    let today = Utc::now().date_naive();
    let start_date = today - Duration::days(DAYS_TO_SHOW);
    let start_date_str = start_date.format("%Y-%m-%d").to_string();

    let sql = format!(
        "SELECT date, SUM(total_tokens) as daily_tokens, COUNT(id) as daily_sessions
         FROM sessions
         WHERE date >= '{}'
         GROUP BY date",
        start_date_str
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;

    let mut daily_data: HashMap<String, u64> = HashMap::new();
    let mut max_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut total_sessions: u32 = 0;
    let mut active_days: u32 = 0;

    while let Some(row) = rows.next()? {
        let date_str: String = row.get(0)?;
        let tokens: u32 = row.get(1)?;
        let sessions: u32 = row.get(2)?;

        let tokens_u64 = tokens as u64;
        daily_data.insert(date_str, tokens_u64);

        if tokens_u64 > max_tokens {
            max_tokens = tokens_u64;
        }
        total_tokens += tokens_u64;
        total_sessions += sessions;
        active_days += 1;
    }

    // Grab total cost for the period
    let cost_sql = format!(
        "SELECT COALESCE(SUM(total_cost), 0) FROM sessions WHERE date >= '{}'",
        start_date_str
    );
    let total_cost: f64 = conn.query_row(&cost_sql, [], |r| r.get(0))?;

    if active_days == 0 {
        println!("\n  No activity found in the last {} days.", DAYS_TO_SHOW);
        return Ok(());
    }

    let bucket_size = (max_tokens as f64 / 4.0).ceil() as u64;

    fn get_color(tokens: u64, bucket_size: u64) -> &'static str {
        if tokens == 0 {
            "\x1b[38;5;236m"
        } else if tokens <= bucket_size {
            "\x1b[38;5;30m"
        } else if tokens <= bucket_size * 2 {
            "\x1b[38;5;37m"
        } else if tokens <= bucket_size * 3 {
            "\x1b[38;5;43m"
        } else {
            "\x1b[38;5;51m"
        }
    }

    println!(
        "\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mActivity Heatmap\x1b[0m · Last {} Days\n",
        DAYS_TO_SHOW
    );

    // Align start to Sunday
    let mut grid_start = start_date;
    while grid_start.weekday().num_days_from_sunday() != 0 {
        grid_start -= Duration::days(1);
    }

    // Build weeks grid with date tracking for month labels
    struct WeekCol {
        colors: [&'static str; 7],
        first_date: NaiveDate, // Sunday of this week
    }

    let mut weeks: Vec<WeekCol> = Vec::new();
    let mut current_colors: [&str; 7] = [""; 7];
    let mut week_start = grid_start;
    let mut day_index = 0;

    let mut loop_date = grid_start;
    while loop_date <= today {
        let ds = loop_date.format("%Y-%m-%d").to_string();

        let token_count = if loop_date < start_date {
            None
        } else {
            Some(*daily_data.get(&ds).unwrap_or(&0))
        };

        current_colors[day_index] = if let Some(tokens) = token_count {
            get_color(tokens, bucket_size)
        } else {
            ""
        };

        day_index += 1;
        if day_index == 7 {
            weeks.push(WeekCol {
                colors: current_colors,
                first_date: week_start,
            });
            current_colors = [""; 7];
            day_index = 0;
            week_start = loop_date + Duration::days(1);
        }
        loop_date += Duration::days(1);
    }
    if day_index != 0 {
        weeks.push(WeekCol {
            colors: current_colors,
            first_date: week_start,
        });
    }

    // ── Month labels row ──
    // Each week column below is 2 visible chars wide ("■ "). A 3-char month
    // name (e.g. "Jan") therefore spans 1.5 cells. To keep alignment perfect,
    // we render the label as " " + name (4 chars = 2 cells), then skip the
    // following cell so the next label lands on a fresh column.
    print!("      "); // align with day labels
    let month_names = ["", "Jan", "Feb", "Mar", "Apr", "May", "Jun",
                        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut last_month: u32 = weeks.first().map(|w| w.first_date.month()).unwrap_or(0);
    let mut skip_next = false;
    for week in &weeks {
        if skip_next {
            skip_next = false;
            continue;
        }
        let m = week.first_date.month();
        if m != last_month {
            let name = month_names[m as usize];
            // Print padded to exactly 4 visible chars = 2 cells.
            print!("\x1b[38;5;246m {}\x1b[0m", name);
            last_month = m;
            skip_next = true;
        } else {
            print!("  ");
        }
    }
    println!();

    // ── Grid ──
    let day_labels = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    for day_idx in 0..7 {
        if day_idx == 1 || day_idx == 3 || day_idx == 5 {
            print!("  \x1b[38;5;242m{}\x1b[0m ", day_labels[day_idx]);
        } else {
            print!("      ");
        }

        for week in &weeks {
            let color = week.colors[day_idx];
            if color.is_empty() {
                print!("  ");
            } else {
                print!("{}■ \x1b[0m", color);
            }
        }
        println!();
    }

    // ── Stats ──
    println!();
    println!(
        "  \x1b[38;5;43m◈\x1b[0m Sessions    \x1b[1m{}\x1b[0m",
        total_sessions
    );
    println!(
        "  \x1b[38;5;114m◆\x1b[0m Tokens      \x1b[1m{}\x1b[0m",
        crate::ui::table::compact_num(total_tokens)
    );
    println!(
        "  \x1b[38;5;220m$\x1b[0m Cost        \x1b[1m\x1b[38;5;220m${:.2}\x1b[0m",
        total_cost
    );
    println!(
        "  \x1b[38;5;246m▸\x1b[0m Active Days  \x1b[1m{}\x1b[0m \x1b[38;5;242m/ {} total\x1b[0m",
        active_days, DAYS_TO_SHOW
    );

    // Legend
    print!("\n  \x1b[38;5;242mLess\x1b[0m ");
    print!("\x1b[38;5;236m■\x1b[0m ");
    print!("\x1b[38;5;30m■\x1b[0m ");
    print!("\x1b[38;5;37m■\x1b[0m ");
    print!("\x1b[38;5;43m■\x1b[0m ");
    print!("\x1b[38;5;51m■\x1b[0m ");
    println!("\x1b[38;5;242mMore\x1b[0m\n");

    Ok(())
}
