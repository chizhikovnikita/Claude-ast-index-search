//! Integration tests for the `module-route` command.
//!
//! Each test spins its own `TempDir`, inserts modules and edges directly into
//! a real SQLite DB, then either calls `cmd_module_route` for smoke-tests or
//! invokes the release binary for output-shape assertions.

use ast_index::commands::modules::cmd_module_route;
use ast_index::db;
use rusqlite::Connection;
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────────

fn open_fresh_db(project_root: &std::path::Path) -> Connection {
    if db::db_exists(project_root) {
        db::delete_db(project_root).unwrap();
    }
    let conn = db::open_db(project_root).unwrap();
    db::init_db(&conn).unwrap();
    conn
}

/// Insert a module row returning its id.
fn insert_module(conn: &Connection, name: &str, path: &str) -> i64 {
    conn.execute(
        "INSERT OR IGNORE INTO modules (name, path) VALUES (?1, ?2)",
        rusqlite::params![name, path],
    )
    .unwrap();
    conn.query_row(
        "SELECT id FROM modules WHERE name = ?1",
        rusqlite::params![name],
        |row| row.get(0),
    )
    .unwrap()
}

/// Insert a directed dependency edge.
fn insert_dep(conn: &Connection, from_id: i64, to_id: i64, kind: &str) {
    conn.execute(
        "INSERT INTO module_deps (module_id, dep_module_id, dep_kind) VALUES (?1, ?2, ?3)",
        rusqlite::params![from_id, to_id, kind],
    )
    .unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// 1. Linear chain A→B→C→D: shortest path from A to D should be a single path
///    with 3 hops.
#[test]
fn finds_shortest_path_in_chain() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let a = insert_module(&conn, "a", "a");
    let b = insert_module(&conn, "b", "b");
    let c = insert_module(&conn, "c", "c");
    let d = insert_module(&conn, "d", "d");
    insert_dep(&conn, a, b, "implementation");
    insert_dep(&conn, b, c, "implementation");
    insert_dep(&conn, c, d, "implementation");
    drop(conn);

    cmd_module_route(dir.path(), "a", "d", false, 50, 20, 2000, "all", "text")
        .expect("should succeed");
}

/// 2. Disconnected graph: querying unreachable module returns Ok without panic.
#[test]
fn unreachable_returns_empty_with_reason() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let _ = insert_module(&conn, "alpha", "alpha");
    let _ = insert_module(&conn, "beta", "beta");
    // No edge between alpha and beta.
    drop(conn);

    // Should succeed (Ok) but produce an "unreachable" empty_reason in output.
    cmd_module_route(dir.path(), "alpha", "beta", false, 50, 20, 2000, "all", "text")
        .expect("unreachable should return Ok");
}

/// REPRO: direct edge A→B with --all should return exactly 1 path (1 hop).
#[test]
fn direct_edge_all_returns_one_path() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let a = insert_module(&conn, "app", "app");
    let b = insert_module(&conn, "libx", "libx");
    insert_dep(&conn, a, b, "implementation");
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "app",
            "--to", "libx",
            "--all",
        ])
        .output()
        .expect("binary invocation must succeed");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be JSON: {e}; stdout={stdout}"));

    let paths = v["paths"].as_array().expect("paths must be array");
    assert_eq!(paths.len(), 1, "direct edge --all must yield 1 path; got: {}", stdout);
    assert_eq!(paths[0]["length"].as_u64().unwrap(), 1);
}

/// REPRO 2: real-world shape — direct edge AND indirect via intermediate, --all.
#[test]
fn direct_plus_indirect_all() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    // Use Gradle colon → dot normalised names matching real-world DB.
    let app = insert_module(&conn, "app", "app");
    let target = insert_module(&conn, "sdk.clips-viewer.shared.clips-design", "target");
    let mid = insert_module(&conn, "feature.foo", "mid");
    insert_dep(&conn, app, target, "implementation");
    insert_dep(&conn, app, mid, "implementation");
    insert_dep(&conn, mid, target, "implementation");
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", ":app",
            "--to", ":sdk:clips-viewer:shared:clips-design",
            "--all",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("must be JSON: {e}; stdout={stdout}"));
    let paths = v["paths"].as_array().expect("paths must be array");
    assert_eq!(paths.len(), 2, "direct + indirect --all must yield 2 paths; got: {}", stdout);
}

/// REPRO 3: heavy fanout where DFS may exhaust timeout before finding direct edge.
/// Build :app with many children; one direct edge to target; many decoy subtrees.
#[test]
fn heavy_fanout_all_with_direct_edge() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let app = insert_module(&conn, "app", "app");
    // Target name sorts late alphabetically so DFS reaches it last.
    let target = insert_module(&conn, "zzz.target", "target");
    insert_dep(&conn, app, target, "implementation");

    // Many decoy children with their own subtrees.
    for i in 0..30 {
        let child = insert_module(&conn, &format!("child{:02}", i), &format!("c{}", i));
        insert_dep(&conn, app, child, "implementation");
        for j in 0..20 {
            let gc = insert_module(&conn, &format!("g{:02}_{:02}", i, j), &format!("g{}_{}", i, j));
            insert_dep(&conn, child, gc, "implementation");
            for k in 0..5 {
                let ggc = insert_module(&conn, &format!("h{:02}_{:02}_{:02}", i, j, k), &format!("h{}_{}_{}", i, j, k));
                insert_dep(&conn, gc, ggc, "implementation");
            }
        }
    }
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "app",
            "--to", "zzz.target",
            "--all",
            "--timeout-ms", "100", // tight timeout to force the bug
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    eprintln!("REPRO output: {}", stdout);
    let paths = v["paths"].as_array().unwrap();
    // After fix: direct-target edge is processed first, so we always find the
    // 1-hop path even with a tight timeout.
    assert!(!paths.is_empty(), "must record direct edge before exploring siblings; got: {}", stdout);
    assert_eq!(paths[0]["length"].as_u64().unwrap(), 1);

    // search_stats must be populated for --all mode.
    let stats = &v["search_stats"];
    assert!(stats.is_object(), "search_stats must be present for --all; got: {}", stdout);
    // edges_explored must include the direct edge. nodes_visited may be 0
    // because reverse-BFS pruning lets us record the direct hit without
    // ever pushing a child frame.
    assert!(stats["edges_explored"].as_u64().unwrap_or(0) >= 1);
}

/// REPRO 4: completely unreachable target — reverse-BFS pruning detects
/// this instantly without consuming the timeout, so the message is
/// "unreachable", not "truncated_timeout".
#[test]
fn unreachable_pruning_avoids_timeout() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let app = insert_module(&conn, "app", "app");
    let _ = insert_module(&conn, "isolated.target", "tgt");
    // Big disconnected fanout: DFS without pruning would burn timeout, but
    // reverse-BFS sees target has no incoming edges and returns immediately.
    for i in 0..15 {
        let c = insert_module(&conn, &format!("c{:02}", i), &format!("c{}", i));
        insert_dep(&conn, app, c, "implementation");
        for j in 0..15 {
            let g = insert_module(&conn, &format!("g{:02}_{:02}", i, j), &format!("g{}_{}", i, j));
            insert_dep(&conn, c, g, "implementation");
        }
    }
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "app",
            "--to", "isolated.target",
            "--all",
            "--timeout-ms", "5000",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let empty_reason = v["empty_reason"].as_str().unwrap_or("");
    assert_eq!(empty_reason, "unreachable", "pruning must short-circuit; got: {}", stdout);
}

/// REPRO 5: large connected DAG with many simple paths from app→target —
/// max_paths cap is exercised, and pruning still keeps DFS bounded.
#[test]
fn many_paths_hits_max_paths_cap() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    // Diamond layered graph: app → l0_{0..4} → l1_{0..4} → ... → target.
    // Number of simple paths = 5^layers, easily exceeds max_paths=5.
    let app = insert_module(&conn, "app", "app");
    let target = insert_module(&conn, "target", "target");
    let mut prev_layer: Vec<i64> = vec![app];
    for layer in 0..3 {
        let mut cur: Vec<i64> = Vec::new();
        for i in 0..5 {
            let n = insert_module(&conn, &format!("L{}_{:02}", layer, i), &format!("L{}_{}", layer, i));
            cur.push(n);
        }
        for &p in &prev_layer {
            for &c in &cur {
                insert_dep(&conn, p, c, "implementation");
            }
        }
        prev_layer = cur;
    }
    for &p in &prev_layer {
        insert_dep(&conn, p, target, "implementation");
    }
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "app",
            "--to", "target",
            "--all",
            "--max-paths", "5",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let paths = v["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 5, "must cap at max_paths=5; got: {}", stdout);
    assert_eq!(v["truncated"].as_bool(), Some(true));
    assert_eq!(v["truncation_reason"].as_str(), Some("max_paths"));
}

/// 3. Cycle A→B→A and A→C→D: query A→D must not hang or panic.
///    `--all` mode with cycle: should find exactly [A→C→D].
#[test]
fn cycle_does_not_break_traversal() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let a = insert_module(&conn, "a", "a");
    let b = insert_module(&conn, "b", "b");
    let c = insert_module(&conn, "c", "c");
    let d = insert_module(&conn, "d", "d");
    insert_dep(&conn, a, b, "implementation");
    insert_dep(&conn, b, a, "implementation"); // cycle
    insert_dep(&conn, a, c, "implementation");
    insert_dep(&conn, c, d, "implementation");
    drop(conn);

    // --all with cycle must terminate and not infinite-loop.
    cmd_module_route(dir.path(), "a", "d", true, 50, 20, 2000, "all", "text")
        .expect("cycle must not cause infinite loop");
}

/// 4. Diamond A→B→D, A→C→D: --all should return 2 paths of length 2.
///    We test via JSON output from the binary to count paths structurally.
#[test]
fn all_paths_returns_diamond() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let a = insert_module(&conn, "a", "a");
    let b = insert_module(&conn, "b", "b");
    let c = insert_module(&conn, "c", "c");
    let d = insert_module(&conn, "d", "d");
    insert_dep(&conn, a, b, "implementation");
    insert_dep(&conn, a, c, "implementation");
    insert_dep(&conn, b, d, "implementation");
    insert_dep(&conn, c, d, "implementation");
    drop(conn);

    // Run via binary to inspect JSON.
    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "a",
            "--to", "d",
            "--all",
        ])
        .output()
        .expect("binary invocation must succeed");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must be valid JSON");

    let paths = v["paths"].as_array().expect("paths must be array");
    assert_eq!(
        paths.len(),
        2,
        "diamond graph must yield exactly 2 paths; got: {}",
        stdout
    );
    // Both paths must have length 2 (2 hops).
    for path in paths {
        assert_eq!(path["length"].as_u64().unwrap(), 2);
    }
}

/// 5. Kind filter eliminates the only path.
///    A→B via "implementation", query with --via-kind=api must return empty.
#[test]
fn kind_filter_eliminates_only_path() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let a = insert_module(&conn, "x", "x");
    let b = insert_module(&conn, "y", "y");
    insert_dep(&conn, a, b, "implementation");
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "x",
            "--to", "y",
            "--via-kind", "api",
        ])
        .output()
        .expect("binary invocation must succeed");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must be valid JSON");

    let paths = v["paths"].as_array().expect("paths must be array");
    assert!(paths.is_empty(), "api filter must eliminate implementation-only path");

    // Should have an empty_reason of "kind_filter" or "unreachable".
    let reason = v["empty_reason"].as_str().unwrap_or("");
    assert!(
        reason == "kind_filter" || reason == "unreachable",
        "expected kind_filter or unreachable, got: {}",
        reason
    );
}

/// 6. JSON output must be valid and contain no ANSI escape codes.
#[test]
fn json_output_shape_is_clean() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let a = insert_module(&conn, "mod_a", "mod_a");
    let b = insert_module(&conn, "mod_b", "mod_b");
    insert_dep(&conn, a, b, "api");
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "mod_a",
            "--to", "mod_b",
        ])
        .output()
        .expect("binary invocation must succeed");

    let stdout_bytes = &out.stdout;

    // No ANSI escape sequences in stdout.
    assert!(
        !stdout_bytes.windows(2).any(|w| w == b"\x1b["),
        "JSON output must not contain ANSI escape codes"
    );

    // Must parse as JSON.
    let stdout = String::from_utf8_lossy(stdout_bytes);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must be valid JSON");

    // Required top-level keys.
    assert!(v.get("from").is_some(), "JSON must have 'from' key");
    assert!(v.get("to").is_some(), "JSON must have 'to' key");
    assert!(v.get("paths").is_some(), "JSON must have 'paths' key");
    assert!(v.get("count").is_some(), "JSON must have 'count' key");
    assert!(v.get("truncated").is_some(), "JSON must have 'truncated' key");

    // Path from mod_a → mod_b (direct api edge).
    assert_eq!(v["count"].as_u64().unwrap(), 1);
}

/// 7. Fan-out graph: many paths, --max-paths 3 caps results and sets truncated.
#[test]
fn max_paths_truncation_signals() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    // Build a fan-out: root → n1..n10 → target (10+ paths via distinct intermediaries).
    let root = insert_module(&conn, "root", "root");
    let target = insert_module(&conn, "target", "target");
    for i in 1..=10 {
        let mid = insert_module(&conn, &format!("mid{}", i), &format!("mid{}", i));
        insert_dep(&conn, root, mid, "implementation");
        insert_dep(&conn, mid, target, "implementation");
    }
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "root",
            "--to", "target",
            "--all",
            "--max-paths", "3",
        ])
        .output()
        .expect("binary invocation must succeed");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must be valid JSON");

    let paths = v["paths"].as_array().expect("paths must be array");
    assert_eq!(paths.len(), 3, "max-paths=3 must cap at 3 paths");
    assert_eq!(v["truncated"].as_bool().unwrap(), true, "truncated must be true");
    assert_eq!(
        v["truncation_reason"].as_str().unwrap_or(""),
        "max_paths",
        "truncation_reason must be max_paths"
    );
}

/// 8. Self-query: querying A→A returns a trivial zero-length path with empty_reason="self".
#[test]
fn self_query_returns_trivial_path() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    // Need at least one module_dep row so dep_count > 0 (otherwise command bails early).
    let a = insert_module(&conn, "self_mod", "self_mod");
    let b = insert_module(&conn, "other_mod", "other_mod");
    insert_dep(&conn, a, b, "implementation");
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "json",
            "module-route",
            "--from", "self_mod",
            "--to", "self_mod",
        ])
        .output()
        .expect("binary invocation must succeed");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must be valid JSON");

    assert_eq!(
        v["empty_reason"].as_str().unwrap_or(""),
        "self",
        "self-query must return empty_reason=self"
    );
    assert_eq!(
        v["count"].as_u64().unwrap(),
        1,
        "self-query must return exactly 1 trivial path"
    );
}

/// Bonus: Mermaid output uses id aliases and has correct fence.
#[test]
fn mermaid_output_uses_id_aliases() {
    let dir = TempDir::new().unwrap();
    let conn = open_fresh_db(dir.path());

    let a = insert_module(&conn, "ma", "ma");
    let b = insert_module(&conn, "mb", "mb");
    insert_dep(&conn, a, b, "api");
    drop(conn);

    let bin = env!("CARGO_BIN_EXE_ast-index");
    let out = std::process::Command::new(bin)
        .env("AST_INDEX_DB_PATH", db::get_db_path(dir.path()).unwrap())
        .args([
            "--format", "mermaid",
            "module-route",
            "--from", "ma",
            "--to", "mb",
        ])
        .output()
        .expect("binary invocation must succeed");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("```mermaid"),
        "mermaid output must start with ```mermaid fence"
    );
    assert!(
        stdout.contains("n0[") || stdout.contains("n1["),
        "mermaid output must use id aliases like n0[ or n1["
    );
}
