use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedModel {
    pub id: String,
    pub name: String,
    pub creator: String,
    pub release_date: String,
    pub context_window: Option<u32>,
    pub param_count: Option<u64>,
    pub input_price: f64,
    pub output_price: f64,
    pub speed_tok_s: Option<f64>,
    pub ttft_s: Option<f64>,
    pub open_weight: bool,
    pub source: String,
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    pub model_id: String,
    pub benchmark: String,
    pub score: f64,
    pub max_score: Option<f64>,
    pub category: String,
}
