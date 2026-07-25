use chrono::{Datelike, Duration, Utc};
use rusqlite::Connection;
use std::collections::HashMap;

const DAYS_TO_SHOW: i64 = 150; // Roughly 5 months to fit standard terminal

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

    // HashMap mapping YYYY-MM-DD -> daily_tokens
    let mut daily_data: HashMap<String, u64> = HashMap::new();
    let mut max_tokens: u64 = 0;
    let mut total_tokens = 0;
    let mut total_sessions = 0;
    let mut active_days = 0;

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

    if active_days == 0 {
        println!("\n  No activity found in the last {} days.", DAYS_TO_SHOW);
        return Ok(());
    }

    // Determine buckets for coloring
    // We'll use 4 buckets above 0
    let bucket_size = (max_tokens as f64 / 4.0).ceil() as u64;

    fn get_color(tokens: u64, bucket_size: u64) -> &'static str {
        if tokens == 0 {
            "\x1b[38;5;236m" // Very dark gray for 0
        } else if tokens <= bucket_size {
            "\x1b[38;5;22m" // Dark green
        } else if tokens <= bucket_size * 2 {
            "\x1b[38;5;28m" // Green
        } else if tokens <= bucket_size * 3 {
            "\x1b[38;5;40m" // Bright green
        } else {
            "\x1b[38;5;47m" // Neon green
        }
    }

    println!(
        "\n  \x1b[38;5;43m━━\x1b[0m \x1b[1mActivity Heatmap\x1b[0m · Last {} Days\n",
        DAYS_TO_SHOW
    );

    // Grid construction
    // Rows: Sunday, Monday, Tuesday, Wednesday, Thursday, Friday, Saturday (0 to 6)
    // Columns: Weeks

    // Find the start date's weekday
    let mut current_date = start_date;
    // Align start to the beginning of the week (Sunday)
    while current_date.weekday().num_days_from_sunday() != 0 {
        current_date -= Duration::days(1);
    }

    let mut weeks = Vec::new();
    let mut current_week = [""; 7];
    let mut day_index = 0;

    let mut loop_date = current_date;
    while loop_date <= today {
        let ds = loop_date.format("%Y-%m-%d").to_string();

        // Skip rendering days before our intended start date in the first week, just put spaces
        let token_count = if loop_date < start_date {
            None
        } else {
            Some(*daily_data.get(&ds).unwrap_or(&0))
        };

        if let Some(tokens) = token_count {
            let color = get_color(tokens, bucket_size);
            // using a solid block '■' or standard block '▇'
            current_week[day_index] = color;
        } else {
            current_week[day_index] = ""; // empty string for out-of-bounds days
        }

        day_index += 1;
        if day_index == 7 {
            weeks.push(current_week);
            current_week = [""; 7];
            day_index = 0;
        }
        loop_date += Duration::days(1);
    }
    if day_index != 0 {
        weeks.push(current_week);
    }

    // Print the grid
    let day_labels = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    for day_idx in 0..7 {
        // Only print labels for Mon, Wed, Fri
        if day_idx == 1 || day_idx == 3 || day_idx == 5 {
            print!("  \x1b[38;5;242m{}\x1b[0m ", day_labels[day_idx]);
        } else {
            print!("      ");
        }

        for week in &weeks {
            let color = week[day_idx];
            if color.is_empty() {
                print!("  "); // 2 spaces
            } else {
                print!("{}■ \x1b[0m", color); // Use ■ followed by space
            }
        }
        println!();
    }

    // Legend and summary
    println!(
        "\n  \x1b[38;5;246mSessions:\x1b[0m \x1b[1m{}\x1b[0m",
        total_sessions
    );
    println!(
        "  \x1b[38;5;246mTotal Tokens:\x1b[0m \x1b[1m{}\x1b[0m",
        crate::ui::table::compact_num(total_tokens)
    );
    println!(
        "  \x1b[38;5;246mActive Days:\x1b[0m \x1b[1m{}\x1b[0m",
        active_days
    );

    print!("  \x1b[38;5;246mLess \x1b[0m");
    print!("\x1b[38;5;236m■ \x1b[0m");
    print!("\x1b[38;5;22m■ \x1b[0m");
    print!("\x1b[38;5;28m■ \x1b[0m");
    print!("\x1b[38;5;40m■ \x1b[0m");
    print!("\x1b[38;5;47m■ \x1b[0m");
    println!("\x1b[38;5;246m More\x1b[0m\n");

    Ok(())
}
