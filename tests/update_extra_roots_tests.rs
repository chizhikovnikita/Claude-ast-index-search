//! Regression tests for `update_directory_incremental` honouring extra roots.
//!
//! Bug: previously, `update` only walked the primary root. Files in any extra
//! root (registered via `db::add_extra_root` and indexed by `rebuild`) were
//! seen as missing from the walk and deleted from the DB on every `update` —
//! wiping all third-party / external sources between sessions.
//!
//! These tests cover the round-trip: rebuild populates both roots, update
//! preserves them, and update still correctly detects real deletions and new
//! files in either root.

use std::fs;
use std::path::Path;

use ast_index::{db, indexer};
use rusqlite::Connection;
use tempfile::TempDir;

/// Initialise an on-disk DB at the conventional location for `project_root`.
/// We use `open_db` (not in-memory) so subsequent calls reopen the same DB,
/// matching how the CLI commands behave.
fn open_fresh_db(project_root: &Path) -> Connection {
    if db::db_exists(project_root) {
        db::delete_db(project_root).unwrap();
    }
    let conn = db::open_db(project_root).unwrap();
    db::init_db(&conn).unwrap();
    // Record the primary root in metadata, mirroring `cmd_rebuild`.
    conn.execute(
        "INSERT OR REPLACE INTO metadata (key, value) VALUES ('project_root', ?1)",
        rusqlite::params![project_root.to_string_lossy().to_string()],
    )
    .unwrap();
    conn
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn count_files(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap()
}

fn has_file(conn: &Connection, rel_path: &str) -> bool {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path = ?1",
            rusqlite::params![rel_path],
            |row| row.get(0),
        )
        .unwrap();
    count > 0
}

/// Rebuild the index for the given primary + extra roots, mimicking the
/// relevant slice of `cmd_rebuild`.
fn rebuild(conn: &mut Connection, primary: &Path, extras: &[&Path]) {
    for extra in extras {
        db::add_extra_root(conn, &extra.to_string_lossy()).unwrap();
    }
    indexer::index_directory_with_config(conn, primary, false, false, None, None).unwrap();
    for extra in extras {
        indexer::index_directory_with_config(conn, extra, false, false, None, None).unwrap();
    }
}

const SWIFT_SAMPLE: &str = "import Foundation\n\nclass Sample {}\n";

/// THE regression test: after rebuild populates both roots, a subsequent
/// `update` with no filesystem changes must not touch the file count and
/// must not report any deletions.
#[test]
fn update_preserves_extra_root_files() {
    let primary_dir = TempDir::new().unwrap();
    let extra_dir = TempDir::new().unwrap();
    let primary = primary_dir.path();
    let extra = extra_dir.path();

    write_file(&primary.join("App.swift"), SWIFT_SAMPLE);
    write_file(&extra.join("pkgs/Lib.swift"), SWIFT_SAMPLE);

    let mut conn = open_fresh_db(primary);
    rebuild(&mut conn, primary, &[extra]);

    let files_before = count_files(&conn);
    assert_eq!(files_before, 2, "expected both primary and extra files indexed");
    assert!(has_file(&conn, "App.swift"));
    assert!(has_file(&conn, "pkgs/Lib.swift"));

    let (updated, changed, deleted) =
        indexer::update_directory_incremental(&mut conn, primary, false, None, None).unwrap();

    assert_eq!(deleted, 0, "update must not delete extra-root files");
    assert_eq!(updated, 0);
    assert_eq!(changed, 0);
    assert_eq!(count_files(&conn), files_before);
    assert!(has_file(&conn, "pkgs/Lib.swift"), "extra-root file disappeared after update");
}

/// Update must still detect a genuine deletion under an extra root.
#[test]
fn update_detects_deleted_extra_root_file() {
    let primary_dir = TempDir::new().unwrap();
    let extra_dir = TempDir::new().unwrap();
    let primary = primary_dir.path();
    let extra = extra_dir.path();

    write_file(&primary.join("App.swift"), SWIFT_SAMPLE);
    write_file(&extra.join("a/Keep.swift"), SWIFT_SAMPLE);
    let drop_me = extra.join("a/Drop.swift");
    write_file(&drop_me, SWIFT_SAMPLE);

    let mut conn = open_fresh_db(primary);
    rebuild(&mut conn, primary, &[extra]);
    assert_eq!(count_files(&conn), 3);

    fs::remove_file(&drop_me).unwrap();

    let (_, _, deleted) =
        indexer::update_directory_incremental(&mut conn, primary, false, None, None).unwrap();

    assert_eq!(deleted, 1);
    assert!(has_file(&conn, "App.swift"));
    assert!(has_file(&conn, "a/Keep.swift"));
    assert!(!has_file(&conn, "a/Drop.swift"));
}

/// New files appearing under either root between sessions must be indexed.
#[test]
fn update_indexes_new_files_in_both_roots() {
    let primary_dir = TempDir::new().unwrap();
    let extra_dir = TempDir::new().unwrap();
    let primary = primary_dir.path();
    let extra = extra_dir.path();

    write_file(&primary.join("App.swift"), SWIFT_SAMPLE);
    write_file(&extra.join("Lib.swift"), SWIFT_SAMPLE);

    let mut conn = open_fresh_db(primary);
    rebuild(&mut conn, primary, &[extra]);
    assert_eq!(count_files(&conn), 2);

    write_file(&primary.join("New.swift"), SWIFT_SAMPLE);
    write_file(&extra.join("pkgs/NewLib.swift"), SWIFT_SAMPLE);

    let (updated, _, deleted) =
        indexer::update_directory_incremental(&mut conn, primary, false, None, None).unwrap();

    assert_eq!(deleted, 0);
    assert_eq!(updated, 2, "both new files should be parsed and stored");
    assert!(has_file(&conn, "New.swift"));
    assert!(has_file(&conn, "pkgs/NewLib.swift"));
}

/// If an extra root has been removed from disk entirely (e.g. user wiped a
/// build cache), update must skip the missing root and report its prior
/// entries as deleted, without crashing.
#[test]
fn update_skips_missing_extra_root() {
    let primary_dir = TempDir::new().unwrap();
    let extra_dir = TempDir::new().unwrap();
    let primary = primary_dir.path();
    let extra = extra_dir.path().to_path_buf();

    write_file(&primary.join("App.swift"), SWIFT_SAMPLE);
    write_file(&extra.join("Lib.swift"), SWIFT_SAMPLE);

    let mut conn = open_fresh_db(primary);
    rebuild(&mut conn, primary, &[&extra]);
    assert_eq!(count_files(&conn), 2);

    // Drop the extra root entirely.
    drop(extra_dir);
    assert!(!extra.exists());

    let (_, _, deleted) =
        indexer::update_directory_incremental(&mut conn, primary, false, None, None).unwrap();

    assert_eq!(deleted, 1, "files under the missing extra root should be marked deleted");
    assert!(has_file(&conn, "App.swift"));
    assert!(!has_file(&conn, "Lib.swift"));
}

/// Regression: previously `cmd_update` ignored `.ast-index.yaml`, so on a
/// monorepo configured with `include: [adfox, yabs/adfox]` the walker would
/// crawl the entire root, hang indefinitely, and pull in files from outside
/// the configured scope (e.g. `crypta/`, `sim/`). Now `update` accepts an
/// `include` list and only walks those sub-paths.
#[test]
fn update_with_include_skips_files_outside_include() {
    use ast_index::parsers;

    let primary_dir = TempDir::new().unwrap();
    let primary = primary_dir.path();

    // Pick an extension we definitely parse to keep the walker honest.
    assert!(parsers::is_supported_extension("rs"));

    write_file(&primary.join("adfox/src/a.rs"),  "fn a() {}\n");
    write_file(&primary.join("yabs/adfox/b.rs"), "fn b() {}\n");
    // Files outside the include — must NOT enter the index after update.
    write_file(&primary.join("crypta/c.rs"),     "fn c() {}\n");
    write_file(&primary.join("sim/d.rs"),        "fn d() {}\n");

    let mut conn = open_fresh_db(primary);

    // Seed the DB with only the in-scope files, the way `rebuild` with
    // `include` would have left it.
    indexer::index_directory_scoped(
        &mut conn, primary, &primary.join("adfox"), false, false, None, None,
    ).unwrap();
    indexer::index_directory_scoped(
        &mut conn, primary, &primary.join("yabs/adfox"), false, false, None, None,
    ).unwrap();

    let before = count_files(&conn);
    assert_eq!(before, 2, "seed: both in-scope files indexed, nothing else");
    assert!(has_file(&conn, "adfox/src/a.rs"));
    assert!(has_file(&conn, "yabs/adfox/b.rs"));

    let include = vec!["adfox".to_string(), "yabs/adfox".to_string()];

    let (_updated, _changed, deleted) =
        indexer::update_directory_incremental(&mut conn, primary, false, Some(&include), None)
            .unwrap();

    assert_eq!(deleted, 0, "no in-scope files were removed; nothing should be deleted");
    assert_eq!(count_files(&conn), 2, "update must not pull in crypta/ or sim/");
    assert!(!has_file(&conn, "crypta/c.rs"), "crypta/ is outside include — must stay out");
    assert!(!has_file(&conn, "sim/d.rs"),    "sim/ is outside include — must stay out");
}
