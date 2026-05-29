use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use libtest_mimic::{Arguments, Failed, Trial};
use serde::Deserialize;
use tempfile::TempDir;

#[derive(Deserialize)]
struct TestCase {
    #[serde(default)]
    description: Option<String>,
    command: Vec<String>,
    #[serde(default)]
    out: Vec<String>,
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn run_ast_index(fixture: &Path, db_path: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ast-index"))
        .args(args)
        .current_dir(fixture)
        .env("AST_INDEX_DB_PATH", db_path)
        .output()
        .expect("failed to spawn ast-index");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// Result lines carry indexed data (e.g. "UserService [class]: …").
// Headers ("Symbols matching …") and blank lines are skipped.
fn is_result_line(line: &str) -> bool {
    let t = line.trim();
    t.contains('[') && t.contains("]:")
}

fn collect_trials(dir: &Path, root: &Path, trials: &mut Vec<Trial>) {
    let yaml_path = dir.join("tests.yaml");
    if yaml_path.exists() {
        let rel = dir
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let yaml = fs::read_to_string(&yaml_path)
            .unwrap_or_else(|e| panic!("can't read {}: {e}", yaml_path.display()));
        let cases: Vec<TestCase> = serde_yaml::from_str(&yaml)
            .unwrap_or_else(|e| panic!("invalid tests.yaml in {rel}: {e}"));

        let db_dir = Arc::new(TempDir::new().expect("tempdir"));
        let db_path = db_dir.path().join("index.db");
        let fixture = dir.to_path_buf();

        let (ok, stdout, stderr) = run_ast_index(&fixture, &db_path, &["rebuild"]);
        assert!(
            ok,
            "rebuild failed for {rel}:\nstdout: {stdout}\nstderr: {stderr}"
        );

        for case in cases {
            let name = format!("{rel} | {}", case.command.join(" "));
            let fixture = fixture.clone();
            let db_dir = Arc::clone(&db_dir);
            let db_path = db_path.clone();

            trials.push(Trial::test(name, move || {
                let _ = &db_dir; // keep TempDir alive for the duration of the trial
                let args: Vec<&str> = case.command.iter().map(String::as_str).collect();
                let (_, stdout, stderr) = run_ast_index(&fixture, &db_path, &args);

                let mut errors = Vec::new();

                for entry in &case.out {
                    if !stdout.lines().any(|l| l.trim().contains(entry.as_str())) {
                        errors.push(format!("  missing:    {entry:?}"));
                    }
                }
                for line in stdout.lines().filter(|l| is_result_line(l)) {
                    if !case.out.iter().any(|e| line.trim().contains(e.as_str())) {
                        errors.push(format!("  unexpected: {:?}", line.trim()));
                    }
                }

                if errors.is_empty() {
                    Ok(())
                } else {
                    let mut msg = errors.join("\n");
                    if let Some(desc) = &case.description {
                        msg = format!("{desc}\n{msg}");
                    }
                    msg.push_str(&format!(
                        "\n  stdout:\n{}",
                        stdout
                            .lines()
                            .map(|l| format!("    {l}"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ));
                    if !stderr.trim().is_empty() {
                        msg.push_str(&format!("\n  stderr: {}", stderr.trim()));
                    }
                    Err(Failed::from(msg))
                }
            }));
        }
    }

    if let Ok(mut entries) = fs::read_dir(dir) {
        let mut subdirs: Vec<PathBuf> = entries
            .by_ref()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        subdirs.sort();
        for sub in subdirs {
            collect_trials(&sub, root, trials);
        }
    }
}

fn main() {
    let args = Arguments::from_args();
    let root = fixtures_root();
    let mut trials = Vec::new();
    collect_trials(&root, &root, &mut trials);
    libtest_mimic::run(&args, trials).exit();
}
