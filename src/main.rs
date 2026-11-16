//! Lockin — local-first study and exam operating system.
//!
//! Architecture:
//! - `models`     typed domain models shared by every layer
//! - `db`         SQLite connection, migrations, seed data, repository queries
//! - `services`   business logic (plans, dashboard, revision, analytics, …)
//! - `scheduling` pure scheduling rules (template expansion, overlap, replanning)
//! - `timer`      monotonic focus timer + crash-safe checkpointing
//! - `import_export` versioned "Lockin Plan Format" + full backup
//! - `ui`         thin bridge between Slint callbacks and services

mod app;
mod db;
mod error;
mod import_export;
mod models;
mod scheduling;
mod services;
mod timer;
mod ui;

slint::include_modules!();

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("lockin_final_hope=info,warn")),
        )
        .with_target(false)
        .init();

    if let Err(err) = app::run() {
        tracing::error!("fatal: {err}");
        eprintln!("lockin-final-hope: {err}");
        std::process::exit(1);
    }
}
