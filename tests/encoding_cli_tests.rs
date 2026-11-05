use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

const LEGACY_SOURCE: &str = "import { helper } from './helper';\n\
    // Русский исходный код: обработка заказа пользователя\n\
    export function processLegacy() {\n\
      // TODO: Заявка пользователя принята\n\
      return helper('Привет, мир');\n\
    }\n\
    export function обработатьЗаказ() { return 'Заказ готов'; }\n";

struct Project {
    _temp: TempDir,
    root: PathBuf,
    cache: PathBuf,
}

impl Project {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = Self {
            root,
            cache: temp.path().join("cache"),
            _temp: temp,
        };
        project.write("package.json", "{\"name\":\"encoding-fixture\"}\n");
        project.write_cp1251("legacy.ts", LEGACY_SOURCE);
        project.write(
            "helper.ts",
            "export function helper(value: string) { return value; }\n",
        );
        project.write(
            "modern.ts",
            "export function processModern() { return 'Современный UTF-8'; }\n",
        );
        project
    }

    fn write(&self, path: &str, content: &str) {
        fs::write(self.root.join(path), content).unwrap();
    }

    fn write_cp1251(&self, path: &str, content: &str) -> Vec<u8> {
        let (bytes, _, errors) = encoding_rs::WINDOWS_1251.encode(content);
        assert!(!errors, "fixture must be representable in Windows-1251");
        fs::write(self.root.join(path), bytes.as_ref()).unwrap();
        bytes.into_owned()
    }

    fn output(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_ast-index"))
            .current_dir(&self.root)
            .env("AST_INDEX_CACHE_DIR", &self.cache)
            .env("AST_INDEX_DISABLE_GC", "1")
            .env("AST_INDEX_SKIP_MINIFIED", "1")
            .env("NO_COLOR", "1")
            .env_remove("AST_INDEX_DB_PATH")
            .env_remove("KOTLIN_INDEX_DB_PATH")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            !output.stdout.windows(2).any(|bytes| bytes == b"\x1b["),
            "stdout must not contain ANSI escapes"
        );
        output
    }

    fn run(&self, args: &[&str]) -> String {
        String::from_utf8(self.output(args).stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.extend_from_slice(&["--format", "json"]);
        let text = self.run(&args);
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{args:?} did not return JSON: {error}; {text}"))
    }
}

#[test]
fn mixed_encodings_work_in_search_source_outline_and_imports() {
    let project = Project::new();
    let original = fs::read(project.root.join("legacy.ts")).unwrap();
    let rebuild = project.output(&["rebuild"]);
    assert!(String::from_utf8_lossy(&rebuild.stderr).contains("[encoding]"));

    for name in ["processLegacy", "processModern", "обработатьЗаказ"] {
        let found = project.json(&["symbol", name]);
        assert!(
            found["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["name"] == name),
            "{found:#}"
        );
    }

    let symbol = project.json(&["symbol", "processLegacy", "--with-content"]);
    let content = symbol["items"][0]["content"].as_str().unwrap();
    assert!(content.contains("Заявка пользователя принята"), "{content}");
    assert!(content.contains("Привет, мир"), "{content}");
    let search_source = project.json(&["search", "processLegacy", "--with-content"]);
    assert!(search_source["symbols"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Привет, мир"));

    let search = project.json(&["search", "Заявка"]);
    let hits = search["content_matches"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{search:#}");
    assert_eq!(hits[0]["line"], 4);
    assert!(hits[0]["content"].as_str().unwrap().contains("Заявка"));

    let todo = project.run(&["todo", "Заявка"]);
    assert!(todo.contains("legacy.ts:4"), "{todo}");
    assert!(todo.contains("Заявка пользователя принята"), "{todo}");

    let outline = project.json(&["outline", "legacy.ts"]);
    assert!(outline["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "processLegacy"));
    assert!(outline["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "обработатьЗаказ"));
    let imports = project.run(&["imports", "legacy.ts"]);
    assert!(imports.contains("helper"), "{imports}");

    let explore = project.json(&["explore", "processLegacy"]);
    assert!(explore.to_string().contains("Привет, мир"), "{explore:#}");
    assert_eq!(fs::read(project.root.join("legacy.ts")).unwrap(), original);
}

#[test]
fn incremental_update_reindexes_cp1251_text_and_word_prefilter() {
    let project = Project::new();
    project.run(&["rebuild"]);
    let updated = LEGACY_SOURCE
        .replace("processLegacy", "processUpdated")
        .replace(
            "Заявка пользователя принята",
            "Обновлённая заявка пользователя сохранена",
        );
    let expected_bytes = project.write_cp1251("legacy.ts", &updated);

    project.run(&["update"]);
    let symbol = project.json(&["symbol", "processUpdated", "--with-content"]);
    assert_eq!(symbol["items"][0]["name"], "processUpdated");
    assert!(symbol["items"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Обновлённая заявка"));
    let old = project.json(&["symbol", "processLegacy"]);
    assert!(old["items"].as_array().unwrap().is_empty(), "{old:#}");
    let search = project.json(&["search", "Обновлённая"]);
    assert_eq!(
        search["content_matches"].as_array().unwrap().len(),
        1,
        "{search:#}"
    );
    assert_eq!(
        fs::read(project.root.join("legacy.ts")).unwrap(),
        expected_bytes
    );
    project.run(&["update"]);
    assert_eq!(project.json(&["stats"])["stats"]["file_count"], 3);
}

#[test]
fn graph_resolves_imports_from_cp1251_source() {
    let project = Project::new();
    project.write(
        "unrelated.ts",
        "export function helper(value: string) { return value + 'other'; }\n",
    );
    project.run(&["rebuild"]);
    project.json(&["graph", "build"]);
    let report = project.json(&["graph", "dependencies", "processLegacy"]);
    let dependencies = report["items"].as_array().unwrap();
    let helper = dependencies
        .iter()
        .find(|item| item["other"]["name"] == "helper")
        .unwrap_or_else(|| panic!("missing imported helper in {report:#}"));
    assert_eq!(helper["confidence"], "import", "{helper:#}");
    assert!(helper["other"]["path"]
        .as_str()
        .unwrap()
        .ends_with("helper.ts"));
}

#[test]
fn cp1251_search_preserves_minified_exclusions_and_limits() {
    let project = Project::new();
    project.write_cp1251(
        "ignored.min.js",
        "function ignored() { return 'Заявка пользователя принята'; }\n",
    );
    project.write_cp1251(
        "another.ts",
        "// TODO: Заявка пользователя принята\n// TODO: Заявка повторно принята\n",
    );
    project.run(&["rebuild"]);

    let search = project.json(&["search", "Заявка", "--limit", "1"]);
    assert_eq!(
        search["content_matches"].as_array().unwrap().len(),
        1,
        "{search:#}"
    );
    assert_eq!(search["pagination"]["content_matches"]["total"], 3);
    let search = project.json(&["search", "Заявка", "--limit", "20"]);
    assert_eq!(
        search["content_matches"].as_array().unwrap().len(),
        3,
        "{search:#}"
    );
    assert!(!search.to_string().contains("ignored.min.js"), "{search:#}");
}
