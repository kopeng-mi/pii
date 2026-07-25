use super::types::{Evaluation, UnifiedModel};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::env;
use std::thread;

const LLM_STATS_BASE: &str = "https://api.llm-stats.com/stats/v1";
const AA_BASE: &str = "https://artificialanalysis.ai/api/v2";

fn fetch_llm_stats() -> (Vec<UnifiedModel>, Vec<Evaluation>) {
    let mut unified_models = Vec::new();
    let mut evaluations = Vec::new();
    if let Ok(llm_key) = env::var("LLM_STATS_API_KEY") {
        if let Ok(res) = ureq::get(&format!("{}/models?limit=200", LLM_STATS_BASE))
            .set("Authorization", &format!("Bearer {}", llm_key))
            .call()
        {
            if let Ok(json) = res.into_json::<Value>() {
                if let Some(models) = json.get("models").and_then(|m| m.as_array()) {
                    for m in models {
                        if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                            let mut input_price = 0.0;
                            let mut output_price = 0.0;
                            if let Some(providers) = m.get("providers").and_then(|p| p.as_array()) {
                                for p in providers {
                                    if input_price == 0.0 {
                                        input_price = p.get("input_price_per_m").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                        output_price = p.get("output_price_per_m").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    }
                                }
                            }
                            if let Some(scores) = m.get("top_scores").and_then(|s| s.as_object()) {
                                for (bench_id, bench_val) in scores {
                                    if let Some(s) = bench_val.get("score").and_then(|v| v.as_f64()) {
                                        evaluations.push(Evaluation {
                                            model_id: id.to_string(),
                                            benchmark: bench_val.get("benchmark_name").and_then(|v| v.as_str()).unwrap_or(bench_id).to_string(),
                                            score: s,
                                            max_score: bench_val.get("max_score").and_then(|v| v.as_f64()),
                                            category: bench_val.get("category").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                        });
                                    }
                                }
                            }
                            unified_models.push(UnifiedModel {
                                id: id.to_string(),
                                name: m.get("name").and_then(|v| v.as_str()).unwrap_or(id).to_string(),
                                creator: m.get("organization").and_then(|o| o.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                release_date: m.get("release_date").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                context_window: m.get("context_window").and_then(|v| v.as_u64()).map(|v| v as u32),
                                param_count: m.get("param_count").and_then(|v| v.as_u64()),
                                input_price,
                                output_price,
                                speed_tok_s: None,
                                ttft_s: None,
                                open_weight: m.get("open_weight").and_then(|v| v.as_bool()).unwrap_or(false),
                                source: "llm-stats".to_string(),
                                raw_json: serde_json::to_string(m).unwrap_or_default(),
                            });
                        }
                    }
                }
            }
        }
    }
    (unified_models, evaluations)
}

fn fetch_aa() -> Value {
    if let Ok(aa_key) = env::var("ARTIFICIALANALYSIS_API_KEY") {
        if let Ok(res) = ureq::get(&format!("{}/data/llms/models", AA_BASE))
            .set("x-api-key", &aa_key)
            .call()
        {
            if let Ok(json) = res.into_json::<Value>() {
                return json;
            }
        }
    }
    Value::Null
}

pub fn fetch_models() -> Result<(Vec<UnifiedModel>, Vec<Evaluation>), Box<dyn std::error::Error>> {
    let llm_handle = thread::spawn(fetch_llm_stats);
    let aa_handle = thread::spawn(fetch_aa);

    let (llm_models, mut evaluations) = llm_handle.join().unwrap_or_default();
    let aa_json = aa_handle.join().unwrap_or(Value::Null);

    let mut unified_models: std::collections::HashMap<String, UnifiedModel> = 
        llm_models.into_iter().map(|m| (m.id.clone(), m)).collect();

    // 2. Fetch from Artificial Analysis (Enrichment / Fallback)
    if let Some(data) = aa_json.get("data").and_then(|d| d.as_array()) {
        for m in data {
            if let Some(aa_id) = m.get("id").and_then(|v| v.as_str()) {
                let speed = m.get("median_output_tokens_per_second").and_then(|v| v.as_f64());
                let ttft = m.get("median_time_to_first_token_seconds").and_then(|v| v.as_f64());
                
                let mut input_price = 0.0;
                let mut output_price = 0.0;
                if let Some(pricing) = m.get("pricing") {
                    input_price = pricing.get("price_1m_input_tokens").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    output_price = pricing.get("price_1m_output_tokens").and_then(|v| v.as_f64()).unwrap_or(0.0);
                }

                let aa_name = m.get("name").and_then(|v| v.as_str()).unwrap_or(aa_id);
                let matched_key = crate::models::fuzzy::fuzzy_match_model(aa_id, aa_name, unified_models.values());

                if let Some(evals) = m.get("evaluations").and_then(|e| e.as_object()) {
                    for (bench_id, bench_val) in evals {
                        if let Some(s) = bench_val.as_f64() {
                            let max_score = if bench_id.contains("index") { 100.0 } else { 1.0 };
                            let category = if bench_id.contains("coding") || bench_id.contains("livecodebench") { "coding" }
                                else if bench_id.contains("math") { "math" }
                                else { "general" };

                            evaluations.push(Evaluation {
                                model_id: matched_key.clone().unwrap_or(aa_id.to_string()),
                                benchmark: bench_id.to_string(),
                                score: s,
                                max_score: Some(max_score),
                                category: category.to_string(),
                            });
                        }
                    }
                }

                if let Some(k) = &matched_key {
                    let entry = unified_models.get_mut(k).unwrap();
                    entry.speed_tok_s = entry.speed_tok_s.or(speed);
                    entry.ttft_s = entry.ttft_s.or(ttft);
                    if entry.input_price == 0.0 {
                        entry.input_price = input_price;
                        entry.output_price = output_price;
                    }
                    entry.source = "merged".to_string();
                } else {
                    let model = UnifiedModel {
                        id: aa_id.to_string(),
                        name: m.get("name").and_then(|v| v.as_str()).unwrap_or(aa_id).to_string(),
                        creator: m.get("model_creator").and_then(|c| c.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        release_date: "".to_string(),
                        context_window: None,
                        param_count: None,
                        input_price,
                        output_price,
                        speed_tok_s: speed,
                        ttft_s: ttft,
                        open_weight: false,
                        source: "aa".to_string(),
                        raw_json: serde_json::to_string(m).unwrap_or_default(),
                    };
                    unified_models.insert(aa_id.to_string(), model);
                }
            }
        }
    }

    Ok((unified_models.into_values().collect(), evaluations))
}

#[allow(dead_code)]
pub fn save_models(conn: &rusqlite::Connection, models: &[UnifiedModel], evals: &[Evaluation]) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO models (id, name, creator, release_date, context_window, param_count, input_price, output_price, speed_tok_s, ttft_s, open_weight, source, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
    )?;

    for m in models {
        stmt.execute(rusqlite::params![
            m.id,
            m.name,
            m.creator,
            m.release_date,
            m.context_window,
            m.param_count,
            m.input_price,
            m.output_price,
            m.speed_tok_s,
            m.ttft_s,
            m.open_weight,
            m.source,
            m.raw_json,
        ])?;
    }

    let mut stmt_evals = conn.prepare(
        "INSERT OR REPLACE INTO scores (model_id, benchmark, score, max_score, category)
         VALUES (?1, ?2, ?3, ?4, ?5)"
    )?;

    for e in evals {
        stmt_evals.execute(rusqlite::params![
            e.model_id,
            e.benchmark,
            e.score,
            e.max_score,
            e.category,
        ])?;
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('models_fetched_date', ?1)",
        [&today],
    )?;

    Ok(())
}

pub fn refresh_if_needed(conn: &rusqlite::Connection, force: bool) -> rusqlite::Result<()> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut stmt = conn.prepare("SELECT value FROM meta WHERE key = 'models_fetched_date'")?;
    let last_fetched: Option<String> = stmt.query_row([], |row| row.get(0)).optional()?;

    if force || last_fetched.is_none() || last_fetched.unwrap() != today {
        // Two parallel network calls + DB writes. Show three lines of progress
        // so the user knows nothing is hung.
        let llm_pb = crate::ui::progress::Progress::spinner("Fetching LLM-Stats");
        let aa_pb = crate::ui::progress::Progress::spinner("Fetching Artificial Analysis");
        if let Ok((models, evals)) = fetch_models() {
            llm_pb.finish();
            aa_pb.finish();
            let save_pb = crate::ui::progress::Progress::new("Saving models", (models.len() + evals.len()) as u64);
            save_models_with_progress(conn, &models, &evals, save_pb)?;
        } else {
            llm_pb.fail("network error");
            aa_pb.fail("network error");
        }
    }
    Ok(())
}

/// Like `save_models` but ticks a progress bar as rows land. Two passes:
/// first models (large rows), then evaluations (smaller rows).
fn save_models_with_progress(
    conn: &rusqlite::Connection,
    models: &[UnifiedModel],
    evals: &[Evaluation],
    mut pb: crate::ui::progress::Progress,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT OR REPLACE INTO models (id, name, creator, release_date, context_window, param_count, input_price, output_price, speed_tok_s, ttft_s, open_weight, source, raw_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
    )?;
    for (i, m) in models.iter().enumerate() {
        stmt.execute(rusqlite::params![
            m.id, m.name, m.creator, m.release_date,
            m.context_window, m.param_count, m.input_price, m.output_price,
            m.speed_tok_s, m.ttft_s, m.open_weight, m.source, m.raw_json,
        ])?;
        pb.tick((i as u64) + 1);
    }

    let model_total = models.len() as u64;
    let mut stmt_evals = conn.prepare(
        "INSERT OR REPLACE INTO scores (model_id, benchmark, score, max_score, category)
         VALUES (?1, ?2, ?3, ?4, ?5)"
    )?;
    for (i, e) in evals.iter().enumerate() {
        stmt_evals.execute(rusqlite::params![e.model_id, e.benchmark, e.score, e.max_score, e.category])?;
        pb.tick(model_total + (i as u64) + 1);
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('models_fetched_date', ?1)",
        [&today],
    )?;
    pb.finish();
    Ok(())
}
