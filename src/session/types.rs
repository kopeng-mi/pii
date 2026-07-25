pub struct SessionRow {
    pub id: String,
    pub project: String,
    pub file_path: String,
    pub file_size: u64,
    pub date: String,
    pub time: String,
    pub prompt: String,
    pub total_calls: u32,
    pub total_tokens: u32,
    pub total_cost: f64,
    pub errors: u32,
}

pub struct CallRow {
    pub session_id: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub tokens: u32,
    pub cost: f64,
    pub is_error: bool,
}
