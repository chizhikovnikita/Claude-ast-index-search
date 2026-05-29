use std::fs;
use std::path::Path;

use ast_index::{db, indexer};
use tempfile::TempDir;

fn open_fresh_db(project_root: &Path) -> rusqlite::Connection {
    if db::db_exists(project_root) {
        db::delete_db(project_root).unwrap();
    }
    let conn = db::open_db(project_root).unwrap();
    db::init_db(&conn).unwrap();
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

const SWIFT_SAMPLE: &str = "import Foundation\n\nclass Sample {}\n";

fn rebuild(primary: &Path, extra: &Path) -> rusqlite::Connection {
    let mut conn = open_fresh_db(primary);
    db::add_extra_root(&conn, &extra.to_string_lossy()).unwrap();
    indexer::index_directory_with_config(&mut conn, primary, false, false, None).unwrap();
    indexer::index_directory_with_config(&mut conn, extra, false, false, None).unwrap();
    conn
}

#[test]
fn rebuild_allows_same_relative_path_in_primary_and_extra_root() {
    let primary_dir = TempDir::new().unwrap();
    let extra_dir = TempDir::new().unwrap();
    let rel = "Geko/Config.swift";

    write_file(&primary_dir.path().join(rel), SWIFT_SAMPLE);
    write_file(&extra_dir.path().join(rel), SWIFT_SAMPLE);

    let conn = rebuild(primary_dir.path(), extra_dir.path());

    let collisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path = ?1",
            rusqlite::params![rel],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(collisions, 2, "both roots must keep their own copy");

    let distinct_roots: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT root_path) FROM files WHERE path = ?1",
            rusqlite::params![rel],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        distinct_roots, 2,
        "duplicate rel_path must be namespaced by root"
    );
}

#[test]
fn update_deletes_only_the_missing_copy_of_a_colliding_path() {
    let primary_dir = TempDir::new().unwrap();
    let extra_dir = TempDir::new().unwrap();
    let rel = "Workspace.swift";

    write_file(&primary_dir.path().join(rel), SWIFT_SAMPLE);
    let extra_file = extra_dir.path().join(rel);
    write_file(&extra_file, SWIFT_SAMPLE);

    let mut conn = rebuild(primary_dir.path(), extra_dir.path());

    fs::remove_file(&extra_file).unwrap();

    let (_, _, deleted) =
        indexer::update_directory_incremental(&mut conn, primary_dir.path(), false, None, None)
            .unwrap();
    assert_eq!(
        deleted, 1,
        "only the missing extra-root file should be deleted"
    );

    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path = ?1",
            rusqlite::params![rel],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 1,
        "primary copy must remain after extra-root deletion"
    );

    let remaining_root: String = conn
        .query_row(
            "SELECT root_path FROM files WHERE path = ?1",
            rusqlite::params![rel],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_root,
        db::normalize_root_for_storage(primary_dir.path()),
        "the surviving row must belong to the primary root"
    );
}
