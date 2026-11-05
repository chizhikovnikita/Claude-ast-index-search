use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use ast_index::db::{self, SearchScope, SymbolKind};
use rusqlite::Connection;
use tempfile::TempDir;

fn selector_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    db::init_db(&conn).unwrap();
    for path in ["src/nested/inside.rs", "src/outside.rs"] {
        let file = db::upsert_file(&conn, path, 0, 10).unwrap();
        db::insert_symbol(&conn, file, "PathTarget", SymbolKind::Function, 1, None).unwrap();
        conn.execute(
            "INSERT INTO refs(name, file_id, line, context) VALUES ('PathTarget', ?1, 2, 'PathTarget()')",
            [file],
        )
        .unwrap();
    }
    conn
}

#[test]
fn file_queries_and_scopes_accept_platform_path_selectors() {
    let conn = selector_db();
    let selectors = if cfg!(windows) {
        vec!["src/nested", r"src\nested"]
    } else {
        vec!["src/nested"]
    };
    for selector in selectors {
        assert_eq!(
            db::find_files(&conn, selector, 10).unwrap(),
            ["src/nested/inside.rs"]
        );
        assert_eq!(
            db::find_files_with_roots(&conn, selector, 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db::count_files_with_roots_scoped(&conn, selector, &SearchScope::none()).unwrap(),
            1
        );
        assert_eq!(
            db::count_files_with_roots_terms_scoped(&conn, &[selector], &SearchScope::none())
                .unwrap(),
            1
        );
        assert_eq!(
            db::find_files_with_roots_terms_scoped(&conn, &[selector], 10, &SearchScope::none())
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db::find_files_with_roots_scoped(&conn, selector, 10, &SearchScope::none())
                .unwrap()
                .len(),
            1
        );

        for scope in [
            SearchScope {
                in_file: Some(selector),
                ..SearchScope::none()
            },
            SearchScope {
                dir_prefix: Some(selector),
                ..SearchScope::none()
            },
            SearchScope {
                module: Some(selector),
                ..SearchScope::none()
            },
        ] {
            assert!(scope.matches_path("src/nested/inside.rs"));
            assert!(!scope.matches_path("src/outside.rs"));
            assert_eq!(
                db::count_symbols_by_name_scoped(&conn, "PathTarget", None, &scope, false).unwrap(),
                1
            );
            assert_eq!(
                db::count_references_scoped(&conn, "PathTarget", &scope).unwrap(),
                1
            );
            let refs = db::find_references_scoped(&conn, "PathTarget", 10, &scope).unwrap();
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].path, "src/nested/inside.rs");
            assert_eq!(
                db::count_files_with_roots_scoped(&conn, "", &scope).unwrap(),
                1
            );
        }
    }
}

fn run(cwd: &Path, cache: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_ast-index"))
        .current_dir(cwd)
        .args(args)
        .env("AST_INDEX_CACHE_DIR", cache)
        .env("AST_INDEX_DISABLE_GC", "1")
        .env_remove("AST_INDEX_DB_PATH")
        .env_remove("KOTLIN_INDEX_DB_PATH")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn indexed_paths_use_slashes_and_cli_scopes_nested_directories() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    let nested = root.join("src/nested");
    let cache = temp.path().join("cache");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"paths\"\nversion=\"0.0.0\"\n",
    )
    .unwrap();
    fs::write(nested.join("inside.rs"), "pub fn scoped_inside() {}\n").unwrap();
    fs::write(root.join("src/outside.rs"), "pub fn scoped_outside() {}\n").unwrap();
    run(&root, &cache, &["rebuild"]);

    let stats: serde_json::Value =
        serde_json::from_slice(&run(&root, &cache, &["stats", "--format", "json"]).stdout).unwrap();
    let conn = Connection::open(stats["db_path"].as_str().unwrap()).unwrap();
    let mut paths = db::find_files(&conn, "", 10).unwrap();
    paths.sort();
    assert_eq!(paths, ["src/nested/inside.rs", "src/outside.rs"]);
    drop(conn);

    let all: serde_json::Value = serde_json::from_slice(
        &run(&root, &cache, &["symbol", "scoped_", "--format", "json"]).stdout,
    )
    .unwrap();
    assert_eq!(all["items"].as_array().unwrap().len(), 2);

    let scoped: serde_json::Value = serde_json::from_slice(
        &run(&nested, &cache, &["symbol", "scoped_", "--format", "json"]).stdout,
    )
    .unwrap();
    assert_eq!(scoped["items"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["items"][0]["name"], "scoped_inside");

    let selector = if cfg!(windows) {
        r"src\nested"
    } else {
        "src/nested"
    };
    let selected: serde_json::Value = serde_json::from_slice(
        &run(
            &root,
            &cache,
            &[
                "symbol",
                "scoped_",
                "--in-file",
                selector,
                "--format",
                "json",
            ],
        )
        .stdout,
    )
    .unwrap();
    assert_eq!(selected["items"].as_array().unwrap().len(), 1);
    assert_eq!(selected["items"][0]["name"], "scoped_inside");
    let files: serde_json::Value =
        serde_json::from_slice(&run(&root, &cache, &["file", selector, "--format", "json"]).stdout)
            .unwrap();
    assert_eq!(files.as_array().unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn posix_backslashes_remain_literal_filename_characters() {
    let temp = TempDir::new().unwrap();
    let path = r"src\literal.rs";
    fs::write(temp.path().join(path), "pub fn literal_backslash() {}\n").unwrap();
    let mut conn = Connection::open_in_memory().unwrap();
    db::init_db(&conn).unwrap();
    ast_index::indexer::index_directory(&mut conn, temp.path(), false, false).unwrap();
    assert_eq!(db::find_files(&conn, path, 10).unwrap(), [path]);
    assert!(db::find_files(&conn, "src/literal.rs", 10)
        .unwrap()
        .is_empty());
    let scope = SearchScope {
        in_file: Some(path),
        ..SearchScope::none()
    };
    assert!(scope.matches_path(path));
    assert!(!scope.matches_path("src/literal.rs"));
    assert_eq!(
        db::count_files_with_roots_scoped(&conn, path, &scope).unwrap(),
        1
    );
}
