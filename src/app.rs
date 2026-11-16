//! Application bootstrap: database, settings, tracing, UI.

use crate::error::AppResult;

pub fn run() -> AppResult<()> {
    let db = crate::db::Database::open_default()?;
    tracing::info!("database at {}", db.path.display());
    let settings = crate::db::repo::load_settings(&db.conn)?;
    let plan_id = crate::services::plan::current_plan_id(&db.conn, &settings)?;
    tracing::info!("active plan: {plan_id}");
    crate::ui::run(db, settings, plan_id)
}
