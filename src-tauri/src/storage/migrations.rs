//! Versioned schema migrations.
//!
//! Each migration is embedded in the binary, applied inside a transaction, and
//! recorded through `PRAGMA user_version`. Adding a migration means appending a
//! row to `MIGRATIONS` — never editing a released file.

use rusqlite::Connection;

use crate::error::AppResult;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../../migrations/001_init.sql"),
    },
    Migration {
        version: 2,
        name: "citation_page_kind",
        sql: include_str!("../../migrations/002_citation_page_kind.sql"),
    },
    Migration {
        version: 3,
        name: "canvas_files",
        sql: include_str!("../../migrations/003_canvas_files.sql"),
    },
    Migration {
        version: 4,
        name: "canvas_file_folders",
        sql: include_str!("../../migrations/004_canvas_file_folders.sql"),
    },
];

pub const LATEST_VERSION: i64 = 4;

pub fn run(conn: &mut Connection) -> AppResult<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > LATEST_VERSION {
        return Err(crate::error::AppError::Message(format!(
            "数据库版本（{current}）高于此应用支持的版本（{LATEST_VERSION}）。请升级应用后重试。"
        )));
    }
    for migration in MIGRATIONS {
        if migration.version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        // `user_version` cannot take a bound parameter; the value is a compile-time constant.
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
        tracing::info!(version = migration.version, name = migration.name, "migration applied");
    }
    Ok(())
}
