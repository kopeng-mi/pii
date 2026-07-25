use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine, Points},
        Block, Borders, Paragraph,
    },
    Terminal,
};
use std::f64::consts::PI;
use std::io::{self, stdout};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};

pub struct SpiderData {
    pub name: String,
    pub values: Vec<f64>, // 0.0..=1.0 normalized scores
    pub color: Color,
}

pub fn run_spider_chart(models: Vec<SpiderData>, categories: Vec<String>) -> io::Result<()> {
    if models.is_empty() || categories.is_empty() {
        return Ok(());
    }
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|f| {
            let size = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(10), Constraint::Length(3)].as_ref())
                .split(size);

            let chart_area = chunks[0];
            let legend_area = chunks[1];

            let canvas = Canvas::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(
                            " Model Comparison · {} models · {} benchmarks ",
                            models.len(),
                            categories.len()
                        ))
                        .title_style(Style::default().fg(Color::Cyan).add_modifier(ratatui::style::Modifier::BOLD)),
                )
                .marker(ratatui::symbols::Marker::Braille)
                .x_bounds([-150.0, 150.0])
                .y_bounds([-150.0, 150.0])
                .paint(|ctx| {
                    let num_axes = categories.len() as f64;

                    // Radial grid rings
                    for r in [25.0_f64, 50.0, 75.0, 100.0] {
                        for i in 0..categories.len() {
                            let a1 = (i as f64) * 2.0 * PI / num_axes - PI / 2.0;
                            let a2 = ((i + 1) as f64) * 2.0 * PI / num_axes - PI / 2.0;
                            ctx.draw(&CanvasLine {
                                x1: a1.cos() * r,
                                y1: a1.sin() * r,
                                x2: a2.cos() * r,
                                y2: a2.sin() * r,
                                color: Color::Rgb(45, 45, 50),
                            });
                        }
                    }

                    // Axes and category labels
                    for i in 0..categories.len() {
                        let angle = (i as f64) * 2.0 * PI / num_axes - PI / 2.0;
                        let x2 = angle.cos() * 105.0;
                        let y2 = angle.sin() * 105.0;

                        // Axis line
                        ctx.draw(&CanvasLine {
                            x1: 0.0,
                            y1: 0.0,
                            x2,
                            y2,
                            color: Color::DarkGray,
                        });

                        // Category labels (outside the ring)
                        let lx = angle.cos() * 130.0;
                        let ly = angle.sin() * 130.0;
                        ctx.print(lx, ly, category_short(&categories[i]));
                    }

                    // Scale labels along the first axis (up)
                    for (r, label) in [(25.0, "25"), (50.0, "50"), (75.0, "75"), (100.0, "100")] {
                        ctx.print(2.0, r, format!("·{}", label));
                    }

                    // Filled polygon area (dotted) — render first so outlines overlay cleanly.
                    for m in &models {
                        let pts = polygon_points(m, num_axes);
                        // dense points for fill
                        let fill_pts = fill_points(&pts, 90);
                        ctx.draw(&Points {
                            coords: &fill_pts,
                            color: dim_color(m.color),
                        });
                    }

                    // Polygon outlines + vertices
                    for m in &models {
                        for i in 0..categories.len() {
                            let next_i = (i + 1) % categories.len();
                            let v1 = m.values.get(i).unwrap_or(&0.0) * 100.0;
                            let v2 = m.values.get(next_i).unwrap_or(&0.0) * 100.0;
                            let a1 = (i as f64) * 2.0 * PI / num_axes - PI / 2.0;
                            let a2 = (next_i as f64) * 2.0 * PI / num_axes - PI / 2.0;
                            ctx.draw(&CanvasLine {
                                x1: a1.cos() * v1,
                                y1: a1.sin() * v1,
                                x2: a2.cos() * v2,
                                y2: a2.sin() * v2,
                                color: m.color,
                            });
                        }

                        // Vertices as visible dots
                        let vertices: Vec<(f64, f64)> = (0..categories.len())
                            .map(|i| {
                                let v = m.values.get(i).unwrap_or(&0.0) * 100.0;
                                let a = (i as f64) * 2.0 * PI / num_axes - PI / 2.0;
                                (a.cos() * v, a.sin() * v)
                            })
                            .collect();
                        ctx.draw(&Points {
                            coords: &vertices,
                            color: m.color,
                        });
                    }
                });

            f.render_widget(canvas, chart_area);

            // Legend: model name + colored swatch + hint
            let mut spans: Vec<Span> = vec![Span::styled(
                "  Press q/Esc to exit  ·  ",
                Style::default().fg(Color::DarkGray),
            )];
            for m in &models {
                spans.push(Span::styled(
                    "■ ".to_string(),
                    Style::default().fg(m.color).add_modifier(ratatui::style::Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("{}  ", m.name),
                    Style::default().fg(m.color),
                ));
            }
            let legend = Paragraph::new(Line::from(spans))
                .block(Block::default().borders(Borders::TOP));
            f.render_widget(legend, legend_area);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Build the (x, y) vertices of a model's polygon.
fn polygon_points(m: &SpiderData, num_axes: f64) -> Vec<(f64, f64)> {
    (0..m.values.len())
        .map(|i| {
            let v = m.values.get(i).unwrap_or(&0.0) * 100.0;
            let a = (i as f64) * 2.0 * PI / num_axes - PI / 2.0;
            (a.cos() * v, a.sin() * v)
        })
        .collect()
}

/// Approximate a polygon fill by scanning horizontal scanlines inside the bounding box
/// and emitting a point at every few pixels. Cheap, looks like a shaded area.
fn fill_points(verts: &[(f64, f64)], density: usize) -> Vec<(f64, f64)> {
    if verts.is_empty() {
        return vec![];
    }
    let mut min_y = verts[0].1;
    let mut max_y = verts[0].1;
    let mut min_x = verts[0].0;
    let mut max_x = verts[0].0;
    for &(x, y) in verts {
        if y < min_y {
            min_y = y;
        }
        if y > max_y {
            max_y = y;
        }
        if x < min_x {
            min_x = x;
        }
        if x > max_x {
            max_x = x;
        }
    }

    let step = ((max_x - min_x).abs().max((max_y - min_y).abs())) / density as f64;
    if step <= 0.0 {
        return vec![];
    }

    let mut pts = Vec::new();
    let mut y = min_y;
    while y <= max_y {
        let intersections = scanline_intersections(verts, y);
        if intersections.len() >= 2 {
            // pair them up and fill between
            let mut xs = intersections;
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < xs.len() {
                let mut x = xs[i];
                while x <= xs[i + 1] {
                    pts.push((x, y));
                    x += step;
                }
                i += 2;
            }
        }
        y += step;
    }
    pts
}

/// Compute intersections of a horizontal line at y with the polygon edges.
fn scanline_intersections(verts: &[(f64, f64)], y: f64) -> Vec<f64> {
    let mut xs = Vec::new();
    let n = verts.len();
    for i in 0..n {
        let (x1, y1) = verts[i];
        let (x2, y2) = verts[(i + 1) % n];
        // Skip horizontal edges.
        if (y2 - y1).abs() < f64::EPSILON {
            continue;
        }
        if (y >= y1 && y < y2) || (y >= y2 && y < y1) {
            let t = (y - y1) / (y2 - y1);
            xs.push(x1 + t * (x2 - x1));
        }
    }
    xs
}

fn category_short(s: &str) -> String {
    if s.chars().count() <= 9 {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars().take(8) {
        out.push(c);
    }
    out.push('…');
    out
}

fn dim_color(c: Color) -> Color {
    match c {
        Color::Cyan => Color::Rgb(0, 100, 110),
        Color::Yellow => Color::Rgb(100, 100, 0),
        Color::Magenta => Color::Rgb(110, 0, 110),
        Color::Green => Color::Rgb(0, 110, 0),
        Color::Red => Color::Rgb(110, 0, 0),
        Color::White => Color::Rgb(120, 120, 120),
        other => other,
    }
}

/// Allow callers to build a chart area with custom title; kept for future API.
#[allow(dead_code)]
fn _chart_area(r: Rect) -> Rect {
    r
}
