use crate::ui::picker::Selection;
use crate::ui::table::truncate;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

// ── Node ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Node {
    id: String,
    file_path: String,
    name: String,
    cost_str: String,
    time_str: String,
    date: String,
    time: String,
    parent: String,
    project: String,
}

// ── Folders (synthetic project headers) ──────────────────────────────────

const FOLDER_PREFIX: &str = "__folder__";

fn folder_id(project: &str) -> String {
    format!("{}{}", FOLDER_PREFIX, project)
}

fn is_folder_id(s: &str) -> bool {
    s.starts_with(FOLDER_PREFIX)
}

fn folder_project(id: &str) -> &str {
    id.strip_prefix(FOLDER_PREFIX).unwrap_or("")
}

// ── Public entry ─────────────────────────────────────────────────────────

pub fn run_tree_picker(
    conn: &Connection,
    days: Option<u32>,
    prompt: &str,
) -> rusqlite::Result<Option<Selection>> {
    let nodes = load_nodes(conn, days)?;
    if nodes.is_empty() {
        println!("No sessions found.");
        return Ok(None);
    }
    run_inner(nodes, prompt)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
}

/// Debug: dump tree structure to stdout (non-interactive).
pub fn dump_tree(conn: &Connection) -> rusqlite::Result<()> {
    let nodes = load_nodes(conn, None)?;
    let tree = Tree::build(&nodes);
    let id_to_node: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for r in &tree.roots {
        dump_walk(r, 0, &tree, &id_to_node);
    }
    Ok(())
}

// ── Tree structure ───────────────────────────────────────────────────────

struct Tree {
    roots: Vec<String>,
    children: HashMap<String, Vec<String>>,  // parent_path → child ids
    id_to_path: HashMap<String, String>,     // id → normalized file_path
}

impl Tree {
    fn build(nodes: &[Node]) -> Self {
        let id_to_path: HashMap<String, String> = nodes
            .iter()
            .map(|n| (n.id.clone(), norm(&n.file_path)))
            .collect();

        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        for n in nodes {
            if !n.parent.is_empty() {
                children
                    .entry(norm(&n.parent))
                    .or_default()
                    .push(n.id.clone());
            }
        }
        // Self-loop protection
        for n in nodes {
            if !n.parent.is_empty() && norm(&n.parent) == norm(&n.file_path) {
                if let Some(kids) = children.get_mut(&norm(&n.file_path)) {
                    kids.retain(|c| c != &n.id);
                }
            }
        }

        let known_paths: HashSet<&str> = id_to_path.values().map(|s| s.as_str()).collect();

        // Session roots: no parent or parent not in our set
        let mut session_roots: Vec<String> = Vec::new();
        for n in nodes {
            if n.parent.is_empty() {
                session_roots.push(n.id.clone());
            } else {
                let pnorm = norm(&n.parent);
                if pnorm == norm(&n.file_path) || !known_paths.contains(pnorm.as_str()) {
                    session_roots.push(n.id.clone());
                }
            }
        }

        // Group session roots by project, get per-project latest timestamp
        let id_to_node: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut proj_roots: HashMap<&str, Vec<String>> = HashMap::new();
        let mut proj_latest: HashMap<&str, String> = HashMap::new();  // project → "date time"
        for sr in &session_roots {
            if let Some(n) = id_to_node.get(sr.as_str()) {
                let ts = format!("{} {}", n.date, n.time);
                let prev = proj_latest.entry(&n.project).or_default();
                if ts > *prev {
                    *prev = ts;
                }
                proj_roots.entry(&n.project).or_default().push(sr.clone());
            }
        }

        // Sort projects by most-recent activity
        let mut projects_sorted: Vec<&str> = proj_roots.keys().copied().collect();
        projects_sorted.sort_by(|a, b| {
            let ta = proj_latest.get(a).map(|s| s.as_str()).unwrap_or("");
            let tb = proj_latest.get(b).map(|s| s.as_str()).unwrap_or("");
            tb.cmp(ta)
        });

        // Build roots list: project folders first, then their session roots as children
        let mut roots: Vec<String> = Vec::new();
        for proj in &projects_sorted {
            let fid = folder_id(proj);
            roots.push(fid.clone());
            // Sort session roots within project by date DESC
            let mut srs = proj_roots.get(proj).cloned().unwrap_or_default();
            srs.sort_by(|a, b| {
                let na = id_to_node.get(a.as_str());
                let nb = id_to_node.get(b.as_str());
                match (na, nb) {
                    (Some(x), Some(y)) => y.date.cmp(&x.date).then(y.time.cmp(&x.time)),
                    _ => std::cmp::Ordering::Equal,
                }
            });
            children.entry(fid.clone()).or_insert(srs);
        }

        Tree { roots, children, id_to_path }
    }

    fn has_children(&self, id: &str) -> bool {
        if let Some(p) = self.id_to_path.get(id) {
            self.children.get(p).map_or(false, |v| !v.is_empty())
        } else {
            // Folders are keyed by folder_id, not by path
            self.children.get(id).map_or(false, |v| !v.is_empty())
        }
    }

    fn child_ids(&self, id: &str) -> &[String] {
        if let Some(p) = self.id_to_path.get(id) {
            self.children.get(p).map_or(&[], |v| v.as_slice())
        } else {
            self.children.get(id).map_or(&[], |v| v.as_slice())
        }
    }
}

// ── Flat visible list (DFS walk of expanded nodes) ───────────────────────

struct FlatRow {
    id: String,
    depth: usize,
    is_last: bool,
    ancestor_last: Vec<bool>,
    is_folder: bool,
}

fn build_visible(tree: &Tree, expanded: &HashSet<String>) -> Vec<FlatRow> {
    let mut out = Vec::new();
    let root_count = tree.roots.len();
    for (ri, root) in tree.roots.iter().enumerate() {
        let is_last = ri + 1 == root_count;
        walk_flat(root, 0, is_last, &[], true, tree, expanded, &mut out);
    }
    out
}

fn walk_flat(
    id: &str,
    depth: usize,
    is_last: bool,
    ancestor_last: &[bool],
    _parent_is_folder: bool,
    tree: &Tree,
    expanded: &HashSet<String>,
    out: &mut Vec<FlatRow>,
) {
    // A root node inherits folder status from parent. Non-root nodes
    // (sessions below the project level) are never folders.
    let is_folder = depth == 0 && is_folder_id(id);
    out.push(FlatRow {
        id: id.to_string(),
        depth,
        is_last,
        ancestor_last: ancestor_last.to_vec(),
        is_folder,
    });
    if !expanded.contains(id) {
        return;
    }
    let kids = tree.child_ids(id);
    let klen = kids.len();
    let mut child_ancestor = ancestor_last.to_vec();
    child_ancestor.push(is_last);
    for (ki, kid) in kids.iter().enumerate() {
        let kid_last = ki + 1 == klen;
        walk_flat(kid, depth + 1, kid_last, &child_ancestor, false, tree, expanded, out);
    }
}

// ── Interactive picker loop ──────────────────────────────────────────────

fn run_inner(nodes: Vec<Node>, _prompt: &str) -> io::Result<Option<Selection>> {
    let tree = Tree::build(&nodes);
    let id_to_node: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // Auto-expand: most-recent folder + its first root's immediate children
    let mut expanded: HashSet<String> = HashSet::new();
    if let Some(top) = tree.roots.first() {
        expanded.insert(top.clone());
        if let Some(first_kid) = tree.child_ids(top).first() {
            expanded.insert(first_kid.clone());
            for grandkid in tree.child_ids(first_kid) {
                expanded.insert(grandkid.clone());
            }
        }
    }

    let mut stdout = io::stdout();
    let _guard = TermGuard::enter(&mut stdout)?;
    let (term_w, term_h) = terminal::size().unwrap_or((120, 30));
    // Reserve: 1 header + max_rows items + 1 hint = max_rows + 2
    let max_rows = ((term_h as usize) * 2 / 5).max(5).min(40);
    let lines = max_rows + 2;

    // Reserve vertical space (inline, like picker.rs)
    for _ in 0..lines {
        stdout.write_all(b"\r\n")?;
    }
    stdout.flush()?;
    queue!(stdout, cursor::MoveUp(lines as u16))?;
    stdout.flush()?;
    let (_, start_row) = cursor::position().unwrap_or((0, 0));

    let mut selected: usize = 0;

    loop {
        let mut visible = build_visible(&tree, &expanded);

        // Accordion-on-nav: if the current selection is a collapsed folder,
        // auto-expand it and close all other folders.
        if let Some(row) = visible.get(selected) {
            if row.is_folder && !expanded.contains(&row.id) {
                let id = row.id.clone();
                for fid in &tree.roots {
                    if fid != &id {
                        expanded.remove(fid);
                    }
                }
                expanded.insert(id);
                visible = build_visible(&tree, &expanded);
            }
        }
        if selected >= visible.len() {
            selected = visible.len().saturating_sub(1);
        }

        // Viewport window
        let vis_start = if selected >= max_rows {
            selected + 1 - max_rows
        } else {
            0
        };
        let vis_end = visible.len().min(vis_start + max_rows);
        let rendered = vis_end - vis_start;

        // Draw
        queue!(stdout, cursor::MoveTo(0, start_row))?;

        // Header
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::AnsiValue(43)),
            Print("  "),
            Print("\x1b[1mSessions (tree)\x1b[0m"),
            ResetColor,
            Print("  "),
            SetForegroundColor(Color::AnsiValue(242)),
            Print(format!(
                "{}/{} · ↑↓ nav · ←→ fold · enter select · esc quit",
                visible.len(), nodes.len()
            )),
            ResetColor,
            Print("\r\n"),
        )?;

        // Rows
        for vi in vis_start..vis_end {
            queue!(stdout, Clear(ClearType::CurrentLine))?;
            let row = &visible[vi];
            let is_sel = vi == selected;

            // Marker
            let marker = if is_sel { "▸" } else { " " };
            queue!(
                stdout,
                SetForegroundColor(Color::AnsiValue(if is_sel { 43 } else { 246 })),
                Print(format!(" {} ", marker)),
                ResetColor,
            )?;

            if row.is_folder {
                // Project folder row: bold name with chevron, no cost/time
                let proj = folder_project(&row.id);
                let has_kids = tree.has_children(&row.id);
                let chevron = if !has_kids {
                    " "
                } else if expanded.contains(&row.id) {
                    "▾"
                } else {
                    "▸"
                };
                let name_w = (term_w as usize).saturating_sub(8).max(10);
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(if is_sel { 220 } else { 43 })),
                    Print("\x1b[1m"),
                    Print(format!("{} ", chevron)),
                    Print(truncate(proj, name_w)),
                    Print("\x1b[0m"),
                    ResetColor,
                    Print("\r\n"),
                )?;
                continue;
            }

            // Session row
            let n = match id_to_node.get(row.id.as_str()) {
                Some(n) => n,
                None => continue,
            };

            // Tree connectors
            if row.depth > 0 {
                let mut prefix = String::new();
                for d in 1..row.depth {
                    if d < row.ancestor_last.len() && row.ancestor_last[d] {
                        prefix.push_str("   ");
                    } else {
                        prefix.push_str(" │ ");
                    }
                }
                if row.is_last {
                    prefix.push_str(" └─");
                } else {
                    prefix.push_str(" ├─");
                }
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(237)),
                    Print(prefix),
                    ResetColor,
                )?;
            }

            // Chevron — only if node has children
            let has_kids = tree.has_children(&row.id);
            if has_kids {
                let chevron = if expanded.contains(&row.id) { "▾" } else { "▸" };
                queue!(
                    stdout,
                    SetForegroundColor(Color::AnsiValue(43)),
                    Print(format!("{} ", chevron)),
                    ResetColor,
                )?;
            } else {
                // Leaf: no chevron, push an extra space so name alignment
                // within the same depth stays consistent.
                queue!(stdout, Print("  "))?;
            }

            // Name — compute available width without chevron if leaf
            let indent_w = if row.depth > 0 { row.depth * 3 + 1 } else { 0 };
            let chevron_w: usize = if has_kids { 2 } else { 2 }; // both slots take 2 chars
            let fixed = 4 + indent_w + chevron_w + 2 + 6 + 2 + 5;
            let name_w = (term_w as usize).saturating_sub(fixed).max(10);
            let name = truncate(&n.name, name_w);
            let name_color = if is_sel { 250u8 } else { 246u8 };
            let cost_color = if n.cost_str == "--" { 242u8 } else { 220u8 };

            queue!(
                stdout,
                SetForegroundColor(Color::AnsiValue(name_color)),
                Print(format!("{:<w$}", name, w = name_w)),
                ResetColor,
                Print("  "),
                SetForegroundColor(Color::AnsiValue(cost_color)),
                Print(format!("{:>6}", n.cost_str)),
                ResetColor,
                Print("  "),
                SetForegroundColor(Color::AnsiValue(242)),
                Print(format!("{:>5}", n.time_str)),
                ResetColor,
                Print("\r\n"),
            )?;
        }

        // Blank remaining rows
        for _ in rendered..max_rows {
            queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;
        }

        // Hint
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::AnsiValue(242)),
            Print("  · folders"),
            ResetColor,
            Print("\r\n"),
        )?;
        stdout.flush()?;

        // Input
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                cleanup(&mut stdout, start_row, lines)?;
                return Ok(None);
            }
            KeyCode::Enter => {
                if let Some(row) = visible.get(selected) {
                    let id = row.id.clone();
                    cleanup(&mut stdout, start_row, lines)?;
                    let n = id_to_node[id.as_str()];
                    return Ok(Some(Selection {
                        id: n.id.clone(),
                        file_path: n.file_path.clone(),
                    }));
                }
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if selected + 1 < visible.len() {
                    selected += 1;
                }
            }
            KeyCode::Left => {
                if let Some(row) = visible.get(selected) {
                    let id = row.id.clone();
                    if expanded.remove(&id) {
                        // collapse — nothing else
                    } else if row.is_folder {
                        // already collapsed, nothing to do
                    } else {
                        // jump to parent
                        if let Some(n) = id_to_node.get(id.as_str()) {
                            if !n.parent.is_empty() {
                                let pnorm = norm(&n.parent);
                                if let Some(pid) = tree.id_to_path.iter().find(|(_, p)| p.as_str() == pnorm).map(|(id, _)| id.clone()) {
                                    if let Some(pos) = visible.iter().position(|r| r.id == pid) {
                                        selected = pos;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            KeyCode::Right => {
                if let Some(row) = visible.get(selected) {
                    let id = row.id.clone();
                    if tree.has_children(&id) && !expanded.contains(&id) {
                        if row.is_folder {
                            // Accordion: close all other folders, open this one
                            let folder_ids: Vec<String> = tree.roots.iter().filter(|r| *r != &id).cloned().collect();
                            for fid in &folder_ids {
                                expanded.remove(fid);
                            }
                        }
                        expanded.insert(id);
                    } else if expanded.contains(&id) && !row.is_folder {
                        // move into first child
                        if selected + 1 < visible.len() {
                            selected += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// ── DB loading ───────────────────────────────────────────────────────────

fn load_nodes(conn: &Connection, days: Option<u32>) -> rusqlite::Result<Vec<Node>> {
    let sql = "SELECT id, file_path, project, date, time, prompt, total_cost, ai_name, parent_session
         FROM sessions
         WHERE (?1 IS NULL OR date >= date('now', '-' || ?1 || ' days'))
         ORDER BY date DESC, time DESC";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([days], |row| {
        let id: String = row.get(0)?;
        let file_path: String = row.get(1)?;
        let project: String = row.get(2)?;
        let date: String = row.get(3)?;
        let time: String = row.get(4)?;
        let first_prompt: String = row.get(5)?;
        let total_cost: f64 = row.get(6)?;
        let ai_name: String = row.get::<_, Option<String>>(7)?.unwrap_or_default();
        let parent_session: String = row.get::<_, Option<String>>(8)?.unwrap_or_default();
        let name = if !ai_name.is_empty() { ai_name } else { first_prompt };
        let cost_str = if total_cost > 0.0 {
            format!("${:.2}", total_cost)
        } else {
            "--".into()
        };
        Ok(Node {
            id,
            file_path,
            name: name.replace(['\n', '\r'], " "),
            cost_str,
            time_str: relative_time(&date, &time),
            date,
            time,
            parent: parent_session,
            project,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn norm(p: &str) -> String {
    p.replace('\\', "/")
}

fn relative_time(date: &str, _time: &str) -> String {
    let now = chrono::Local::now().date_naive();
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return "--".into();
    }
    let y: i32 = parts[0].parse().unwrap_or(0);
    let m: u32 = parts[1].parse().unwrap_or(1);
    let d: u32 = parts[2].parse().unwrap_or(1);
    match chrono::NaiveDate::from_ymd_opt(y, m, d) {
        Some(sd) => {
            let days = (now - sd).num_days().max(0);
            if days == 0 {
                "today".into()
            } else if days < 30 {
                format!("{}d", days)
            } else {
                format!("{}mo", days / 30)
            }
        }
        None => "--".into(),
    }
}

struct TermGuard;
impl TermGuard {
    fn enter(stdout: &mut impl Write) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        queue!(stdout, cursor::Hide)?;
        stdout.flush()?;
        Ok(Self)
    }
}
impl Drop for TermGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = queue!(stdout, ResetColor, cursor::Show);
        let _ = stdout.flush();
        let _ = terminal::disable_raw_mode();
    }
}

fn cleanup(stdout: &mut impl Write, start_row: u16, lines: usize) -> io::Result<()> {
    queue!(stdout, cursor::MoveTo(0, start_row))?;
    for _ in 0..lines {
        queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;
    }
    queue!(stdout, cursor::MoveTo(0, start_row))?;
    stdout.flush()?;
    Ok(())
}

fn dump_walk(id: &str, depth: usize, tree: &Tree, id_to_node: &HashMap<&str, &Node>) {
    let indent = "  ".repeat(depth);
    if is_folder_id(id) {
        let proj = folder_project(id);
        let chevron = if tree.has_children(id) { "▼" } else { "·" };
        println!("{}{} folder [{}]", indent, chevron, proj);
        for kid in tree.child_ids(id) {
            dump_walk(kid, depth + 1, tree, id_to_node);
        }
        return;
    }
    let n = match id_to_node.get(id) {
        Some(n) => n,
        None => return,
    };
    let chevron = if tree.has_children(id) { "▼" } else { "·" };
    println!("{}{} [{}] {}", indent, chevron, &n.id[..8.min(n.id.len())], truncate(&n.name, 60));
    for kid in tree.child_ids(id) {
        dump_walk(kid, depth + 1, tree, id_to_node);
    }
}
