use crate::session::types::{CallRow, SessionRow};
use crate::models::types::UnifiedModel;
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
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

    // Pre-fetch all models once for cost estimation
    let mut db_models = Vec::new();
    if let Ok(mut stmt_m) = conn.prepare("SELECT id, name, input_price, output_price FROM models") {
        if let Ok(rows) = stmt_m.query_map([], |row| {
            Ok(UnifiedModel {
                id: row.get(0)?,
                name: row.get(1)?,
                creator: "".to_string(),
                release_date: "".to_string(),
                context_window: None,
                param_count: None,
                input_price: row.get(2)?,
                output_price: row.get(3)?,
                speed_tok_s: None,
                ttft_s: None,
                open_weight: false,
                source: "".to_string(),
                raw_json: "".to_string(),
            })
        }) {
            for r in rows.flatten() {
                db_models.push(r);
            }
        }
    }

    // Buffer for project dirs and files we plan to process
    let mut planned: Vec<(PathBuf, String, PathBuf, u64, String)> = Vec::new();

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

            planned.push((project_dir.clone(), display_project.clone(), file_path, size, path_str));
        }
    }

    if planned.is_empty() {
        return Ok(());
    }

    // Wrap all writes in a single transaction — massive speedup for bulk inserts.
    conn.execute_batch("BEGIN")?;

    let result = (|| -> rusqlite::Result<()> {
        // Fuzzy match cache persists across all calls in this sync run.
        let mut cost_cache: std::collections::HashMap<String, Option<usize>> = std::collections::HashMap::new();

        let mut pb = crate::ui::progress::Progress::new("Syncing sessions", planned.len() as u64);
        for (i, (_, display_project, file_path, size, _path_str)) in planned.into_iter().enumerate() {
            if let Some((mut session_row, mut call_rows)) =
                parse_session_file(&file_path, &display_project, size)
            {
                if session_row.total_cost == 0.0 && session_row.total_tokens > 0 && !db_models.is_empty() {
                    let mut session_cost = 0.0;
                    for call in &mut call_rows {
                        if call.cost == 0.0 && call.tokens > 0 {
                            if let Some(est) = crate::models::fuzzy::estimate_cost_cached(
                                &call.model, call.input_tokens, call.output_tokens,
                                &db_models, &mut cost_cache,
                            ) {
                                call.cost = est;
                            }
                        }
                        session_cost += call.cost;
                    }
                    session_row.total_cost = session_cost;
                }

                // last_model = the model from the chronologically last call
                session_row.last_model = call_rows.last().map(|c| c.model.clone()).unwrap_or_default();

                insert_session(conn, &session_row, &call_rows)?;
            }
            pb.tick((i as u64) + 1);
        }
        pb.finish();
        Ok(())
    })();

    match result {
        Ok(()) => { conn.execute_batch("COMMIT")?; }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(e);
        }
    }

    Ok(())
}

fn decode_project_name(encoded: &str) -> String {
    let mut decoded = encoded.replace("--", "/").replace("-", "/");
    if decoded.starts_with('/') {
        decoded.remove(0);
    }
    if decoded.ends_with('/') {
        decoded.pop();
    }
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
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut id = String::new();
    let mut timestamp = String::new();
    let mut prompt = String::new();

    let mut total_calls = 0;
    let mut total_tokens = 0;
    let mut total_cost = 0.0;
    let mut errors = 0;

    let mut calls = Vec::new();
    let mut unique_models = std::collections::HashSet::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
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

                let mut usage_obj = value.get("usage").and_then(|u| u.as_object());
                let mut model_str = value.get("model").and_then(|m| m.as_str());
                let mut error_val = value.get("stopReason").and_then(|s| s.as_str());

                if usage_obj.is_none() {
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

                    unique_models.insert(model.clone());

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
            }
        }
    }

    if id.is_empty() {
        return None;
    }

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
        models: unique_models.into_iter().collect::<Vec<_>>().join(" "),
        total_calls,
        total_tokens,
        total_cost,
        errors,
        last_model: String::new(), // Populated by caller after cost estimation
    };

    Some((session, calls))
}

fn insert_session(
    conn: &Connection,
    session: &SessionRow,
    calls: &[CallRow],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sessions (id, project, file_path, file_size, date, time, prompt, models, total_calls, total_tokens, total_cost, errors, last_model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        (
            &session.id,
            &session.project,
            &session.file_path,
            session.file_size,
            &session.date,
            &session.time,
            &session.prompt,
            &session.models,
            session.total_calls,
            session.total_tokens,
            session.total_cost,
            session.errors,
            &session.last_model,
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
