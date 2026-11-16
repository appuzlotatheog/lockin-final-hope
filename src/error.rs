use std::path::PathBuf;

/// Structured application error. Every fallible layer funnels into this so the
/// UI can show an actionable message instead of silently failing.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("file error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Validation(String),

    #[error("{0} not found")]
    NotFound(String),

    #[error("could not determine the application data directory")]
    NoDataDir,

    #[error("{0}")]
    Other(String),
}

impl AppError {
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Location of the SQLite database, e.g. `~/.local/share/lockin-final-hope/lockin.db`
/// on Linux. Resolved through the `directories` crate so Windows/macOS use the
/// platform-correct data directory.
pub fn data_dir() -> AppResult<PathBuf> {
    directories::ProjectDirs::from("dev", "lockin", "lockin-final-hope")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or(AppError::NoDataDir)
}

pub fn db_path() -> AppResult<PathBuf> {
    Ok(data_dir()?.join("lockin.db"))
}
