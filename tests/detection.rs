use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use moru_narrow::{install, kotlin, python, rust_lang, typescript};
use serde_json::json;
use tempfile::TempDir;

fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }
}

fn setup_kotlin_project(root: &Path, dep_line: &str, extra: &[(&str, &str)]) {
    let base = [
        (
            "settings.gradle.kts",
            "include(\":core:ui\")\ninclude(\":feature:home\")\n",
        ),
        ("core/ui/build.gradle.kts", ""),
        (
            "core/ui/src/main/kotlin/Card.kt",
            "package app.ui\nfun Card(title: String) {}\n",
        ),
        ("feature/home/build.gradle.kts", dep_line),
        (
            "feature/home/src/main/kotlin/Home.kt",
            "package app.home\nimport app.ui.Card\nfun home() { Card(\"x\") }\n",
        ),
    ];
    write_files(root, &base);
    write_files(root, extra);
}

fn kotlin_tier1(root: &Path, name: &str, decl: &str) -> BTreeSet<String> {
    let mut repo = kotlin::Repo::new(root);
    let decls = BTreeSet::from([decl.to_string()]);
    let res = kotlin::recommend(&mut repo, name, Some(&decls));
    res.tiers[0].keys().cloned().collect()
}

fn ts_tier1(root: &Path, name: &str, decl: &str) -> BTreeSet<String> {
    let mut repo = typescript::Repo::new(root);
    let decls = BTreeSet::from([decl.to_string()]);
    let res = typescript::recommend(&mut repo, name, Some(&decls));
    res.tiers[0].keys().cloned().collect()
}

#[test]
fn kotlin_project_string_dependency() {
    let tmp = TempDir::new().unwrap();
    setup_kotlin_project(
        tmp.path(),
        r#"dependencies { implementation(project(":core:ui")) }"#,
        &[],
    );
    assert!(kotlin_tier1(tmp.path(), "Card", "core/ui/src/main/kotlin/Card.kt")
        .contains("feature/home/src/main/kotlin/Home.kt"));
}

#[test]
fn kotlin_project_path_argument() {
    let tmp = TempDir::new().unwrap();
    setup_kotlin_project(
        tmp.path(),
        r#"dependencies { implementation(project(path = ":core:ui")) }"#,
        &[],
    );
    assert!(kotlin_tier1(tmp.path(), "Card", "core/ui/src/main/kotlin/Card.kt")
        .contains("feature/home/src/main/kotlin/Home.kt"));
}

#[test]
fn kotlin_typesafe_accessor() {
    let tmp = TempDir::new().unwrap();
    setup_kotlin_project(
        tmp.path(),
        "dependencies { implementation(projects.core.ui) }",
        &[],
    );
    assert!(kotlin_tier1(tmp.path(), "Card", "core/ui/src/main/kotlin/Card.kt")
        .contains("feature/home/src/main/kotlin/Home.kt"));
}

#[test]
fn kotlin_convention_plugin_dependency() {
    let tmp = TempDir::new().unwrap();
    setup_kotlin_project(
        tmp.path(),
        r#"plugins { id("app.feature") }"#,
        &[(
            "build-logic/convention/src/main/kotlin/FeaturePlugin.kt",
            "fun Project.deps() { add(\"implementation\", project(\":core:ui\")) }\n",
        )],
    );
    assert!(kotlin_tier1(tmp.path(), "Card", "core/ui/src/main/kotlin/Card.kt")
        .contains("feature/home/src/main/kotlin/Home.kt"));
}

#[test]
fn kotlin_registered_convention_plugin_applies_only_to_its_modules() {
    let tmp = TempDir::new().unwrap();
    setup_kotlin_project(
        tmp.path(),
        "plugins { alias(libs.plugins.app.feature) }",
        &[
            (
                "gradle/libs.versions.toml",
                "[plugins]\napp-feature = { id = \"app.feature\" }\n",
            ),
            (
                "build-logic/convention/build.gradle.kts",
                "register(\"feature\") {\n id = libs.plugins.app.feature.get().pluginId\n implementationClass = \"FeaturePlugin\"\n}\n",
            ),
            (
                "build-logic/convention/src/main/kotlin/FeaturePlugin.kt",
                "class FeaturePlugin { fun f() { add(\"implementation\", project(\":core:ui\")) } }\n",
            ),
            ("feature/other/build.gradle.kts", ""),
            (
                "feature/other/src/main/kotlin/Other.kt",
                "package app.other\nimport app.ui.Card\nfun o() { Card(\"x\") }\n",
            ),
        ],
    );
    let t1 = kotlin_tier1(tmp.path(), "Card", "core/ui/src/main/kotlin/Card.kt");
    assert!(t1.contains("feature/home/src/main/kotlin/Home.kt"));
    assert!(!t1.contains("feature/other/src/main/kotlin/Other.kt"));
}

#[test]
fn kotlin_declarations_in_two_files_are_grouped() {
    let tmp = TempDir::new().unwrap();
    write_files(
        tmp.path(),
        &[
            (
                "settings.gradle.kts",
                "include(\":a\")\ninclude(\":b\")\ninclude(\":app\")\n",
            ),
            ("a/build.gradle.kts", ""),
            ("b/build.gradle.kts", ""),
            (
                "app/build.gradle.kts",
                r#"dependencies { implementation(project(":a")) }"#,
            ),
            ("a/src/main/kotlin/A.kt", "package pa\nfun icon(x: Int) {}\n"),
            ("b/src/main/kotlin/B.kt", "package pb\nfun icon(x: Int) {}\n"),
            (
                "app/src/main/kotlin/Use.kt",
                "package pu\nimport pa.icon\nfun u() { icon(1) }\n",
            ),
        ],
    );
    let mut repo = kotlin::Repo::new(tmp.path());
    let res = kotlin::recommend(&mut repo, "icon", None);
    let groups = res.groups.as_ref().unwrap();
    assert_eq!(
        groups.get("a/src/main/kotlin/A.kt").unwrap(),
        &vec!["app/src/main/kotlin/Use.kt".to_string()]
    );
    assert!(groups.get("b/src/main/kotlin/B.kt").unwrap().is_empty());
    assert!(kotlin::render(&res, false).contains("sees the declaration in a/src/main/kotlin/A.kt"));
}

#[test]
fn kotlin_unrelated_module_stays_demoted() {
    let tmp = TempDir::new().unwrap();
    setup_kotlin_project(tmp.path(), "", &[]);
    assert!(!kotlin_tier1(tmp.path(), "Card", "core/ui/src/main/kotlin/Card.kt")
        .contains("feature/home/src/main/kotlin/Home.kt"));
}

#[test]
fn ts_workspace_package_exports() {
    let tmp = TempDir::new().unwrap();
    let ui_pkg = json!({ "name": "@acme/ui", "exports": { "./button": "./src/button.tsx" } }).to_string();
    let web_pkg = json!({ "name": "web" }).to_string();
    write_files(
        tmp.path(),
        &[
            ("packages/ui/package.json", &ui_pkg),
            (
                "packages/ui/src/button.tsx",
                "export function Button(p: {a: string}) { return null }\n",
            ),
            ("apps/web/package.json", &web_pkg),
            (
                "apps/web/page.tsx",
                "import { Button } from \"@acme/ui/button\";\nexport const x = <Button a=\"1\" />;\n",
            ),
        ],
    );
    assert!(ts_tier1(tmp.path(), "Button", "packages/ui/src/button.tsx")
        .contains("apps/web/page.tsx"));
}

#[test]
fn ts_tsconfig_extends_paths() {
    let tmp = TempDir::new().unwrap();
    let base_cfg = json!({
        "compilerOptions": { "baseUrl": ".", "paths": { "~/*": ["../app/src/*"] } }
    })
    .to_string();
    let app_cfg = json!({ "extends": "../tooling/base.json" }).to_string();
    write_files(
        tmp.path(),
        &[
            ("tooling/base.json", &base_cfg),
            ("app/tsconfig.json", &app_cfg),
            ("app/src/lib.ts", "export function load(id: string) { return id }\n"),
            ("app/src/use.ts", "import { load } from \"~/lib\";\nload(\"1\");\n"),
        ],
    );
    assert!(ts_tier1(tmp.path(), "load", "app/src/lib.ts").contains("app/src/use.ts"));
}

#[test]
fn ts_flow_lists_same_folder_importers_without_the_name() {
    let tmp = TempDir::new().unwrap();
    write_files(
        tmp.path(),
        &[
            ("tsconfig.json", "{}"),
            ("src/lib.ts", "export function save(id: string) { return id }\n"),
            (
                "src/svc/store.ts",
                "import { save } from \"../lib\";\nexport const put = () => save(\"1\");\n",
            ),
            (
                "src/svc/runtime.ts",
                "import { put } from \"./store\";\nexport const run = () => put();\n",
            ),
        ],
    );
    let mut repo = typescript::Repo::new(tmp.path());
    let decls = BTreeSet::from(["src/lib.ts".to_string()]);
    let res = typescript::recommend(&mut repo, "save", Some(&decls));
    assert_eq!(res.flow, vec!["src/svc/runtime.ts".to_string()]);
    assert!(typescript::render(&res, false, true).contains("src/svc/runtime.ts"));
    assert!(!typescript::render(&res, false, false).contains("src/svc/runtime.ts"));
}

#[test]
fn ts_js_extension_specifier() {
    let tmp = TempDir::new().unwrap();
    write_files(
        tmp.path(),
        &[
            ("tsconfig.json", "{}"),
            ("src/lib.ts", "export function load(id: string) { return id }\n"),
            ("src/use.ts", "import { load } from \"./lib.js\";\nload(\"1\");\n"),
        ],
    );
    assert!(ts_tier1(tmp.path(), "load", "src/lib.ts").contains("src/use.ts"));
}

#[test]
fn install_updates_agents_md_idempotently() {
    let tmp = TempDir::new().unwrap();
    let agents_path = tmp.path().join("AGENTS.md");
    fs::write(&agents_path, "# Project\n").unwrap();
    install::update_agents_md(&agents_path).unwrap();
    let once = fs::read_to_string(&agents_path).unwrap();
    assert!(once.contains("<!-- moru-narrow:begin -->"));
    install::update_agents_md(&agents_path).unwrap();
    let twice = fs::read_to_string(&agents_path).unwrap();
    assert_eq!(once, twice);
}

#[test]
fn python_imports_distinguish_real_caller_from_unrelated() {
    let tmp = TempDir::new().unwrap();
    write_files(
        tmp.path(),
        &[
            ("pyproject.toml", "[project]\nname = \"demo\"\n"),
            ("scripts/auth.py", "def verify(token: str) -> bool:\n    return bool(token)\n"),
            ("scripts/app.py", "from auth import verify\ndef run():\n    return verify('ok')\n"),
            ("scripts/other.py", "def unrelated():\n    return verify('nope')\n"),
        ],
    );
    let mut repo = python::Repo::new(tmp.path());
    let decls = BTreeSet::from(["scripts/auth.py".to_string()]);
    let res = python::recommend(&mut repo, "verify", Some(&decls));
    assert!(res.tiers[0].contains_key("scripts/app.py"));
    assert!(!res.tiers[0].contains_key("scripts/other.py"));
}

#[test]
fn rust_use_distinguishes_real_caller_from_unrelated() {
    let tmp = TempDir::new().unwrap();
    write_files(
        tmp.path(),
        &[
            ("Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n"),
            ("src/parser.rs", "pub fn parse_item(s: &str) -> usize { s.len() }\n"),
            ("src/main.rs", "mod parser;\nuse crate::parser::parse_item;\nfn main() { let _ = parse_item(\"a\"); }\n"),
            ("src/other.rs", "// parse_item comment only\n"),
        ],
    );
    let mut repo = rust_lang::Repo::new(tmp.path());
    let decls = BTreeSet::from(["src/parser.rs".to_string()]);
    let res = rust_lang::recommend(&mut repo, "parse_item", Some(&decls));
    assert!(res.tiers[0].contains_key("src/main.rs"));
    assert!(!res.tiers[0].contains_key("src/other.rs"));
}
