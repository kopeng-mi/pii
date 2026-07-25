use crate::session::types::{CallRow, SessionRow};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn sync_sessions(conn: &Connection) -> rusqlite::Result<()> {
    let mut sessions_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    sessions_dir.push(".pi");
    sessions_dir.push("agent");
    sessions_dir.push("sessions");

    if !sessions_dir.exists() {
        return Ok(());
    }

    let mut stmt = conn.prepare("SELECT file_path, file_size FROM sessions")?;
    let mut current_cache = std::collections::HashMap::new();
    let rows = stmt.query_map([], |row| {
        let path: String = row.get(0)?;
        let size: u64 = row.get(1)?;
        Ok((path, size))
    })?;

    for row in rows {
        if let Ok((path, size)) = row {
            current_cache.insert(path, size);
        }
    }

    let entries = match fs::read_dir(&sessions_dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        if !entry.file_type().map_or(false, |ft| ft.is_dir()) {
            continue;
        }
        let project_dir = entry.path();
        let project_name = entry.file_name().to_string_lossy().to_string();
        let display_project = decode_project_name(&project_name);

        let files = match fs::read_dir(&project_dir) {
            Ok(f) => f,
            Err(_) => continue,
        };

        for file_entry in files.flatten() {
            let file_path = file_entry.path();
            if file_path.extension().map_or(true, |ext| ext != "jsonl") {
                continue;
            }

            let path_str = file_path.to_string_lossy().to_string();
            let metadata = match fs::metadata(&file_path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size = metadata.len();

            if let Some(&cached_size) = current_cache.get(&path_str) {
                if cached_size == size {
                    continue; // Skip, not modified
                }
            }

            // Parse file and insert/update DB
            if let Some((session_row, call_rows)) =
                parse_session_file(&file_path, &display_project, size)
            {
                insert_session(conn, &session_row, &call_rows)?;
            }
        }
    }

    Ok(())
}

fn decode_project_name(encoded: &str) -> String {
    // Basic decode from "--C--Users-..."
    let mut decoded = encoded.replace("--", "/").replace("-", "/");
    if decoded.starts_with('/') {
        decoded.remove(0);
    }
    if decoded.ends_with('/') {
        decoded.pop();
    }
    // Return last component as short name
    Path::new(&decoded)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(decoded)
}

fn parse_session_file(
    path: &Path,
    project: &str,
    file_size: u64,
) -> Option<(SessionRow, Vec<CallRow>)> {
    let content = fs::read_to_string(path).ok()?;

    let mut id = String::new();
    let mut timestamp = String::new();
    let mut prompt = String::new();

    let mut total_calls = 0;
    let mut total_tokens = 0;
    let mut total_cost = 0.0;
    let mut errors = 0;

    let mut calls = Vec::new();

    for line in content.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            let t = value["type"].as_str().unwrap_or("");
            if t == "session" {
                id = value["id"].as_str().unwrap_or("").to_string();
                timestamp = value["timestamp"].as_str().unwrap_or("").to_string();
            } else if t == "message" {
                if let Some(role) = value
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                {
                    if role == "user" && prompt.is_empty() {
                        if let Some(content) =
                            value["message"].get("content").and_then(|c| c.as_array())
                        {
                            if let Some(first) = content.first() {
                                if let Some(text) = first.get("text").and_then(|t| t.as_str()) {
                                    prompt = text.to_string();
                                }
                            }
                        }
                    }
                }

                // If usage is present either at root or inside message, log the call
                let mut usage_obj = value.get("usage").and_then(|u| u.as_object());
                let mut model_str = value.get("model").and_then(|m| m.as_str());
                let mut error_val = value.get("stopReason").and_then(|s| s.as_str());

                if usage_obj.is_none() {
                    // Try looking inside message (older format)
                    if let Some(msg_obj) = value.get("message") {
                        usage_obj = msg_obj.get("usage").and_then(|u| u.as_object());
                        if model_str.is_none() {
                            model_str = msg_obj.get("model").and_then(|m| m.as_str());
                        }
                        if error_val.is_none() {
                            error_val = msg_obj.get("stopReason").and_then(|s| s.as_str());
                        }
                    }
                }

                if let Some(usage) = usage_obj {
                    total_calls += 1;
                    let input = usage.get("input").and_then(|v| v.as_f64()).unwrap_or(0.0) as u32;
                    let output = usage.get("output").and_then(|v| v.as_f64()).unwrap_or(0.0) as u32;
                    let tokens = input + output;
                    total_tokens += tokens;

                    let cost = usage
                        .get("cost")
                        .and_then(|v| v.as_object())
                        .and_then(|c| c.get("total").and_then(|t| t.as_f64()))
                        .unwrap_or(0.0);
                    total_cost += cost;

                    let model = model_str.unwrap_or("unknown").to_string();
                    let is_error = error_val == Some("error");
                    if is_error {
                        errors += 1;
                    }

                    calls.push(CallRow {
                        session_id: id.clone(),
                        model,
                        input_tokens: input,
                        output_tokens: output,
                        tokens,
                        cost,
                        is_error,
                    });
                }
            } else if t == "model_change" {
                // Ignore for now
            }
        }
    }

    if id.is_empty() {
        return None;
    }

    // "2026-07-25T05:10:12.669Z"
    let date = timestamp.split('T').next().unwrap_or("").to_string();
    let time = timestamp
        .split('T')
        .nth(1)
        .and_then(|t| t.get(0..5))
        .unwrap_or("")
        .to_string();

    let session = SessionRow {
        id,
        project: project.to_string(),
        file_path: path.to_string_lossy().to_string(),
        file_size,
        date,
        time,
        prompt,
        total_calls,
        total_tokens,
        total_cost,
        errors,
    };

    Some((session, calls))
}

fn insert_session(
    conn: &Connection,
    session: &SessionRow,
    calls: &[CallRow],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sessions (id, project, file_path, file_size, date, time, prompt, total_calls, total_tokens, total_cost, errors)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        (
            &session.id,
            &session.project,
            &session.file_path,
            session.file_size,
            &session.date,
            &session.time,
            &session.prompt,
            session.total_calls,
            session.total_tokens,
            session.total_cost,
            session.errors,
        ),
    )?;

    conn.execute("DELETE FROM calls WHERE session_id = ?1", [&session.id])?;

    let mut stmt = conn.prepare(
        "INSERT INTO calls (session_id, model, input_tokens, output_tokens, tokens, cost, is_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;

    for call in calls {
        stmt.execute((
            &call.session_id,
            &call.model,
            call.input_tokens,
            call.output_tokens,
            call.tokens,
            call.cost,
            call.is_error,
        ))?;
    }

    Ok(())
}
