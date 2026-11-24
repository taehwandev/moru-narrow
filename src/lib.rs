pub mod gradleplugins;
pub mod install;
pub mod kotlin;
pub mod python;
pub mod rust_lang;
pub mod tsresolve;
pub mod typescript;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub fn detect_lang(repo: &Path) -> Result<&'static str, String> {
    if repo.join("tsconfig.json").exists() {
        return Ok("ts");
    }
    let has_root_gradle = fs::read_dir(repo)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let p = e.path();
            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname.starts_with("settings.gradle") || fname.starts_with("build.gradle") {
                return true;
            }
            if p.is_dir() {
                return fs::read_dir(&p)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|sub| {
                        sub.file_name()
                            .to_str()
                            .is_some_and(|sn| sn.starts_with("build.gradle"))
                    });
            }
            false
        });
    if has_root_gradle {
        return Ok("kotlin");
    }
    if repo.join("Cargo.toml").exists() {
        return Ok("rust");
    }
    if repo.join("pyproject.toml").exists()
        || repo.join("setup.py").exists()
        || repo.join("requirements.txt").exists()
        || repo.join("scripts").is_dir()
    {
        return Ok("python");
    }
    if repo.join("package.json").exists() {
        return Ok("ts");
    }
    Err("moru-narrow: cannot detect the language (looked for tsconfig.json, Gradle, Cargo.toml, or Python project files); pass --lang kotlin|ts|python|rust".to_string())
}

pub fn run_cli(argv: &[String]) -> Result<(), String> {
    if argv.first().map(String::as_str) == Some("--install") {
        return install::run_install(&argv[1..]);
    }

    let mut name: Option<String> = None;
    let mut repo = ".".to_string();
    let mut lang: Option<String> = None;
    let mut decls: BTreeSet<String> = BTreeSet::new();
    let mut show_all = false;
    let mut show_flow = false;
    let mut as_json = false;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                println!(
                    "moru-narrow <Name> [--repo DIR] [--decl FILE] [--all] [--flow] [--json] [--lang kotlin|ts|python|rust]\n\
                     moru-narrow --install [--target claude|agents|both] [--project] [--agents-md] [--force]"
                );
                return Ok(());
            }
            "-V" | "--version" => {
                println!("moru-narrow {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--repo" => {
                i += 1;
                repo = argv
                    .get(i)
                    .ok_or_else(|| "--repo requires a directory".to_string())?
                    .clone();
            }
            "--lang" => {
                i += 1;
                let v = argv
                    .get(i)
                    .ok_or_else(|| "--lang requires kotlin, ts, python, or rust".to_string())?;
                if !matches!(v.as_str(), "kotlin" | "ts" | "python" | "rust") {
                    return Err("--lang must be kotlin, ts, python, or rust".to_string());
                }
                lang = Some(v.clone());
            }
            "--decl" => {
                i += 1;
                let d = argv
                    .get(i)
                    .ok_or_else(|| "--decl requires a file path".to_string())?;
                decls.insert(d.clone());
            }
            "--all" => show_all = true,
            "--flow" => show_flow = true,
            "--json" => as_json = true,
            arg if !arg.starts_with('-') && name.is_none() => {
                name = Some(arg.to_string());
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
        i += 1;
    }

    let Some(symbol) = name else {
        return Err(
            "usage: moru-narrow <Name> [--repo DIR] [--decl FILE] [--all] [--flow] [--json] [--lang kotlin|ts|python|rust]"
                .to_string(),
        );
    };

    let repo_path = Path::new(&repo);
    let resolved_lang = match lang.as_deref() {
        Some("kotlin") => "kotlin",
        Some("ts") => "ts",
        Some("python") => "python",
        Some("rust") => "rust",
        _ => detect_lang(repo_path)?,
    };

    let decl_opt = if decls.is_empty() { None } else { Some(&decls) };

    match resolved_lang {
        "kotlin" => {
            let mut r = kotlin::Repo::new(repo_path);
            let res = kotlin::recommend(&mut r, &symbol, decl_opt);
            if as_json {
                println!("{}", serde_json::to_string(&res.to_json()).unwrap_or_else(|_| "{}".into()));
            } else {
                println!("{}", kotlin::render(&res, show_all));
            }
        }
        "ts" => {
            let mut r = typescript::Repo::new(repo_path);
            let res = typescript::recommend(&mut r, &symbol, decl_opt);
            if as_json {
                println!("{}", serde_json::to_string(&res.to_json()).unwrap_or_else(|_| "{}".into()));
            } else {
                println!("{}", typescript::render(&res, show_all, show_flow));
            }
        }
        "python" => {
            let mut r = python::Repo::new(repo_path);
            let res = python::recommend(&mut r, &symbol, decl_opt);
            if as_json {
                println!("{}", serde_json::to_string(&res.to_json()).unwrap_or_else(|_| "{}".into()));
            } else {
                println!("{}", python::render(&res, show_all));
            }
        }
        "rust" => {
            let mut r = rust_lang::Repo::new(repo_path);
            let res = rust_lang::recommend(&mut r, &symbol, decl_opt);
            if as_json {
                println!("{}", serde_json::to_string(&res.to_json()).unwrap_or_else(|_| "{}".into()));
            } else {
                println!("{}", rust_lang::render(&res, show_all));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
