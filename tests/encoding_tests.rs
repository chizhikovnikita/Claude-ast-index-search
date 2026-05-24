//! End-to-end tests for Windows-1251 (CP1251) source file support.
//!
//! These cover the indexer pipeline (`fs::read_to_string` → `encoding::read_file_to_string`)
//! and the public `decode_bytes` API. Without `src/encoding.rs`, a CP1251 file would
//! fail UTF-8 validation in `fs::read_to_string` and be silently skipped.

use std::fs;
use std::path::Path;

use ast_index::{db, encoding, indexer};
use rusqlite::Connection;
use tempfile::TempDir;

fn open_fresh_db(project_root: &Path) -> Connection {
    if db::db_exists(project_root) {
        db::delete_db(project_root).unwrap();
    }
    let conn = db::open_db(project_root).unwrap();
    db::init_db(&conn).unwrap();
    conn
}

fn has_symbol(conn: &Connection, name: &str) -> bool {
    let mut stmt = conn
        .prepare("SELECT 1 FROM symbols WHERE name = ?1 LIMIT 1")
        .unwrap();
    stmt.exists(rusqlite::params![name]).unwrap()
}

#[test]
fn decode_bytes_passes_utf8_through_unchanged() {
    let s = "function processOrder() { return 'ok'; } // Обрабатывает";
    let out = encoding::decode_bytes(s.as_bytes(), None);
    assert_eq!(out, s);
}

#[test]
fn decode_bytes_handles_cp1251_cyrillic() {
    // "Привет, мир" in Windows-1251 bytes:
    let cp1251 = [
        0xCFu8, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, 0x2C, 0x20, 0xEC, 0xE8, 0xF0,
    ];
    let out = encoding::decode_bytes(&cp1251, None);
    assert_eq!(out, "Привет, мир");
}

#[test]
fn indexer_processes_cp1251_php_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let php_utf8 = "<?php\n\
        // Обрабатывает заказ пользователя\n\
        function processOrder($userId) {\n\
            // Возвращает true если успешно\n\
            return true;\n\
        }\n";
    let (cp1251_bytes, _, had_errors) = encoding_rs::WINDOWS_1251.encode(php_utf8);
    assert!(!had_errors, "CP1251 must encode all bytes in fixture");
    fs::write(root.join("test.php"), &*cp1251_bytes).unwrap();

    let mut conn = open_fresh_db(root);
    indexer::index_directory(&mut conn, root, false, false).unwrap();

    assert!(
        has_symbol(&conn, "processOrder"),
        "indexer must produce 'processOrder' symbol from a CP1251-encoded PHP file"
    );
}

#[test]
fn mixed_encoding_project_indexes_both_files() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // utf8.php — already UTF-8.
    fs::write(
        root.join("utf8.php"),
        "<?php\nfunction utf8Func() { return 1; }\n",
    )
    .unwrap();

    // cp1251.php — UTF-8 source re-encoded to Windows-1251 bytes.
    let php_utf8 = "<?php\n\
        // Комментарий в CP1251\n\
        function cp1251Func() { return 2; }\n";
    let (cp1251_bytes, _, _) = encoding_rs::WINDOWS_1251.encode(php_utf8);
    fs::write(root.join("cp1251.php"), &*cp1251_bytes).unwrap();

    let mut conn = open_fresh_db(root);
    indexer::index_directory(&mut conn, root, false, false).unwrap();

    assert!(has_symbol(&conn, "utf8Func"), "UTF-8 file must still index");
    assert!(
        has_symbol(&conn, "cp1251Func"),
        "CP1251 file must index after auto-detection"
    );
}

#[test]
fn cp1251_file_increments_fallback_counter() {
    let _ = encoding::take_fallback_count();
    let (cp1251_bytes, _, _) = encoding_rs::WINDOWS_1251.encode("Привет");
    let _ = encoding::decode_bytes(&cp1251_bytes, None);
    assert!(
        encoding::take_fallback_count() >= 1,
        "fallback decode must increment FALLBACK_DECODE_COUNT"
    );
}
