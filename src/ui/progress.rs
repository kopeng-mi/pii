//! Tiny progress indicator. Single-line, dim, doesn't pollute the terminal.
//!
//! Usage (determinate):
//! ```ignore
//! let pb = Progress::new("Loading models", 10);
//! for i in 0..10 { pb.tick(i); /* work */ }
//! pb.finish();
//! ```
//!
//! Usage (indeterminate, spin-only):
//! ```ignore
//! let pb = Progress::spinner("Fetching from LLM-Stats");
//! /* work */
//! pb.finish();
//! ```
use std::io::{self, Write};
use std::time::{Duration, Instant};

const SPINNER: &[char] = &['⠋', '⠙', '⠸', '⠴', '⠦', '⠇', '⠼', '⠾', '⠿'];

pub struct Progress {
    label: String,
    total: u64,
    current: u64,
    started: Instant,
    finished: bool,
}

impl Progress {
    pub fn new(label: impl Into<String>, total: u64) -> Self {
        let p = Self {
            label: label.into(),
            total,
            current: 0,
            started: Instant::now(),
            finished: false,
        };
        render_line(&p.label, spinner_frame(p.started), p.current, p.total, p.started.elapsed());
        p
    }

    pub fn spinner(label: impl Into<String>) -> Self {
        let p = Self {
            label: label.into(),
            total: 0,
            current: 0,
            started: Instant::now(),
            finished: false,
        };
        render_line(&p.label, spinner_frame(p.started), 0, 0, p.started.elapsed());
        p
    }

    /// Advance progress to `current` (clamped to total).
    pub fn tick(&mut self, current: u64) {
        self.current = current.min(self.total);
        render_line(&self.label, spinner_frame(self.started), self.current, self.total, self.started.elapsed());
    }

    /// Increment by 1.
    #[allow(dead_code)]
    pub fn inc(&mut self) {
        self.tick(self.current + 1);
    }

    pub fn finish(mut self) {
        if self.finished { return; }
        self.finished = true;
        // Show the final state for a fraction of a second isn't necessary —
        // just clear the line so the next view renders cleanly below.
        // \r -> column 0, \x1b[K -> clear to end of line.
        let _ = write!(io::stdout(), "\r\x1b[K");
        let _ = io::stdout().flush();
    }

    pub fn fail(mut self, msg: &str) {
        if self.finished { return; }
        self.finished = true;
        let _ = write!(io::stdout(), "\r\x1b[2m  ✗ \x1b[0m\x1b[1m{}\x1b[0m  \x1b[38;5;196m{}\x1b[0m  \x1b[38;5;246m{}\x1b[0m\n",
            self.label, msg, fmt_dur(self.started.elapsed()));
    }
}

fn spinner_frame(started: Instant) -> char {
    let idx = (started.elapsed().as_millis() / 80) as usize % SPINNER.len();
    SPINNER[idx]
}

fn fmt_dur(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 { format!("{}ms", ms) }
    else if ms < 60_000 { format!("{:.1}s", ms as f64 / 1000.0) }
    else { format!("{}m{:02}s", ms / 60_000, (ms / 1000) % 60) }
}

fn render_line(label: &str, glyph: char, current: u64, total: u64, elapsed: Duration) {
    let (bar, pct) = if total == 0 {
        (String::from("────────────────────"), 0.0_f64)
    } else {
        let bar_width = 20usize;
        let filled = ((current as usize) * bar_width) / (total as usize).max(1);
        (format!("{}{}", "█".repeat(filled), "░".repeat(bar_width - filled)),
         (current as f64) * 100.0 / (total as f64))
    };
    // \r overwrites the previous spinner line.
    // Format: "  ⠋ Label  ████░░░░ 42/100  42.0%  1.2s"
    let _ = write!(
        io::stdout(),
        "\r\x1b[2m  {} \x1b[0m\x1b[1m{}\x1b[0m  \x1b[38;5;43m{}\x1b[0m \x1b[38;5;246m{:>4}/{:<4} {:>5.1}%  {}\x1b[0m\x1b[K",
        glyph, label, bar, current, total, pct, fmt_dur(elapsed)
    );
    let _ = io::stdout().flush();
}

impl Drop for Progress {
    fn drop(&mut self) {
        if !self.finished {
            // Drop without finish(): clear the line so we don't leave a half-rendered
            // spinner behind. \r -> col 0, \x1b[K -> clear to end of line.
            let _ = write!(io::stdout(), "\r\x1b[K");
            let _ = io::stdout().flush();
        }
    }
}