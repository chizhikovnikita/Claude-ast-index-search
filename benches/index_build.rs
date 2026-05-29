//! Microbenchmarks for the default (non-experimental) indexing pipeline over
//! small synthetic projects laid out in `TempDir`s.
//!
//! Coverage:
//!   * full source walk + parse: `indexer::index_directory`
//!   * incremental update after one changed file: `update_directory_incremental`
//!   * module dependency graph build: `index_modules` + `index_module_dependencies`
//!   * Android indexes: `index_xml_usages` + `index_resources`
//!
//! Setups are done in Criterion `setup` closures and are *not* included in the
//! measured iteration time. This keeps the measurements focused on the pipeline
//! stage under test while still exercising public APIs against real on-disk
//! fixtures.
//!
//! Run with:
//!     cargo bench --bench index_build
//!     cargo bench --bench index_build -- --quick

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use tempfile::TempDir;

use ast_index::{db, indexer};

const RUST_FILE: &str = r#"
pub struct Counter { value: u64 }

impl Counter {
    pub fn new() -> Self { Self { value: 0 } }
    pub fn inc(&mut self) { self.value += 1; }
    pub fn get(&self) -> u64 { self.value }
}

pub trait Reporter {
    fn report(&self, label: &str, value: u64);
}

pub struct StdoutReporter;
impl Reporter for StdoutReporter {
    fn report(&self, label: &str, value: u64) {
        println!("{label}={value}");
    }
}

pub fn run(reporter: &dyn Reporter, ticks: u64) {
    let mut c = Counter::new();
    for _ in 0..ticks { c.inc(); }
    reporter.report("ticks", c.get());
}
"#;

const KOTLIN_FILE: &str = r#"
package bench.synth

interface Greeter { fun hello(name: String): String }

class FormalGreeter(private val title: String) : Greeter {
    override fun hello(name: String) = "$title $name"
}

data class Person(val first: String, val last: String) {
    val full: String get() = "$first $last"
}

object Greetings {
    fun greetAll(g: Greeter, people: List<Person>): List<String> =
        people.map { g.hello(it.full) }
}
"#;

const TS_FILE: &str = r#"
export interface Repo<T> {
    get(id: string): Promise<T | null>;
    put(id: string, value: T): Promise<void>;
}

export class MemRepo<T> implements Repo<T> {
    private data = new Map<string, T>();
    async get(id: string) { return this.data.get(id) ?? null; }
    async put(id: string, value: T) { this.data.set(id, value); }
}

export type Result<T, E = Error> =
    | { ok: true; value: T }
    | { ok: false; error: E };

export function ok<T>(value: T): Result<T> { return { ok: true, value }; }
export function err<E>(error: E): Result<never, E> { return { ok: false, error }; }
"#;

const PYTHON_FILE: &str = r#"
from dataclasses import dataclass
from typing import Iterable, Optional

@dataclass
class Item:
    sku: str
    qty: int
    price: float

class Cart:
    def __init__(self) -> None:
        self._items: list[Item] = []

    def add(self, item: Item) -> None:
        self._items.append(item)

    def total(self) -> float:
        return sum(i.qty * i.price for i in self._items)

    def find(self, sku: str) -> Optional[Item]:
        for it in self._items:
            if it.sku == sku:
                return it
        return None

def cart_from(items: Iterable[Item]) -> Cart:
    c = Cart()
    for it in items:
        c.add(it)
    return c
"#;

const GO_FILE: &str = r#"
package synth

type Stack struct { items []int }

func NewStack() *Stack { return &Stack{} }

func (s *Stack) Push(v int) { s.items = append(s.items, v) }

func (s *Stack) Pop() (int, bool) {
    n := len(s.items)
    if n == 0 { return 0, false }
    v := s.items[n-1]
    s.items = s.items[:n-1]
    return v, true
}

func (s *Stack) Len() int { return len(s.items) }
"#;

const SETTINGS_GRADLE: &str = r#"
rootProject.name = "bench"
include(":app")
include(":core")
"#;

const APP_BUILD_GRADLE: &str = r#"
plugins {
    kotlin("android")
}

dependencies {
    implementation(project(":core"))
}
"#;

const CORE_BUILD_GRADLE: &str = r#"
plugins {
    kotlin("jvm")
}
"#;

const ANDROID_KOTLIN_FILE: &str = r#"
package com.example.app

class CustomView

class MainActivity {
    fun title() = R.string.app_name
    fun logo() = R.drawable.ic_logo
}
"#;

const CORE_KOTLIN_FILE: &str = r#"
package com.example.core

class CoreApi
"#;

const LAYOUT_XML: &str = r#"
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent"
    android:layout_height="match_parent">
    <com.example.app.CustomView
        android:id="@+id/custom_view"
        android:layout_width="wrap_content"
        android:layout_height="wrap_content" />
    <TextView
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:text="@string/app_name" />
</LinearLayout>
"#;

const STRINGS_XML: &str = r#"
<resources>
    <string name="app_name">Bench App</string>
    <color name="brand_primary">#00FF00</color>
    <dimen name="screen_margin">16dp</dimen>
</resources>
"#;

const DRAWABLE_XML: &str = r#"
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="24dp"
    android:height="24dp"
    android:viewportWidth="24"
    android:viewportHeight="24">
</vector>
"#;

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// Build a synthetic project on disk once. Returns the project root path
/// (a stable subdirectory inside a `TempDir` held for the bench lifetime).
fn synth_project() -> &'static Path {
    static ROOT: OnceLock<(TempDir, PathBuf)> = OnceLock::new();
    let (_keep, root) = ROOT.get_or_init(|| {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path().join("project");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("kotlin")).unwrap();
        fs::create_dir_all(root.join("web")).unwrap();
        fs::create_dir_all(root.join("py")).unwrap();
        fs::create_dir_all(root.join("go")).unwrap();

        // ~12 files: 3 Rust, 3 Kotlin, 2 TS, 2 Python, 2 Go.
        for i in 0..3 {
            fs::write(root.join(format!("src/lib{i}.rs")), RUST_FILE).unwrap();
        }
        for i in 0..3 {
            fs::write(root.join(format!("kotlin/Module{i}.kt")), KOTLIN_FILE).unwrap();
        }
        for i in 0..2 {
            fs::write(root.join(format!("web/mod{i}.ts")), TS_FILE).unwrap();
        }
        for i in 0..2 {
            fs::write(root.join(format!("py/mod{i}.py")), PYTHON_FILE).unwrap();
        }
        for i in 0..2 {
            fs::write(root.join(format!("go/mod{i}.go")), GO_FILE).unwrap();
        }
        (tmp, root)
    });
    root.as_path()
}

fn create_synth_project(root: &Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("kotlin")).unwrap();
    fs::create_dir_all(root.join("web")).unwrap();
    fs::create_dir_all(root.join("py")).unwrap();
    fs::create_dir_all(root.join("go")).unwrap();

    for i in 0..3 {
        write_file(&root.join(format!("src/lib{i}.rs")), RUST_FILE);
    }
    for i in 0..3 {
        write_file(&root.join(format!("kotlin/Module{i}.kt")), KOTLIN_FILE);
    }
    for i in 0..2 {
        write_file(&root.join(format!("web/mod{i}.ts")), TS_FILE);
    }
    for i in 0..2 {
        write_file(&root.join(format!("py/mod{i}.py")), PYTHON_FILE);
    }
    for i in 0..2 {
        write_file(&root.join(format!("go/mod{i}.go")), GO_FILE);
    }
}

fn create_android_project(root: &Path) {
    write_file(&root.join("settings.gradle.kts"), SETTINGS_GRADLE);
    write_file(&root.join("app/build.gradle.kts"), APP_BUILD_GRADLE);
    write_file(&root.join("core/build.gradle.kts"), CORE_BUILD_GRADLE);
    write_file(
        &root.join("app/src/main/java/com/example/app/MainActivity.kt"),
        ANDROID_KOTLIN_FILE,
    );
    write_file(
        &root.join("core/src/main/java/com/example/core/CoreApi.kt"),
        CORE_KOTLIN_FILE,
    );
    write_file(
        &root.join("app/src/main/res/layout/activity_main.xml"),
        LAYOUT_XML,
    );
    write_file(
        &root.join("app/src/main/res/values/strings.xml"),
        STRINGS_XML,
    );
    write_file(
        &root.join("app/src/main/res/drawable/ic_logo.xml"),
        DRAWABLE_XML,
    );
}

fn fresh_db(root: &Path) {
    let conn = db::open_db(root).unwrap();
    db::init_db(&conn).unwrap();
}

fn bench_index_build(c: &mut Criterion) {
    let project = synth_project();

    let mut group = c.benchmark_group("index_build");
    // Each iter walks ~12 files + writes SQLite — keep the sample tight so
    // total wall time stays bounded even outside `--quick` mode.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("synthetic_~12_files", |b| {
        b.iter_with_setup(
            || {
                // Fresh DB dir per iteration so we measure cold-build cost.
                let dir = TempDir::new().expect("db tempdir");
                let conn = db::open_db(dir.path()).unwrap();
                db::init_db(&conn).unwrap();
                drop(conn);
                dir
            },
            |dir| {
                let mut conn = db::open_db(dir.path()).unwrap();
                let res = indexer::index_directory(&mut conn, project, false, false).unwrap();
                criterion::black_box(res);
            },
        );
    });

    group.finish();
}

fn bench_incremental_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_update");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("single_changed_file", |b| {
        b.iter_batched(
            || {
                let tmp = TempDir::new().expect("project tempdir");
                let root = tmp.path().join("project");
                create_synth_project(&root);
                fresh_db(&root);

                let mut conn = db::open_db(&root).unwrap();
                indexer::index_directory(&mut conn, &root, false, false).unwrap();
                drop(conn);

                write_file(
                    &root.join("src/lib1.rs"),
                    &format!("{RUST_FILE}\npub fn added_bench_symbol() -> u64 {{ 42 }}\n"),
                );
                (tmp, root)
            },
            |(_tmp, root)| {
                let mut conn = db::open_db(&root).unwrap();
                let res =
                    indexer::update_directory_incremental(&mut conn, &root, false, None, None)
                        .unwrap();
                black_box(res);
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

fn bench_module_graph(c: &mut Criterion) {
    let mut group = c.benchmark_group("module_graph");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("index_module_dependencies_default", |b| {
        b.iter_batched(
            || {
                let tmp = TempDir::new().expect("android tempdir");
                let root = tmp.path().join("project");
                create_android_project(&root);
                fresh_db(&root);

                let mut conn = db::open_db(&root).unwrap();
                let walk = indexer::index_directory(&mut conn, &root, false, false).unwrap();
                let module_count =
                    indexer::index_modules_from_files(&conn, &root, &walk.module_files).unwrap();
                assert!(module_count >= 2);
                let build_files = indexer::collect_build_files_from_db(&conn, &root).unwrap();
                (tmp, root, build_files)
            },
            |(_tmp, root, build_files)| {
                let mut conn = db::open_db(&root).unwrap();
                let deps =
                    indexer::index_module_dependencies(&mut conn, &root, &build_files, false)
                        .unwrap();
                let transitive = indexer::build_transitive_deps(&mut conn, false).unwrap();
                black_box((deps, transitive));
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

fn bench_android_indexes(c: &mut Criterion) {
    let mut group = c.benchmark_group("android_indexes");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("xml_and_resources_default", |b| {
        b.iter_batched(
            || {
                let tmp = TempDir::new().expect("android tempdir");
                let root = tmp.path().join("project");
                create_android_project(&root);
                fresh_db(&root);

                let mut conn = db::open_db(&root).unwrap();
                let walk = indexer::index_directory(&mut conn, &root, false, false).unwrap();
                let module_count =
                    indexer::index_modules_from_files(&conn, &root, &walk.module_files).unwrap();
                assert!(module_count >= 2);
                (tmp, root, walk.xml_layout_files, walk.res_files)
            },
            |(_tmp, root, xml_files, res_files)| {
                let mut conn = db::open_db(&root).unwrap();
                let xml = indexer::index_xml_usages(&mut conn, &root, &xml_files, false).unwrap();
                let resources =
                    indexer::index_resources(&mut conn, &root, &res_files, false).unwrap();
                black_box((xml, resources));
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_index_build,
    bench_incremental_update,
    bench_module_graph,
    bench_android_indexes
);
criterion_main!(benches);
