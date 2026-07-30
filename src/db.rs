use rusqlite::{Connection, Result};
use std::path::PathBuf;

pub fn get_db_path() -> PathBuf {
    // Dev builds keep state in the project root so they never pollute the
    // system-wide dir; release builds use the OS app-data dir.
    #[cfg(debug_assertions)]
    {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push(".pii");
        p.push("pii.db");
        return p;
    }
    #[cfg(not(debug_assertions))]
    {
        // OS-conventional per-user app data dir:
        //   Windows: %LOCALAPPDATA%\pii\pii.db
        //   Linux:   ~/.local/share/pii/pii.db
        //   macOS:   ~/Library/Application Support/pii/pii.db
        // Falls back to CWD if even that isn't resolvable.
        let mut p = dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        p.push("pii");
        p.push("pii.db");
        p
    }
}

/// Read a string setting; returns `default` if the key is absent.
pub fn get_setting(conn: &Connection, key: &str, default: &str) -> Result<String> {
    let v: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok();
    Ok(v.unwrap_or_else(|| default.to_string()))
}

pub fn init_db(conn: &Connection) -> Result<()> {
    // Performance PRAGMAs — WAL + tuned cache + mmap
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA cache_size = -16000;
        PRAGMA temp_store = MEMORY;
        PRAGMA mmap_size = 268435456;
        PRAGMA busy_timeout = 5000;

        CREATE TABLE IF NOT EXISTS sessions (
            id          TEXT PRIMARY KEY,
            project     TEXT NOT NULL,
            file_path   TEXT NOT NULL UNIQUE,
            file_size   INTEGER NOT NULL,
            date        TEXT NOT NULL,
            time        TEXT NOT NULL,
            prompt      TEXT DEFAULT '',
            models      TEXT DEFAULT '',
            total_calls INTEGER DEFAULT 0,
            total_tokens INTEGER DEFAULT 0,
            total_cost  REAL DEFAULT 0.0,
            errors      INTEGER DEFAULT 0,
            last_model  TEXT DEFAULT '',
            ai_name     TEXT DEFAULT '',
            parent_session TEXT DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS calls (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id    TEXT NOT NULL REFERENCES sessions(id),
            provider      TEXT NOT NULL DEFAULT '',
            model         TEXT NOT NULL,
            input_tokens  INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            tokens        INTEGER DEFAULT 0,
            cost          REAL DEFAULT 0.0,
            is_error      BOOLEAN DEFAULT 0
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS sessions_fts USING fts5(
            project, prompt, models, last_model,
            content=sessions, content_rowid=rowid
        );

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
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
            INSERT INTO sessions_fts(rowid, project, prompt, models, last_model)
            VALUES (new.rowid, new.project, new.prompt, new.models, new.last_model);
        END;
        CREATE TRIGGER IF NOT EXISTS sessions_ad AFTER DELETE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, project, prompt, models, last_model)
            VALUES('delete', old.rowid, old.project, old.prompt, old.models, old.last_model);
        END;
        CREATE TRIGGER IF NOT EXISTS sessions_au AFTER UPDATE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, project, prompt, models, last_model)
            VALUES('delete', old.rowid, old.project, old.prompt, old.models, old.last_model);
            INSERT INTO sessions_fts(rowid, project, prompt, models, last_model)
            VALUES (new.rowid, new.project, new.prompt, new.models, new.last_model);
        END;

        CREATE INDEX IF NOT EXISTS idx_sessions_date ON sessions(date);
        CREATE INDEX IF NOT EXISTS idx_calls_session ON calls(session_id);
        CREATE INDEX IF NOT EXISTS idx_scores_model ON scores(model_id);
        ",
    )?;
    migrate_sessions_fts(conn)?;
    migrate_sessions_ai_name(conn)?;
    migrate_sessions_parent(conn)?;
    migrate_calls_provider(conn)?;
    Ok(())
}

/// Migrate sessions_fts from older schema (project,prompt,models) to current
/// (project,prompt,models,last_model). FTS5 content tables can't ALTER columns,
/// so we rebuild from the source table when the column count is wrong.
fn migrate_sessions_fts(conn: &Connection) -> Result<()> {
    let n_cols: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sessions_fts')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if n_cols >= 4 {
        return Ok(());
    }
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap_or(0);
    let mut pb = crate::ui::progress::Progress::new(
        "Migrating FTS index (adding last_model)",
        (total as u64).max(1),
    );
    eprintln!("  [migrate] upgrading sessions_fts schema (adding last_model)");
    conn.execute_batch(
        "
        DROP TRIGGER IF EXISTS sessions_ai;
        DROP TRIGGER IF EXISTS sessions_ad;
        DROP TRIGGER IF EXISTS sessions_au;
        DROP TABLE IF EXISTS sessions_fts;
        CREATE VIRTUAL TABLE sessions_fts USING fts5(
            project, prompt, models, last_model,
            content=sessions, content_rowid=rowid
        );
        CREATE TRIGGER sessions_ai AFTER INSERT ON sessions BEGIN
            INSERT INTO sessions_fts(rowid, project, prompt, models, last_model)
            VALUES (new.rowid, new.project, new.prompt, new.models, new.last_model);
        END;
        CREATE TRIGGER sessions_ad AFTER DELETE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, project, prompt, models, last_model)
            VALUES('delete', old.rowid, old.project, old.prompt, old.models, old.last_model);
        END;
        CREATE TRIGGER sessions_au AFTER UPDATE ON sessions BEGIN
            INSERT INTO sessions_fts(sessions_fts, rowid, project, prompt, models, last_model)
            VALUES('delete', old.rowid, old.project, old.prompt, old.models, old.last_model);
            INSERT INTO sessions_fts(rowid, project, prompt, models, last_model)
            VALUES (new.rowid, new.project, new.prompt, new.models, new.last_model);
        END;
        INSERT INTO sessions_fts(rowid, project, prompt, models, last_model)
            SELECT rowid, project, prompt, models, last_model FROM sessions;
        ",
    )?;
    pb.tick((total as u64).max(1));
    pb.finish();
    Ok(())
}

/// Add `ai_name` column to existing `sessions` table if upgrading.
fn migrate_sessions_ai_name(conn: &Connection) -> Result<()> {
    let has_col: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'ai_name')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !has_col {
        eprintln!("  [migrate] adding sessions.ai_name column");
        conn.execute_batch("ALTER TABLE sessions ADD COLUMN ai_name TEXT DEFAULT ''")?;
    }
    Ok(())
}

/// Add `provider` column to existing `calls` table if upgrading.
fn migrate_calls_provider(conn: &Connection) -> Result<()> {
    let has: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('calls') WHERE name = 'provider')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !has {
        eprintln!("  [migrate] adding calls.provider column");
        conn.execute_batch(
            "ALTER TABLE calls ADD COLUMN provider TEXT NOT NULL DEFAULT ''",
        )?;
    }
    Ok(())
}

/// Add `parent_session` column to existing `sessions` table if upgrading.
fn migrate_sessions_parent(conn: &Connection) -> Result<()> {
    let has_col: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'parent_session')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !has_col {
        eprintln!("  [migrate] adding sessions.parent_session column");
        conn.execute_batch("ALTER TABLE sessions ADD COLUMN parent_session TEXT DEFAULT ''")?;
    }
    Ok(())
}
