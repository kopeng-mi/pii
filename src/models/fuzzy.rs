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
        let haystack_id = Utf32String::from(clean_c_id.as_str());
        let mut positions = Vec::new();
        if let Some(score) = id_pattern.indices(haystack_id.slice(..), &mut matcher, &mut positions) {
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
        if let Some(score) = stripped_pattern.indices(haystack_id.slice(..), &mut matcher, &mut positions) {
            if score > best_score && score > 60 {
                best_score = score;
                best_match = Some(candidate.id.clone());
            }
        }
    }

    best_match
}
