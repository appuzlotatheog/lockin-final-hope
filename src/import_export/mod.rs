//! Lockin Plan Format (versioned, human-editable JSON) and full backup/restore.
//! Nothing imported is ever executed, fetched or trusted to touch history.

pub mod apply;
pub mod backup;
pub mod format;
pub mod validate;

pub const MAX_IMPORT_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_BACKUP_BYTES: usize = 25 * 1024 * 1024;
