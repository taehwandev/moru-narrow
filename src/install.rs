use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

pub const RULE: &str = "\
When a task changes a Kotlin (Gradle), TypeScript, Python, or Rust function/constructor/method contract \
(parameter, return type, visibility, deletion, or split) or inspects a symbol's real call-site blast radius, \
run `moru-narrow <Name> --repo <project root>` once and inspect/edit the call sites listed under TIER 1. \
Do not re-run `rg`/`grep` for the same name afterwards; use `--all` only if TIER 1 looks incomplete. \
If the output prints a WARNING about multiple declarations, rerun with `--decl <file>` for the one you change. \
If it notes a React component, read the props type first and verify JSX uses. \
Use plain `rg` for simple text/config searches, pure renames, or unsupported languages.";

const BEGIN: &str = "<!-- moru-narrow:begin -->";
const END: &str = "<!-- moru-narrow:end -->";

fn skill_content() -> String {
    format!(
        "---\n\
name: moru-narrow\n\
description: Rank ripgrep hits into real Tier 1 call sites for Kotlin (Gradle) and TypeScript contract changes, refactors, and blast-radius checks.\n\
---\n\n\
# moru-narrow\n\n\
{RULE}\n\n\
```bash\n\
moru-narrow getGlobalTheme --repo .     # TIER 1 = real callers; tier 2/3 hidden behind --all\n\
```\n\n\
Post-processes `rg -nw`, keeping every hit reachable via `--all`. Native Rust binary, no index, no network, no model calls.\n"
    )
}

pub fn managed_block() -> String {
    format!("{BEGIN}\n## moru-narrow\n\n{RULE}\n{END}\n")
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn skill_dirs(target: &str, project: bool) -> Vec<PathBuf> {
    let base = if project { PathBuf::from(".") } else { home_dir() };
    let roots: &[&str] = match target {
        "claude" => &[".claude"],
        "agents" => &[".agents"],
        _ => &[".claude", ".agents"],
    };
    roots
        .iter()
        .map(|d| base.join(d).join("skills").join("moru-narrow"))
        .collect()
}

fn write_skill(dir: &Path, force: bool) -> Result<String, String> {
    let f = dir.join("SKILL.md");
    if f.exists() && !force {
        return Ok(format!("kept     {} (exists, use --force to overwrite)", f.display()));
    }
    fs::create_dir_all(dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    fs::write(&f, skill_content()).map_err(|e| format!("failed to write {}: {e}", f.display()))?;
    Ok(format!("wrote    {}", f.display()))
}

pub fn update_agents_md(path: &Path) -> Result<String, String> {
    let text = if path.exists() {
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?
    } else {
        String::new()
    };
    let block = managed_block();
    let pat = Regex::new(&format!(
        r"(?s){}.*?{}\n?",
        regex::escape(BEGIN),
        regex::escape(END)
    ))
    .unwrap();

    let new_text = if pat.is_match(&text) {
        pat.replace(&text, block.as_str()).into_owned()
    } else if text.is_empty() {
        block
    } else if text.ends_with("\n\n") {
        format!("{text}{block}")
    } else if text.ends_with('\n') {
        format!("{text}\n{block}")
    } else {
        format!("{text}\n\n{block}")
    };

    fs::write(path, new_text).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(format!("updated  {}", path.display()))
}

pub fn run_install(args: &[String]) -> Result<(), String> {
    let mut target = "both".to_string();
    let mut project = false;
    let mut agents_md = false;
    let mut force = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or_else(|| "--target requires claude, agents, or both".to_string())?;
                if !matches!(val.as_str(), "claude" | "agents" | "both") {
                    return Err("--target must be claude, agents, or both".to_string());
                }
                target = val.clone();
            }
            "--project" => project = true,
            "--agents-md" => agents_md = true,
            "--force" => force = true,
            "-h" | "--help" => {
                println!(
                    "Usage: moru-narrow --install [--target claude|agents|both] [--project] [--agents-md] [--force]"
                );
                return Ok(());
            }
            other => return Err(format!("unrecognized install argument: {other}")),
        }
        i += 1;
    }

    if agents_md {
        println!("{}", update_agents_md(Path::new("AGENTS.md"))?);
    } else {
        for d in skill_dirs(&target, project) {
            println!("{}", write_skill(&d, force)?);
        }
    }
    Ok(())
}
