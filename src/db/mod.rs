//! Database ownership, schema migrations and typed repository queries.

pub mod repo;
pub mod schema;
pub mod seed;

use crate::error::{data_dir, AppResult};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub struct Database {
    pub conn: Connection,
    pub path: PathBuf,
}

impl Database {
    /// Open (or create) the database at the platform data directory and run
    /// migrations + first-run seeding.
    pub fn open_default() -> AppResult<Self> {
        let dir = data_dir()?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("lockin.db");
        let mut db = Self::open(&path)?;
        db.migrate()?;
        db.seed_if_empty()?;
        Ok(db)
    }

    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// In-memory database for tests and previews.
    pub fn open_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Self {
            conn,
            path: PathBuf::from(":memory:"),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn migrate(&mut self) -> AppResult<()> {
        schema::migrate(&mut self.conn)
    }

    pub fn seed_if_empty(&mut self) -> AppResult<()> {
        seed::seed_if_empty(&mut self.conn)
    }

    pub fn checkpoint(&self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
}
