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
    probe.contains("/tests/")
        || probe.contains("/test/")
        || fname.starts_with("test_")
        || fname.ends_with("_test.py")
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

fn module_stems(rel: &str) -> BTreeSet<String> {
    let mut stems = BTreeSet::new();
    let trimmed = rel.trim_end_matches(".py");
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    if let Some(&last) = parts.last() {
        if last == "__init__" && parts.len() >= 2 {
            stems.insert(parts[parts.len() - 2].to_string());
        } else {
            stems.insert(last.to_string());
        }
    }
    if parts.len() >= 2 {
        let dotted = parts.join(".");
        stems.insert(dotted.clone());
        if let Some(stripped) = dotted.strip_prefix("scripts.") {
            stems.insert(stripped.to_string());
        }
        if let Some(stripped) = dotted.strip_prefix("src.") {
            stems.insert(stripped.to_string());
        }
    }
    stems
}

fn imports_any_stem(src: &str, stems: &BTreeSet<String>) -> bool {
    let from_re = Regex::new(r"(?m)^\s*from\s+\.*([\w.]+)\s+import\b").unwrap();
    let imp_re = Regex::new(r"(?m)^\s*import\s+([\w.,\s]+)").unwrap();
    for caps in from_re.captures_iter(src) {
        let mod_path = &caps[1];
        if stems.contains(mod_path)
            || mod_path
                .rsplit('.')
                .next()
                .is_some_and(|leaf| stems.contains(leaf))
        {
            return true;
        }
    }
    for caps in imp_re.captures_iter(src) {
        for item in caps[1].split(',') {
            let name = item.split_whitespace().next().unwrap_or("");
            if stems.contains(name)
                || name
                    .rsplit('.')
                    .next()
                    .is_some_and(|leaf| stems.contains(leaf))
            {
                return true;
            }
        }
    }
    false
}

fn is_string_only(s: &str, name: &str) -> bool {
    let is_q = |c: char| matches!(c, '\'' | '"');
    let is_w = |c: char| c.is_alphanumeric() || c == '_';
    let mut quoted = false;
    let mut unquoted = false;
    let mut start = 0;
    while let Some(pos) = s[start..].find(name) {
        let idx = start + pos;
        let prev = s[..idx].chars().next_back();
        let next = s[idx + name.len()..].chars().next();
        if prev.is_some_and(is_q) && next.is_some_and(is_q) {
            quoted = true;
        }
        if !prev.is_some_and(|c| is_w(c) || is_q(c)) && !next.is_some_and(|c| is_w(c) || is_q(c)) {
            unquoted = true;
        }
        start = idx + 1;
        while start < s.len() && !s.is_char_boundary(start) {
            start += 1;
        }
    }
    quoted && !unquoted
}

pub fn role(line: &str, name: &str) -> &'static str {
    let s = line.trim();
    if s.starts_with('#') {
        return "comment";
    }
    let esc = regex::escape(name);
    let decl_re =
        Regex::new(&format!(r"^(?:async\s+)?(?:def|class)\s+{esc}\b")).unwrap();
    if decl_re.is_match(s) {
        return "declaration";
    }
    if s.starts_with("from ") || s.starts_with("import ") {
        let word_re = Regex::new(&format!(r"\b{esc}\b")).unwrap();
        return if word_re.is_match(s) {
            "import"
        } else {
            "import-other"
        };
    }
    if is_string_only(s, name) {
        return "string";
    }
    let kw_re = Regex::new(&format!(r"\b{esc}\s*=([^=]|$)")).unwrap();
    if kw_re.is_match(s) && !s.contains(&format!("{name}(")) {
        return "named-arg";
    }
    "use"
}

fn rg_hits(root: &Path, name: &str) -> BTreeMap<String, Vec<(usize, String)>> {
    let out = Command::new("rg")
        .args([
            "-nw",
            "-g", "*.py",
            "-g", "!**/__pycache__/**",
            "-g", "!**/.venv/**",
            "-g", "!**/venv/**",
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
    let line_re = Regex::new(r"^\./(.+?\.py):(\d+):(.*)$").unwrap();
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
    let mut dstems: BTreeSet<String> = BTreeSet::new();
    for f in &dfiles {
        dstems.extend(module_stems(f));
    }

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

        let near = dfiles.contains(rel) || imports_any_stem(&src, &dstems);
        let tier_idx = if !strong.is_empty() && near {
            0
        } else if !strong.is_empty() {
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
        "not found in non-test .py".to_string()
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
        (2, t2, "TIER 2: strong match without an import of the declaring module"),
        (3, t3, "TIER 3: weak match (comment, string, keyword arg)"),
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
