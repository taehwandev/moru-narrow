use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

use crate::kotlin::{call_range, HitRow, RecommendResult};

const STRONG: &[&str] = &["use", "import"];

pub fn is_test(rel: &str) -> bool {
    let probe = format!("/{rel}");
    let fname = Path::new(rel)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    probe.contains("/tests/") || fname.ends_with("_tests.rs") || fname.ends_with("_test.rs")
}

pub struct Repo {
    pub root: PathBuf,
    src: HashMap<String, String>,
}

impl Repo {
    pub fn new(root: &Path) -> Self {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        Self {
            root,
            src: HashMap::new(),
        }
    }

    pub fn text(&mut self, rel: &str) -> String {
        if let Some(t) = self.src.get(rel) {
            return t.clone();
        }
        let content = fs::read_to_string(self.root.join(rel)).unwrap_or_default();
        self.src.insert(rel.to_string(), content.clone());
        content
    }
}

fn crate_root_of(rel: &str) -> String {
    if let Some((prefix, _)) = rel.split_once("/src/") {
        prefix.to_string()
    } else {
        ".".to_string()
    }
}

fn mod_name_of(rel: &str) -> Option<String> {
    let stem = Path::new(rel).file_stem()?.to_str()?;
    if matches!(stem, "mod" | "lib" | "main") {
        Path::new(rel)
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(str::to_string)
    } else {
        Some(stem.to_string())
    }
}

fn references_mod_or_symbol(src: &str, mods: &BTreeSet<String>, name: &str) -> bool {
    let esc_name = regex::escape(name);
    let use_sym = Regex::new(&format!(r"\buse\b[^;]*\b{esc_name}\b")).unwrap();
    if use_sym.is_match(src) {
        return true;
    }
    for m in mods {
        let esc = regex::escape(m);
        let re = Regex::new(&format!(r"\b(?:use\b[^;]*\b{esc}\b|mod\s+{esc}\b|{esc}::)")).unwrap();
        if re.is_match(src) {
            return true;
        }
    }
    false
}

pub fn role(line: &str, name: &str) -> &'static str {
    let s = line.trim();
    if s.starts_with("//") || s.starts_with('*') || s.starts_with("/*") {
        return "comment";
    }
    let esc = regex::escape(name);
    let decl_re = Regex::new(&format!(
        r"\b(?:pub(?:\([^)]*\))?\s+)?(?:const\s+|async\s+|unsafe\s+)?(?:fn|struct|enum|trait|type|const|static)\s+{esc}\b"
    ))
    .unwrap();
    if decl_re.is_match(s) {
        return "declaration";
    }
    if s.starts_with("use ") || s.starts_with("pub use ") {
        let word_re = Regex::new(&format!(r"\b{esc}\b")).unwrap();
        return if word_re.is_match(s) {
            "import"
        } else {
            "import-other"
        };
    }
    "use"
}

fn rg_hits(root: &Path, name: &str) -> BTreeMap<String, Vec<(usize, String)>> {
    let out = Command::new("rg")
        .args([
            "-nw",
            "-g", "*.rs",
            "-g", "!**/target/**",
            "--",
            name,
            ".",
        ])
        .current_dir(root)
        .output();
    let stdout = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    };
    let line_re = Regex::new(r"^\./(.+?\.rs):(\d+):(.*)$").unwrap();
    let mut byfile: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for l in stdout.lines() {
        if let Some(caps) = line_re.captures(l) {
            let rel = &caps[1];
            if !is_test(rel) {
                if let Ok(ln) = caps[2].parse::<usize>() {
                    byfile
                        .entry(rel.to_string())
                        .or_default()
                        .push((ln, caps[3].to_string()));
                }
            }
        }
    }
    byfile
}

pub fn recommend(
    repo: &mut Repo,
    name: &str,
    decl_files: Option<&BTreeSet<String>>,
) -> RecommendResult {
    let byfile = rg_hits(&repo.root, name);
    let mut decls: Vec<(String, usize)> = Vec::new();
    for (rel, hits) in &byfile {
        for (ln, tx) in hits {
            if role(tx, name) == "declaration"
                && decl_files.is_none_or(|df| df.contains(rel))
            {
                decls.push((rel.clone(), *ln));
            }
        }
    }
    if let Some(df) = decl_files {
        decls = df
            .iter()
            .map(|f| {
                let ln = byfile
                    .get(f)
                    .and_then(|hits| {
                        hits.iter()
                            .find(|(_, tx)| role(tx, name) == "declaration")
                            .map(|(l, _)| *l)
                    })
                    .unwrap_or(0);
                (f.clone(), ln)
            })
            .collect();
    }

    let dfiles: BTreeSet<String> = decls.iter().map(|(f, _)| f.clone()).collect();
    let dcrates: BTreeSet<String> = dfiles.iter().map(|f| crate_root_of(f)).collect();
    let dmods: BTreeSet<String> = dfiles.iter().filter_map(|f| mod_name_of(f)).collect();

    let mut tiers: [BTreeMap<String, Vec<HitRow>>; 3] = [
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    ];

    for (rel, hits) in &byfile {
        let src = repo.text(rel);
        let lines: Vec<&str> = src.split('\n').collect();
        let rows: Vec<(usize, &str, &'static str)> = hits
            .iter()
            .map(|(ln, tx)| (*ln, tx.as_str(), role(tx, name)))
            .collect();
        let strong: Vec<(usize, &str, &'static str)> = rows
            .iter()
            .copied()
            .filter(|(_, _, r)| STRONG.contains(r))
            .collect();

        if dfiles.contains(rel) && strong.is_empty() {
            continue;
        }

        let same_crate = dcrates.contains(&crate_root_of(rel));
        let near = dfiles.contains(rel)
            || (same_crate && references_mod_or_symbol(&src, &dmods, name))
            || references_mod_or_symbol(&src, &dmods, name);

        let tier_idx = if !strong.is_empty() && near {
            0
        } else if !strong.is_empty() && same_crate {
            1
        } else {
            2
        };

        let chosen = if strong.is_empty() { &rows } else { &strong };
        let mapped: Vec<HitRow> = chosen
            .iter()
            .map(|(ln, tx, r)| HitRow {
                line: *ln,
                to: call_range(&lines, *ln).1,
                role: (*r).to_string(),
                text: tx.trim().chars().take(160).collect(),
            })
            .collect();
        tiers[tier_idx].insert(rel.clone(), mapped);
    }

    let rg_lines = byfile.values().map(Vec::len).sum();
    RecommendResult {
        name: name.to_string(),
        declarations: decls,
        tiers,
        rg_files: byfile.len(),
        rg_lines,
        groups: None,
    }
}

pub fn render(res: &RecommendResult, show_all: bool) -> String {
    let decl_str = if res.declarations.is_empty() {
        "not found in non-test .rs".to_string()
    } else {
        res.declarations
            .iter()
            .map(|(f, ln)| format!("{f}:{ln}"))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let mut o = vec![format!("{}  declared: {decl_str}", res.name)];
    if res.declarations.len() > 1 {
        o.push(format!(
            "WARNING: {} declarations of {}; TIER 1 merges the callers of all of them. Pass --decl <FILE> for the one you change.",
            res.declarations.len(),
            res.name
        ));
    }
    let t1 = &res.tiers[0];
    let t2 = &res.tiers[1];
    let t3 = &res.tiers[2];
    o.push(format!(
        "TIER 1: check these ({} files). rg found {} files, {} lines.",
        t1.len(),
        res.rg_files,
        res.rg_lines
    ));
    for (rel, rows) in t1 {
        o.push(rel.clone());
        for r in rows {
            let range = if r.to > r.line {
                format!("{}-{}", r.line, r.to)
            } else {
                format!("{}", r.line)
            };
            o.push(format!("  {range}: {}", r.text));
        }
    }
    for (k, t, label) in [
        (2, t2, "TIER 2: strong match in same crate without explicit use/mod"),
        (3, t3, "TIER 3: weak match or other crate"),
    ] {
        if show_all {
            o.push(format!("{label} ({} files)", t.len()));
            for (rel, ls) in t {
                o.push(rel.clone());
                for r in ls.iter().take(5) {
                    o.push(format!("  {}: {}", r.line, r.text));
                }
            }
        } else {
            o.push(format!("hidden: tier {k} has {} files (use --all to list them)", t.len()));
        }
    }
    o.join("\n")
}
