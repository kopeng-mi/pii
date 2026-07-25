use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "pii",
    version,
    about = "pi coding agent session analytics & model explorer"
)]
pub struct Cli {
    /// Show today's sessions
    #[arg(short = 't', long)]
    pub today: bool,

    /// Show past 7 days of sessions
    #[arg(short = 'w', long)]
    pub week: bool,

    /// Show past 30 days of sessions
    #[arg(short = 'm', long)]
    pub month: bool,

    /// Show activity heatmap (180 days)
    #[arg(short = 'H', long)]
    pub heatmap: bool,

    /// Show summary dashboard
    #[arg(short = 's', long)]
    pub summary: bool,

    /// Interactive picker to continue a session (default behavior if no args given)
    #[arg(short = 'c', long)]
    pub continue_session: bool,

    /// Interactive picker to inspect a session's details
    #[arg(short = 'i', long)]
    pub inspect: bool,

    /// When inspecting, show individual calls
    #[arg(long)]
    pub calls: bool,

    /// Filter sessions by model name (FTS search)
    #[arg(short = 'q', long, value_name = "PATTERN")]
    pub query: Option<String>,

    /// Sort column (cost, tokens, calls, time)
    #[arg(long, default_value = "time")]
    pub sort: String,

    /// Scope picker to last N days
    #[arg(short = 'd', long, value_name = "DAYS")]
    pub days: Option<u32>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Model detail card or model picker
    Model {
        /// Model name query
        query: Option<String>,
        /// Force API re-fetch
        #[arg(long)]
        refresh: bool,
    },
    /// Compare models side-by-side
    Compare {
        /// Models to compare
        models: Vec<String>,
        /// Show radar chart
        #[arg(long)]
        spider: bool,
    },
    /// Show TrueSkill rankings from LLM-Stats
    Rankings {
        /// Ranking category
        category: Option<String>,
    },
}
