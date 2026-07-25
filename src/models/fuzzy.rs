use crate::models::types::UnifiedModel;
use nucleo::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher, Utf32String,
};

pub fn fuzzy_match_model<'a, I>(aa_id: &str, aa_name: &str, candidates: I) -> Option<String>
where
    I: Iterator<Item = &'a UnifiedModel>,
{
    // Clean up IDs first (common normalizations)
    let cleaned_id = aa_id
        .to_lowercase()
        .replace("anthropic/", "")
        .replace("openai/", "")
        .replace("google/", "")
        .replace("deepseek/", "")
        .replace("-thinking", "")
        .replace(".", "-");
        
    let cleaned_name = aa_name
        .to_lowercase()
        .replace(" (", "-")
        .replace(")", "")
        .replace(" ", "-")
        .replace(".", "-");

    // Hardcoded known mapping overlaps because LLM-Stats drops "claude-fable-5" 
    // while AA emits "cd55210d-358e..." with Name: "Claude Fable 5 (...)"
    let stripped_name = aa_name
        .to_lowercase()
        .split(" (")
        .next()
        .unwrap_or(aa_name)
        .replace(" ", "-")
        .replace(".", "-");

    let mut best_score = 0;
    let mut best_match = None;
    
    let id_pattern = Pattern::parse(&cleaned_id, CaseMatching::Ignore, Normalization::Smart);
    let name_pattern = Pattern::parse(&cleaned_name, CaseMatching::Ignore, Normalization::Smart);
    let stripped_pattern = Pattern::parse(&stripped_name, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);

    for candidate in candidates {
        let clean_c_id = candidate.id.to_lowercase();
        let clean_c_name = candidate.name.to_lowercase();
        
        // Fast paths: exact id or name
        if clean_c_id == cleaned_id || clean_c_name == cleaned_name || clean_c_id == stripped_name || clean_c_name == stripped_name {
            return Some(candidate.id.clone());
        }
        if clean_c_id.starts_with(&cleaned_id) || cleaned_id.starts_with(&clean_c_id) {
            return Some(candidate.id.clone());
        }

        // Fuzzy match via nucleo on ID
        let haystack_name = Utf32String::from(clean_c_id.as_str());
        let mut positions = Vec::new();
        if let Some(score) = id_pattern.indices(haystack_name.slice(..), &mut matcher, &mut positions) {
            if score > best_score && score > 60 {
                best_score = score;
                best_match = Some(candidate.id.clone());
            }
        }
        
        // Fuzzy match via nucleo on Name
        let haystack_name = Utf32String::from(clean_c_name.as_str());
        let mut positions = Vec::new();
        if let Some(score) = name_pattern.indices(haystack_name.slice(..), &mut matcher, &mut positions) {
            if score > best_score && score > 60 {
                best_score = score;
                best_match = Some(candidate.id.clone());
            }
        }

        // Fuzzy match via nucleo on Stripped Name
        if let Some(score) = stripped_pattern.indices(haystack_name.slice(..), &mut matcher, &mut positions) {
            if score > best_score && score > 60 {
                best_score = score;
                best_match = Some(candidate.id.clone());
            }
        }
    }

    best_match
}

#[allow(dead_code)]
pub fn estimate_cost(model_name: &str, in_tokens: u32, out_tokens: u32, candidates: &[UnifiedModel]) -> Option<f64> {
    let mut best_score = 0;
    let mut matched_model = None;

    // Quick cleaning of model name from Pi
    let cleaned_name = model_name
        .to_lowercase()
        .replace("anthropic/", "")
        .replace("openai/", "")
        .replace("google/", "")
        .replace("deepseek/", "")
        .replace("tencent/", "")
        .replace("xiaomi/", "")
        .replace("minimaxai/", "")
        .replace("-thinking", "")
        .replace(".0", "")
        .replace(".", "-");

    let name_pattern = Pattern::parse(&cleaned_name, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);

    for candidate in candidates {
        let clean_c_id = candidate.id.to_lowercase();

        if clean_c_id == cleaned_name || clean_c_id.starts_with(&cleaned_name) || cleaned_name.starts_with(&clean_c_id) {
            matched_model = Some(candidate);
            break;
        }

        let haystack_name = Utf32String::from(clean_c_id.as_str());
        let mut positions = Vec::new();
        if let Some(score) = name_pattern.indices(haystack_name.slice(..), &mut matcher, &mut positions) {
            if score > best_score && score > 60 {
                best_score = score;
                matched_model = Some(candidate);
            }
        }
    }

    if let Some(m) = matched_model {
        let in_cost = (in_tokens as f64 / 1_000_000.0) * m.input_price;
        let out_cost = (out_tokens as f64 / 1_000_000.0) * m.output_price;
        let total = in_cost + out_cost;
        if total > 0.0 {
            Some(total)
        } else {
            None
        }
    } else {
        None
    }
}

/// Cached cost estimation. First call per model name is O(N) fuzzy match;
/// subsequent calls are O(1) HashMap lookups. Cache should live for one sync run.
pub fn estimate_cost_cached(
    model_name: &str,
    in_tokens: u32,
    out_tokens: u32,
    candidates: &[UnifiedModel],
    cache: &mut std::collections::HashMap<String, Option<usize>>,
) -> Option<f64> {
    let idx = cache.entry(model_name.to_string()).or_insert_with(|| {
        // Reuse estimate_cost's matching logic but return the index instead of the model.
        let mut best_score = 0;
        let mut matched_idx: Option<usize> = None;

        let cleaned_name = model_name
            .to_lowercase()
            .replace("anthropic/", "")
            .replace("openai/", "")
            .replace("google/", "")
            .replace("deepseek/", "")
            .replace("tencent/", "")
            .replace("xiaomi/", "")
            .replace("minimaxai/", "")
            .replace("-thinking", "")
            .replace(".0", "")
            .replace(".", "-");

        let name_pattern = Pattern::parse(&cleaned_name, CaseMatching::Ignore, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);

        for (i, candidate) in candidates.iter().enumerate() {
            let clean_c_id = candidate.id.to_lowercase();

            if clean_c_id == cleaned_name || clean_c_id.starts_with(&cleaned_name) || cleaned_name.starts_with(&clean_c_id) {
                matched_idx = Some(i);
                break;
            }

            let haystack_name = Utf32String::from(clean_c_id.as_str());
            let mut positions = Vec::new();
            if let Some(score) = name_pattern.indices(haystack_name.slice(..), &mut matcher, &mut positions) {
                if score > best_score && score > 60 {
                    best_score = score;
                    matched_idx = Some(i);
                }
            }
        }

        matched_idx
    }).clone();

    let m = idx.and_then(|i| candidates.get(i))?;
    let cost = (in_tokens as f64 / 1_000_000.0) * m.input_price
             + (out_tokens as f64 / 1_000_000.0) * m.output_price;
    if cost > 0.0 { Some(cost) } else { None }
}
