use rusqlite::Connection;
fn main() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("
        CREATE TABLE sessions(id TEXT PRIMARY KEY, project TEXT, prompt TEXT);
        CREATE VIRTUAL TABLE sessions_fts USING fts5(project, prompt, models, content=sessions, content_rowid=rowid);
        INSERT INTO sessions (rowid, id, project, prompt) VALUES (1, 's1', 'proj1', 'hello world');
        INSERT INTO sessions_fts(rowid, project, prompt, models) VALUES (1, 'proj1', 'hello world', 'claude gpt');
    ").unwrap();
    let count: i32 = conn.query_row("SELECT count(*) FROM sessions_fts WHERE sessions_fts MATCH 'claude'", [], |r| r.get(0)).unwrap();
    println!("Count: {}", count);
}
