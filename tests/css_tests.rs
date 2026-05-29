//! End-to-end integration tests for CSS, SCSS, PCSS, and Less.
//!
//! For each format we write a representative source file into a fresh
//! `TempDir`, run the real indexer, then assert that the expected symbols
//! (selectors, variables, mixins, imports) land in SQLite.

use std::fs;
use std::path::Path;

use ast_index::{db, indexer};
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

fn symbol_kinds(conn: &Connection, name: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT kind FROM symbols WHERE name = ?1")
        .unwrap();
    let kinds: Vec<String> = stmt
        .query_map(rusqlite::params![name], |row| row.get(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    kinds
}

fn has_symbol(conn: &Connection, name: &str, kind: &str) -> bool {
    symbol_kinds(conn, name).iter().any(|k| k == kind)
}

#[test]
fn css_symbols_indexed_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::write(
        root.join("style.css"),
        r#":root {
  --brand-blue: #00f;
  --pad: 4px;
}

.btn {
  color: var(--brand-blue);
}

#main {
  padding: var(--pad);
}

@keyframes spin {
  from { opacity: 0; }
  to   { opacity: 1; }
}

@import "reset.css";
"#,
    )
    .unwrap();

    let mut conn = open_fresh_db(root);
    let result = indexer::index_directory(&mut conn, root, false, false).unwrap();
    assert!(result.file_count > 0, "indexer should walk .css files");

    assert!(has_symbol(&conn, "btn", "class"));
    assert!(has_symbol(&conn, "main", "object"));
    assert!(has_symbol(&conn, "spin", "function"));
    assert!(has_symbol(&conn, "--brand-blue", "constant"));
    assert!(has_symbol(&conn, "--pad", "constant"));
    assert!(has_symbol(&conn, "reset.css", "import"));
}

#[test]
fn pcss_routes_through_css_parser() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::write(
        root.join("style.pcss"),
        ".card { color: red; }\n#root { padding: 0; }\n",
    )
    .unwrap();
    fs::write(root.join("other.postcss"), ".other { color: blue; }\n").unwrap();

    let mut conn = open_fresh_db(root);
    indexer::index_directory(&mut conn, root, false, false).unwrap();

    assert!(has_symbol(&conn, "card", "class"));
    assert!(has_symbol(&conn, "root", "object"));
    assert!(has_symbol(&conn, "other", "class"));
}

#[test]
fn scss_symbols_indexed_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::write(
        root.join("_variables.scss"),
        "$primary: #ff0;\n$radius: 4px;\n",
    )
    .unwrap();
    fs::write(
        root.join("buttons.scss"),
        r#"@use "variables";
@forward "src/list";

@mixin button($size: 10px) {
  font-size: $size;
}

@function double($x) {
  @return $x * 2;
}

%card {
  padding: 10px;
}

.btn {
  @include button(20px);
  color: $primary;
  @extend %card;
}

#login {
  background: $primary;
}
"#,
    )
    .unwrap();

    let mut conn = open_fresh_db(root);
    indexer::index_directory(&mut conn, root, false, false).unwrap();

    // Variables (with `$` prefix kept)
    assert!(has_symbol(&conn, "$primary", "constant"));
    assert!(has_symbol(&conn, "$radius", "constant"));
    // Mixin / function
    assert!(has_symbol(&conn, "button", "function"));
    assert!(has_symbol(&conn, "double", "function"));
    // Placeholder selector
    assert!(has_symbol(&conn, "%card", "class"));
    // Regular selectors
    assert!(has_symbol(&conn, "btn", "class"));
    assert!(has_symbol(&conn, "login", "object"));
    // Imports
    assert!(has_symbol(&conn, "variables", "import"));
    assert!(has_symbol(&conn, "src/list", "import"));
}

#[test]
fn less_symbols_indexed_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::write(
        root.join("theme.less"),
        r#"@brand: #ff0;
@radius: 4px;

.rounded(@r: 4px) {
  border-radius: @r;
}

.btn {
  .rounded(@radius);
  color: @brand;
}

#main {
  padding: 0;
}

@import "reset.less";
"#,
    )
    .unwrap();

    let mut conn = open_fresh_db(root);
    indexer::index_directory(&mut conn, root, false, false).unwrap();

    // Variables (with `@` prefix kept)
    assert!(has_symbol(&conn, "@brand", "constant"));
    assert!(has_symbol(&conn, "@radius", "constant"));
    // Mixin definition `.rounded()`
    assert!(has_symbol(&conn, "rounded", "function"));
    // Selectors
    assert!(has_symbol(&conn, "btn", "class"));
    assert!(has_symbol(&conn, "main", "object"));
    // Import
    assert!(has_symbol(&conn, "reset.less", "import"));
}
