use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::gradleplugins::convention_deps;

const STRONG: &[&str] = &["use", "override", "import"];

#[derive(Debug, Clone, Serialize)]
pub struct HitRow {
    pub line: usize,
    pub to: usize,
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct RecommendResult {
    pub name: String,
    pub declarations: Vec<(String, usize)>,
    pub tiers: [BTreeMap<String, Vec<HitRow>>; 3],
    pub rg_files: usize,
    pub rg_lines: usize,
    pub groups: Option<BTreeMap<String, Vec<String>>>,
}

impl RecommendResult {
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("name".into(), json!(self.name));
        let decls: Vec<Value> = self
            .declarations
            .iter()
            .map(|(f, ln)| json!([f, ln]))
            .collect();
        obj.insert("declarations".into(), Value::Array(decls));
        obj.insert(
            "tiers".into(),
            json!({
                "1": self.tiers[0],
                "2": self.tiers[1],
                "3": self.tiers[2],
            }),
        );
        obj.insert("rg_files".into(), json!(self.rg_files));
        obj.insert("rg_lines".into(), json!(self.rg_lines));
        if let Some(groups) = &self.groups {
            obj.insert("groups".into(), json!(groups));
        }
        Value::Object(obj)
    }
}

pub fn is_test(rel: &str) -> bool {
    let test_re = Regex::new(r"/(src/)?\w*[Tt]est\w*/").unwrap();
    let probe = format!("/{rel}");
    test_re.is_match(&probe) || rel.ends_with("Test.kt")
}

fn rel_posix(root: &Path, target: &Path) -> String {
    if target == root {
        return ".".to_string();
    }
    match target.strip_prefix(root) {
        Ok(p) => {
            let s = p
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if s.is_empty() {
                ".".to_string()
            } else {
                s
            }
        }
        Err(_) => ".".to_string(),
    }
}

fn collect_gradle_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let slash = p.to_string_lossy();
        if slash.contains("/build/") || slash.ends_with("/build") {
            continue;
        }
        if p.is_dir() {
            collect_gradle_files(&p, out);
        } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name == "build.gradle.kts" || name == "build.gradle" {
                out.push(p);
            }
        }
    }
}

pub struct Repo {
    pub root: PathBuf,
    src: HashMap<String, String>,
    pub build: BTreeMap<String, BTreeSet<String>>,
    pub reach: BTreeMap<String, BTreeSet<String>>,
}

impl Repo {
    pub fn new(root: &Path) -> Self {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut gradle_files = Vec::new();
        collect_gradle_files(&root, &mut gradle_files);

        let proj_re = Regex::new(r#"project\((?:path\s*=\s*)?["'](:[\w:.-]+)["']"#).unwrap();
        let acc_re = Regex::new(r"\bprojects\.([A-Za-z][\w.]*)").unwrap();

        let mut build: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut accessors: Vec<(String, PathBuf, Vec<String>)> = Vec::new();
        let mut module_texts: BTreeMap<String, (PathBuf, String)> = BTreeMap::new();

        for g in gradle_files {
            let Some(d) = g.parent() else { continue };
            let base = Self::settings_root(&root, d);
            let txt = fs::read_to_string(&g).unwrap_or_default();
            let mut deps = BTreeSet::new();
            for caps in proj_re.captures_iter(&txt) {
                let rel_mod = caps[1].trim_matches(':').replace(':', "/");
                let dep_path = base.join(rel_mod);
                deps.insert(rel_posix(&root, &dep_path));
            }
            let mod_key = rel_posix(&root, d);
            let accs: Vec<String> = acc_re
                .captures_iter(&txt)
                .map(|c| c[1].to_string())
                .collect();
            build.insert(mod_key.clone(), deps);
            accessors.push((mod_key.clone(), base.clone(), accs));
            module_texts.insert(mod_key, (base, txt));
        }

        let all_mods: Vec<String> = build.keys().cloned().collect();
        for (mod_key, base, accs) in accessors {
            let mut resolved_accs = BTreeSet::new();
            for acc in accs {
                if let Some(m) = Self::accessor_module(&root, &base, &acc, &all_mods) {
                    resolved_accs.insert(m);
                }
            }
            if let Some(set) = build.get_mut(&mod_key) {
                set.extend(resolved_accs);
            }
        }

        let raw_builds: BTreeMap<String, String> = module_texts
            .iter()
            .map(|(k, (_, t))| (k.clone(), t.clone()))
            .collect();
        let (per, resolved) = convention_deps(&root, &raw_builds);
        let ambient = if resolved {
            BTreeSet::new()
        } else {
            Self::ambient_deps(&root, &build)
        };

        for (mod_key, (base, _)) in &module_texts {
            let mut extra = BTreeSet::new();
            if let Some(p_set) = per.get(mod_key) {
                for p in p_set {
                    let rel_mod = p.trim_matches(':').replace(':', "/");
                    extra.insert(rel_posix(&root, &base.join(rel_mod)));
                }
            }
            extra.extend(ambient.iter().cloned());
            extra.remove(mod_key);
            if let Some(set) = build.get_mut(mod_key) {
                set.extend(extra);
            }
        }

        let mut reach = BTreeMap::new();
        for m in build.keys() {
            reach.insert(m.clone(), Self::closure(m, &build));
        }

        Self {
            root,
            src: HashMap::new(),
            build,
            reach,
        }
    }

    fn ambient_deps(root: &Path, build: &BTreeMap<String, BTreeSet<String>>) -> BTreeSet<String> {
        let mut deps = BTreeSet::new();
        let proj_re = Regex::new(r#"project\(["'](:[\w:.-]+)["']\)"#).unwrap();
        fn visit(dir: &Path, build: &BTreeMap<String, BTreeSet<String>>, proj_re: &Regex, out: &mut BTreeSet<String>) {
            let Ok(entries) = fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                let s = p.to_string_lossy();
                if s.contains("/build/") || s.ends_with("/build") {
                    continue;
                }
                if p.is_dir() {
                    visit(&p, build, proj_re, out);
                } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".kt") || name.ends_with(".kts") {
                        if let Ok(txt) = fs::read_to_string(&p) {
                            for caps in proj_re.captures_iter(&txt) {
                                let m = caps[1].trim_matches(':').replace(':', "/");
                                if build.contains_key(&m) {
                                    out.insert(m);
                                }
                            }
                        }
                    }
                }
            }
        }
        for sub in &["build-logic", "buildSrc"] {
            visit(&root.join(sub), build, &proj_re, &mut deps);
        }
        deps
    }

    fn norm(s: &str) -> String {
        s.chars()
            .filter(|c| !matches!(c, '-' | '_' | '.'))
            .flat_map(char::to_lowercase)
            .collect()
    }

    fn accessor_module(root: &Path, base: &Path, acc: &str, all_mods: &[String]) -> Option<String> {
        let toks: Vec<&str> = acc.split('.').collect();
        let base_rel = rel_posix(root, base);
        for n in (1..=toks.len()).rev() {
            let want: Vec<String> = toks[..n].iter().map(|t| Self::norm(t)).collect();
            for m in all_mods {
                let parts: Vec<String> = if base_rel != "." {
                    let prefix = format!("{base_rel}/");
                    let Some(stripped) = m.strip_prefix(&prefix) else {
                        continue;
                    };
                    stripped.split('/').map(Self::norm).collect()
                } else if m == "." {
                    continue;
                } else {
                    m.split('/').map(Self::norm).collect()
                };
                if parts == want {
                    return Some(m.clone());
                }
            }
        }
        None
    }

    fn settings_root(root: &Path, d: &Path) -> PathBuf {
        let mut cur = d;
        loop {
            if cur.join("settings.gradle.kts").exists() || cur.join("settings.gradle").exists() {
                return cur.to_path_buf();
            }
            if cur == root {
                return root.to_path_buf();
            }
            match cur.parent() {
                Some(p) if p != cur => cur = p,
                _ => return root.to_path_buf(),
            }
        }
    }

    fn closure(start: &str, build: &BTreeMap<String, BTreeSet<String>>) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![start.to_string()];
        while let Some(cur) = stack.pop() {
            if let Some(neighbors) = build.get(&cur) {
                for y in neighbors {
                    if seen.insert(y.clone()) {
                        stack.push(y.clone());
                    }
                }
            }
        }
        seen
    }

    pub fn text(&mut self, rel: &str) -> String {
        if let Some(t) = self.src.get(rel) {
            return t.clone();
        }
        let content = fs::read_to_string(self.root.join(rel)).unwrap_or_default();
        self.src.insert(rel.to_string(), content.clone());
        content
    }

    pub fn module_of(&self, rel: &str) -> Option<String> {
        let mut cur = Path::new(rel).parent();
        while let Some(p) = cur {
            let s = p
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if s.is_empty() {
                break;
            }
            if self.build.contains_key(&s) {
                return Some(s);
            }
            cur = p.parent();
        }
        if self.build.contains_key(".") {
            Some(".".to_string())
        } else {
            None
        }
    }

    pub fn package_of(&mut self, rel: &str) -> String {
        let txt = self.text(rel);
        let pkg_re = Regex::new(r"(?m)^package\s+([\w.]+)").unwrap();
        pkg_re
            .captures(&txt)
            .map(|c| c[1].to_string())
            .unwrap_or_default()
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn matches_kotlin_use(s: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = s[start..].find(name) {
        let idx = start + pos;
        let prev = s[..idx].chars().next_back();
        let prev_ok = match prev {
            None => true,
            Some(c) => !is_word_char(c) && c != '"' && c != '\'',
        };
        if prev_ok {
            let after = &s[idx + name.len()..];
            let next_non_ws = after.chars().find(|c| !c.is_whitespace());
            let after_ok = match next_non_ws {
                None => true,
                Some('(' | '{' | ')' | ',') => true,
                Some(c) if is_word_char(c) => false,
                _ => false,
            };
            // Ensure immediately following char isn't part of a longer identifier (e.g. `CardView (`)
            let immediate = after.chars().next();
            let boundary_ok = !immediate.is_some_and(is_word_char);
            if boundary_ok && after_ok {
                return true;
            }
        }
        start = idx + 1;
        while start < s.len() && !s.is_char_boundary(start) {
            start += 1;
        }
    }
    false
}

pub fn role(line: &str, name: &str) -> &'static str {
    let s = line.trim();
    if s.starts_with("//") || s.starts_with('*') || s.starts_with("/*") {
        return "comment";
    }
    if s.starts_with("import ") || s.starts_with("import\t") {
        return if s.ends_with(&format!(".{name}")) || s.contains(" as ") {
            "import"
        } else {
            "import-other"
        };
    }
    let esc = regex::escape(name);
    let prop_re = Regex::new(&format!(r"\b(val|var)\s+{esc}\b")).unwrap();
    let call_re = Regex::new(&format!(r"\b{esc}\s*\(")).unwrap();
    if prop_re.is_match(s) && !call_re.is_match(s) {
        return "property";
    }
    let fun_decl = Regex::new(&format!(r"\bfun\s+(?:<[^>]*>\s*)?(?:[\w.<>?]+\.)?{esc}\s*[(<]")).unwrap();
    let type_decl = Regex::new(&format!(r"\b(?:class|interface|object|typealias)\s+{esc}\b")).unwrap();
    if fun_decl.is_match(s) || type_decl.is_match(s) {
        return if s.contains("override") {
            "override"
        } else {
            "declaration"
        };
    }
    if matches_kotlin_use(s, name) {
        return "use";
    }
    let named_arg = Regex::new(&format!(r"\b{esc}\s*=")).unwrap();
    if named_arg.is_match(s) && !s.contains(&format!("{name} ==")) && !s.contains(&format!("{name}==")) {
        return "named-arg";
    }
    "other"
}

pub fn call_range(lines: &[&str], line_1based: usize) -> (usize, usize) {
    let mut depth = 0i32;
    let mut seen = false;
    let end_limit = lines.len().min(line_1based + 39);
    for (j, &ln) in lines.iter().enumerate().take(end_limit).skip(line_1based.saturating_sub(1)) {
        for ch in ln.chars() {
            if ch == '(' {
                depth += 1;
                seen = true;
            } else if ch == ')' {
                depth -= 1;
            }
        }
        if seen && depth <= 0 {
            return (line_1based, j + 1);
        }
        if !seen && j > line_1based.saturating_sub(1) {
            break;
        }
    }
    (line_1based, line_1based)
}

fn rg_hits(root: &Path, name: &str) -> BTreeMap<String, Vec<(usize, String)>> {
    let out = Command::new("rg")
        .args(["-nw", "--glob", "*.kt", "--", name, "."])
        .current_dir(root)
        .output();
    let stdout = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    };
    let line_re = Regex::new(r"^\./(.+?\.kt):(\d+):(.*)$").unwrap();
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
    let dmods: BTreeSet<Option<String>> = dfiles.iter().map(|f| repo.module_of(f)).collect();
    let mut dpkgs: BTreeSet<String> = BTreeSet::new();
    for f in &dfiles {
        dpkgs.insert(repo.package_of(f));
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

        if dfiles.contains(rel) && (decl_files.is_some() || strong.is_empty()) {
            continue;
        }

        let m = repo.module_of(rel);
        let in_mod = dmods.contains(&m)
            || m.as_ref().is_some_and(|mod_name| {
                repo.reach.get(mod_name).is_some_and(|rset| {
                    dmods
                        .iter()
                        .flatten()
                        .any(|dm| rset.contains(dm))
                })
            });

        let pk = repo.package_of(rel);
        let near = dpkgs.contains(&pk)
            || dpkgs.iter().filter(|p| !p.is_empty()).any(|p| {
                let pat = format!(r"(?m)^import\s+{}[.\w]*", regex::escape(p));
                Regex::new(&pat).is_ok_and(|re| re.is_match(&src))
            });

        let tier_idx = if !strong.is_empty() && in_mod && near {
            0
        } else if !strong.is_empty() && in_mod {
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

    let groups = if dfiles.len() > 1 {
        Some(compute_groups(repo, &dfiles, &tiers[0]))
    } else {
        None
    };

    let rg_lines = byfile.values().map(Vec::len).sum();
    RecommendResult {
        name: name.to_string(),
        declarations: decls,
        tiers,
        rg_files: byfile.len(),
        rg_lines,
        groups,
    }
}

fn compute_groups(
    repo: &mut Repo,
    dfiles: &BTreeSet<String>,
    t1: &BTreeMap<String, Vec<HitRow>>,
) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for d in dfiles {
        let dm = repo.module_of(d);
        let dp = repo.package_of(d);
        let import_re = if dp.is_empty() {
            None
        } else {
            Regex::new(&format!(r"(?m)^import\s+{}[.\w]*", regex::escape(&dp))).ok()
        };
        let mut hit = Vec::new();
        for rel in t1.keys() {
            let m = repo.module_of(rel);
            let src = repo.text(rel);
            let mod_ok = m == dm
                || match (&m, &dm) {
                    (Some(m_name), Some(dm_name)) => repo
                        .reach
                        .get(m_name)
                        .is_some_and(|r| r.contains(dm_name)),
                    _ => false,
                };
            let pkg_ok = repo.package_of(rel) == dp
                || import_re.as_ref().is_some_and(|re| re.is_match(&src));
            if rel == d || (mod_ok && pkg_ok) {
                hit.push(rel.clone());
            }
        }
        out.insert(d.clone(), hit);
    }
    out
}

fn format_tier1_file(rel: &str, rows: &[HitRow]) -> Vec<String> {
    let mut out = vec![rel.to_string()];
    for r in rows {
        let range = if r.to > r.line {
            format!("{}-{}", r.line, r.to)
        } else {
            format!("{}", r.line)
        };
        out.push(format!("  {range}: {}", r.text));
    }
    out
}

pub fn render(res: &RecommendResult, show_all: bool) -> String {
    let decl_str = if res.declarations.is_empty() {
        "not found in non-test .kt".to_string()
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

    if let Some(groups) = &res.groups {
        let mut seen = BTreeSet::new();
        for (d, rels) in groups {
            o.push(format!("-- sees the declaration in {d}:"));
            for rel in rels {
                if let Some(rows) = t1.get(rel) {
                    o.extend(format_tier1_file(rel, rows));
                    seen.insert(rel.clone());
                }
            }
        }
        let rest: Vec<&String> = t1.keys().filter(|k| !seen.contains(*k)).collect();
        if !rest.is_empty() {
            o.push("-- cannot tell which declaration:".to_string());
            for rel in rest {
                if let Some(rows) = t1.get(rel) {
                    o.extend(format_tier1_file(rel, rows));
                }
            }
        }
    } else {
        for (rel, rows) in t1 {
            o.extend(format_tier1_file(rel, rows));
        }
    }

    for (k, t, label) in [
        (2, t2, "TIER 2: strong match, outside the declaring package or its imports"),
        (3, t3, "TIER 3: weak match or unreachable module"),
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
