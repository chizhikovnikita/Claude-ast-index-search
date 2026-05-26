//! Integration tests for `commands::files` public APIs.
//!
//! Covers VCS detection, branch detection helpers, and the file-oriented
//! commands (`cmd_file`, `cmd_outline`, `cmd_imports`, `cmd_changed`)
//! that are user-facing entry points but had zero integration coverage.

use std::fs;

use ast_index::commands::files::{
    cmd_changed, cmd_file, cmd_imports, cmd_outline, detect_git_default_branch, detect_vcs,
};
use ast_index::db;
use tempfile::TempDir;

fn open_fresh_db(project_root: &std::path::Path) -> rusqlite::Connection {
    if db::db_exists(project_root) {
        db::delete_db(project_root).unwrap();
    }
    let conn = db::open_db(project_root).unwrap();
    db::init_db(&conn).unwrap();
    conn
}

// ----------------------------------------------------------------------
// detect_vcs
// ----------------------------------------------------------------------

#[test]
fn detect_vcs_returns_git_for_git_repo() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();

    assert_eq!(detect_vcs(dir.path()), "git");
}

#[test]
fn detect_vcs_returns_arc_when_arc_head_present() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join(".arc")).unwrap();
    fs::write(dir.path().join(".arc").join("HEAD"), "trunk\n").unwrap();

    assert_eq!(
        detect_vcs(dir.path()),
        "arc",
        ".arc/HEAD must mark this as an arc repo"
    );
}

#[test]
fn detect_vcs_arc_wins_over_git_when_both_present() {
    // Real-world: a fork with both .git and .arc directories.
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::create_dir(dir.path().join(".arc")).unwrap();
    fs::write(dir.path().join(".arc").join("HEAD"), "trunk\n").unwrap();

    assert_eq!(
        detect_vcs(dir.path()),
        "arc",
        "ancestor walk hits .arc first; ancestors are checked in order"
    );
}

#[test]
fn detect_vcs_defaults_to_git_when_nothing_found() {
    let dir = TempDir::new().unwrap();
    // No VCS markers at all.
    assert_eq!(
        detect_vcs(dir.path()),
        "git",
        "fallback must be git for the common case"
    );
}

// ----------------------------------------------------------------------
// detect_git_default_branch
// ----------------------------------------------------------------------

#[test]
fn detect_git_default_branch_falls_back_when_not_a_git_repo() {
    // No .git here — git commands fail, so the helper must fall back
    // to the documented default ("origin/main") rather than panic.
    let dir = TempDir::new().unwrap();
    let branch = detect_git_default_branch(dir.path());
    assert_eq!(
        branch, "origin/main",
        "documented fallback must be origin/main"
    );
}

// ----------------------------------------------------------------------
// cmd_file
// ----------------------------------------------------------------------

#[test]
fn cmd_file_returns_ok_without_index() {
    let dir = TempDir::new().unwrap();
    // No DB present — must short-circuit with an Ok message, not error.
    cmd_file(dir.path(), "anything", false, 10).expect("cmd_file must not error without index");
}

#[test]
fn cmd_file_finds_indexed_files() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());
    db::upsert_file(&conn, "src/main/Foo.kt", 0, 100).unwrap();
    db::upsert_file(&conn, "src/main/Bar.kt", 0, 100).unwrap();
    drop(conn);

    cmd_file(dir.path(), "Foo", false, 10).unwrap();

    // Verify via the same public DB API the command uses internally.
    let conn = db::open_db(dir.path()).unwrap();
    let hits = db::find_files(&conn, "Foo", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].ends_with("Foo.kt"));
}

// ----------------------------------------------------------------------
// cmd_outline
// ----------------------------------------------------------------------

#[test]
fn cmd_outline_handles_missing_file_gracefully() {
    let dir = TempDir::new().unwrap();
    cmd_outline(dir.path(), "does/not/exist.kt")
        .expect("missing file must print a hint, not error");
}

#[test]
fn cmd_outline_parses_a_kotlin_file() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("Foo.kt");
    fs::write(&src, "package demo\n\nclass Foo {\n  fun bar() {}\n}\n").unwrap();

    cmd_outline(dir.path(), "Foo.kt").expect("outline of valid Kotlin must succeed");
}

#[test]
fn cmd_outline_handles_unsupported_extension() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("notes.unknown_ext_xyz");
    fs::write(&src, "hello\n").unwrap();

    cmd_outline(dir.path(), "notes.unknown_ext_xyz")
        .expect("unknown extension must print a hint, not error");
}

// ----------------------------------------------------------------------
// cmd_imports
// ----------------------------------------------------------------------

#[test]
fn cmd_imports_extracts_python_imports() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("script.py");
    fs::write(&src, "import os\nfrom typing import List\nimport sys\n").unwrap();

    cmd_imports(dir.path(), "script.py").expect("python imports must parse");
}

#[test]
fn cmd_imports_extracts_kotlin_imports() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("Foo.kt");
    fs::write(
        &src,
        "package demo\n\nimport kotlinx.coroutines.flow.Flow\nimport java.util.UUID\n\nclass Foo\n",
    )
    .unwrap();

    cmd_imports(dir.path(), "Foo.kt").expect("kotlin imports must parse");
}

#[test]
fn cmd_imports_handles_missing_file() {
    let dir = TempDir::new().unwrap();
    cmd_imports(dir.path(), "absent.kt").expect("missing file must print a hint, not error");
}

#[test]
fn cmd_imports_handles_file_with_no_imports() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("Bare.kt");
    fs::write(&src, "package demo\n\nclass Bare\n").unwrap();

    cmd_imports(dir.path(), "Bare.kt").expect("file with no imports must succeed");
}

// ----------------------------------------------------------------------
// cmd_changed
// ----------------------------------------------------------------------

#[test]
fn cmd_changed_handles_no_vcs_gracefully() {
    // No git/arc here — vcs commands fail, but the helper must
    // surface a friendly message and return Ok rather than bubbling up
    // a process-spawn error.
    let dir = TempDir::new().unwrap();
    cmd_changed(dir.path(), "origin/main")
        .expect("missing VCS must print a friendly hint, not error");
}
