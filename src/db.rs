use rusqlite::{Connection, Result};
use std::path::PathBuf;

pub fn get_db_path() -> PathBuf {
    PathBuf::from("pii.db")
}

pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sessions (
            id          TEXT PRIMARY KEY,
            project     TEXT NOT NULL,
            file_path   TEXT NOT NULL UNIQUE,
            file_size   INTEGER NOT NULL,
            date        TEXT NOT NULL,
            time        TEXT NOT NULL,
            prompt      TEXT DEFAULT '',
            total_calls INTEGER DEFAULT 0,
            total_tokens INTEGER DEFAULT 0,
            total_cost  REAL DEFAULT 0.0,
            errors      INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS calls (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id    TEXT NOT NULL REFERENCES sessions(id),
            model         TEXT NOT NULL,
            input_tokens  INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            tokens        INTEGER DEFAULT 0,
            cost          REAL DEFAULT 0.0,
            is_error      BOOLEAN DEFAULT 0
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS sessions_fts USING fts5(
            project, prompt, models,
            content=sessions, content_rowid=rowid
        );

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS models (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            creator       TEXT DEFAULT '',
            release_date  TEXT DEFAULT '',
            context_window INTEGER,
            param_count   INTEGER,
            input_price   REAL DEFAULT 0.0,
            output_price  REAL DEFAULT 0.0,
            speed_tok_s   REAL,
            ttft_s        REAL,
            open_weight   BOOLEAN DEFAULT 0,
            source        TEXT DEFAULT '',
            raw_json      TEXT DEFAULT '{}'
        );

        CREATE TABLE IF NOT EXISTS scores (
            model_id      TEXT NOT NULL REFERENCES models(id),
            benchmark     TEXT NOT NULL,
            score         REAL NOT NULL,
            max_score     REAL,
            category      TEXT DEFAULT '',
            PRIMARY KEY (model_id, benchmark)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS models_fts USING fts5(
            name, creator, id,
            content=models, content_rowid=rowid
        );

        -- Create triggers to keep models_fts table in sync
        CREATE TRIGGER IF NOT EXISTS models_ai AFTER INSERT ON models BEGIN
            INSERT INTO models_fts(rowid, name, creator, id)
            VALUES (new.rowid, new.name, new.creator, new.id);
        END;
        CREATE TRIGGER IF NOT EXISTS models_ad AFTER DELETE ON models BEGIN
            INSERT INTO models_fts(models_fts, rowid, name, creator, id)
            VALUES('delete', old.rowid, old.name, old.creator, old.id);
        END;
        CREATE TRIGGER IF NOT EXISTS models_au AFTER UPDATE ON models BEGIN
            INSERT INTO models_fts(models_fts, rowid, name, creator, id)
            VALUES('delete', old.rowid, old.name, old.creator, old.id);
            INSERT INTO models_fts(rowid, name, creator, id)
            VALUES (new.rowid, new.name, new.creator, new.id);
        END;

        -- Create triggers to keep FTS table in sync
        CREATE TRIGGER IF NOT EXISTS sessions_ai AFTER INSERT ON sessions BEGIN
            INSERT INTO sessions_fts(rowid, project, prompt, models)
            VALUES (new.rowid, new.project, new.prompt, '');
        END;
        CREATE TRIGGER IF NOT EXISTS sessions_ad AFTER DELETE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, project, prompt, models)
            VALUES('delete', old.rowid, old.project, old.prompt, '');
        END;
        CREATE TRIGGER IF NOT EXISTS sessions_au AFTER UPDATE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, project, prompt, models)
            VALUES('delete', old.rowid, old.project, old.prompt, '');
            INSERT INTO sessions_fts(rowid, project, prompt, models)
            VALUES (new.rowid, new.project, new.prompt, '');
        END;
        ",
    )?;
    Ok(())
}
