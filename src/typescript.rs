use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_json::{json, Value};

use crate::kotlin::HitRow;
use crate::tsresolve::{norm_rel_posix, TsResolver};

const STRONG: &[&str] = &["use", "import"];
const EXTS: &[&str] = &[".ts", ".tsx", ".mts", ".js", ".jsx", ".mjs"];

#[derive(Debug, Clone)]
pub struct TsRecommendResult {
    pub name: String,
    pub declarations: Vec<(String, usize)>,
    pub tiers: [BTreeMap<String, Vec<HitRow>>; 3],
    pub rg_files: usize,
    pub rg_lines: usize,
    pub groups: Option<BTreeMap<String, Vec<String>>>,
    pub flow: Vec<String>,
}

impl TsRecommendResult {
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
        obj.insert("flow".into(), json!(self.flow));
        Value::Object(obj)
    }
}

pub fn is_test(rel: &str) -> bool {
    let probe = format!("/{rel}");
    probe.contains(".test.")
        || probe.contains(".spec.")
        || probe.contains("/__tests__/")
        || probe.contains("/__mocks__/")
}

pub struct Repo {
    pub root: PathBuf,
    resolver: TsResolver,
    src: HashMap<String, String>,
    imp: HashMap<String, BTreeSet<String>>,
    rex: HashMap<String, BTreeSet<String>>,
    all_files_cache: Option<Vec<String>>,
}

impl Repo {
    pub fn new(root: &Path) -> Self {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let resolver = TsResolver::new(root.clone());
        Self {
            root,
            resolver,
            src: HashMap::new(),
            imp: HashMap::new(),
            rex: HashMap::new(),
            all_files_cache: None,
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

    pub fn resolve(&mut self, frm: &str, spec: &str) -> Option<String> {
        let frm_dir = Path::new(frm)
            .parent()
            .map(|p| {
                let s = p.to_string_lossy().into_owned();
                if s.is_empty() { ".".to_string() } else { s }
            })
            .unwrap_or_else(|| ".".to_string());

        let mut base_cands: Vec<String> = Vec::new();
        if spec.starts_with('.') {
            let joined = if frm_dir == "." {
                spec.to_string()
            } else {
                format!("{frm_dir}/{spec}")
            };
            base_cands.push(norm_rel_posix(&joined));
        } else {
            for (pre, tgt) in self.resolver.paths_for(&frm_dir) {
                if let Some(rest) = spec.strip_prefix(&pre) {
                    let joined = if tgt.is_empty() {
                        rest.to_string()
                    } else {
                        format!("{tgt}/{rest}")
                    };
                    base_cands.push(norm_rel_posix(&joined));
                }
            }
            base_cands.extend(self.resolver.workspace_candidates(spec));
        }

        let js_ext_re = Regex::new(r"\.(?:m|c)?jsx?$").unwrap();
        let mut expanded: Vec<String> = Vec::new();
        for c in base_cands {
            expanded.push(c.clone());
            if js_ext_re.is_match(&c) {
                expanded.push(js_ext_re.replace(&c, "").into_owned());
            }
        }

        let mut suffixes: Vec<&str> = vec![""];
        suffixes.extend_from_slice(EXTS);
        suffixes.extend_from_slice(&[".d.ts", "/index.ts", "/index.tsx", "/index.js"]);

        for c in expanded {
            for suf in &suffixes {
                let p = format!("{c}{suf}");
                if EXTS.iter().any(|ext| p.ends_with(ext)) && self.root.join(&p).is_file() {
                    return Some(p);
                }
            }
        }
        None
    }

    pub fn imports(&mut self, rel: &str) -> BTreeSet<String> {
        if let Some(cached) = self.imp.get(rel) {
            return cached.clone();
        }
        let t = self.text(rel);
        let from_re = Regex::new(r#"(?s)(?:import|export)\s[^;'"]*?\bfrom\s*['"]([^'"]+)['"]"#).unwrap();
        let bare_re = Regex::new(r#"\bimport\s*['"]([^'"]+)['"]"#).unwrap();
        let dyn_re = Regex::new(r#"(?:\bimport|\brequire)\s*\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap();

        let mut specs = BTreeSet::new();
        for re in [&from_re, &bare_re, &dyn_re] {
            for caps in re.captures_iter(&t) {
                specs.insert(caps[1].to_string());
            }
        }
        let mut resolved = BTreeSet::new();
        for s in specs {
            if let Some(r) = self.resolve(rel, &s) {
                resolved.insert(r);
            }
        }
        self.imp.insert(rel.to_string(), resolved.clone());
        resolved
    }

    pub fn reexports(&mut self, rel: &str) -> BTreeSet<String> {
        if let Some(cached) = self.rex.get(rel) {
            return cached.clone();
        }
        let t = self.text(rel);
        let rex_re = Regex::new(
            r#"(?s)\bexport\s+(?:type\s+)?(?:\*(?:\s+as\s+\w+)?|\{[^}]*\})\s*from\s*['"]([^'"]+)['"]"#,
        )
        .unwrap();
        let specs: Vec<String> = rex_re.captures_iter(&t).map(|c| c[1].to_string()).collect();
        let mut resolved = BTreeSet::new();
        for s in specs {
            if let Some(r) = self.resolve(rel, &s) {
                resolved.insert(r);
            }
        }
        self.rex.insert(rel.to_string(), resolved.clone());
        resolved
    }

    pub fn all_exporting_files(&mut self) -> Vec<String> {
        if let Some(cached) = &self.all_files_cache {
            return cached.clone();
        }
        let out = Command::new("rg")
            .args([
                "--files",
                "-g", "*.ts",
                "-g", "*.tsx",
                "-g", "*.mts",
                "-g", "*.js",
                "-g", "*.jsx",
                "-g", "*.mjs",
                "-g", "!node_modules",
                "-g", "!.next",
            ])
            .current_dir(&self.root)
            .output();
        let mut files = Vec::new();
        if let Ok(o) = out {
            for f in String::from_utf8_lossy(&o.stdout).split_whitespace() {
                let rel = f.trim_start_matches("./").to_string();
                if !is_test(&rel) && self.text(&rel).contains("export") {
                    files.push(rel);
                }
            }
        }
        files.sort();
        self.all_files_cache = Some(files.clone());
        files
    }

    pub fn providers(&mut self, decl: &str, files: &[String]) -> BTreeSet<String> {
        let mut prov = BTreeSet::new();
        prov.insert(decl.to_string());
        let mut grew = true;
        while grew {
            grew = false;
            for f in files {
                if !prov.contains(f) {
                    let rex = self.reexports(f);
                    if rex.iter().any(|r| prov.contains(r)) {
                        prov.insert(f.clone());
                        grew = true;
                    }
                }
            }
        }
        prov
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_string_only(s: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let is_quote = |c: char| matches!(c, '\'' | '"' | '`');
    let mut has_quoted = false;
    let mut has_unquoted = false;
    let mut start = 0;
    while let Some(pos) = s[start..].find(name) {
        let idx = start + pos;
        let prev = s[..idx].chars().next_back();
        let next = s[idx + name.len()..].chars().next();
        if prev.is_some_and(is_quote) && next.is_some_and(is_quote) {
            has_quoted = true;
        }
        let prev_unq = !prev.is_some_and(|c| is_word(c) || is_quote(c) || c == '.');
        let next_unq = !next.is_some_and(|c| is_word(c) || is_quote(c));
        if prev_unq && next_unq {
            has_unquoted = true;
        }
        start = idx + 1;
        while start < s.len() && !s.is_char_boundary(start) {
            start += 1;
        }
    }
    has_quoted && !has_unquoted
}

fn is_named_arg(s: &str, name: &str) -> bool {
    let esc = regex::escape(name);
    let re = Regex::new(&format!(r"(?:^|[{{,(]\s*){esc}\s*\??:(.*)")).unwrap();
    if let Some(caps) = re.captures(s) {
        return !caps[1].starts_with(':');
    }
    false
}

pub fn role(line: &str, name: &str) -> &'static str {
    let s = line.trim();
    if s.starts_with("//") || s.starts_with('*') || s.starts_with("/*") {
        return "comment";
    }
    let esc = regex::escape(name);
    let decl_re1 = Regex::new(&format!(
        r"\b(?:function\s*\*?|class|interface|type|enum)\s+{esc}\b"
    ))
    .unwrap();
    let decl_re2 = Regex::new(&format!(r"\b(?:const|let|var)\s+{esc}\b")).unwrap();
    if decl_re1.is_match(s) || decl_re2.is_match(s) {
        return "declaration";
    }
    let imp_start = Regex::new(r"^(?:import|export)\b").unwrap();
    let has_from = Regex::new(r"\bfrom\b").unwrap();
    if imp_start.is_match(s) && (has_from.is_match(s) || s.ends_with('{') || s.ends_with(',')) {
        let before_from = s.split("from").next().unwrap_or(s);
        let word_re = Regex::new(&format!(r"\b{esc}\b")).unwrap();
        return if word_re.is_match(before_from) {
            "import"
        } else {
            "import-other"
        };
    }
    if is_string_only(s, name) {
        return "string";
    }
    if is_named_arg(s, name) {
        return "named-arg";
    }
    "use"
}

pub fn call_range(lines: &[&str], line_1based: usize) -> (usize, usize) {
    let mut depth = 0i32;
    let mut seen = false;
    let start_idx = line_1based.saturating_sub(1);
    let end_limit = lines.len().min(line_1based + 39);
    for (j, &ln) in lines.iter().enumerate().take(end_limit).skip(start_idx) {
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
        if !seen && j > start_idx {
            break;
        }
    }
    if let Some(&first_line) = lines.get(start_idx) {
        let jsx_open = Regex::new(r"<\w").unwrap();
        if jsx_open.is_match(first_line) && !first_line.replace("=>", "").contains('>') {
            for (j, &ln) in lines.iter().enumerate().take(end_limit).skip(start_idx + 1) {
                if ln.replace("=>", "").contains('>') {
                    return (line_1based, j + 1);
                }
            }
        }
    }
    (line_1based, line_1based)
}

fn rg_hits(root: &Path, name: &str) -> BTreeMap<String, Vec<(usize, String)>> {
    let out = Command::new("rg")
        .args([
            "-nw",
            "-g", "*.ts",
            "-g", "*.tsx",
            "-g", "*.mts",
            "-g", "*.js",
            "-g", "*.jsx",
            "-g", "*.mjs",
            "-g", "!node_modules",
            "-g", "!.next",
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
    let line_re = Regex::new(r"^\./(.+?\.(?:ts|tsx|mts|js|jsx|mjs)):(\d+):(.*)$").unwrap();
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
) -> TsRecommendResult {
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
    let all_files = repo.all_exporting_files();
    let mut prov = BTreeSet::new();
    for d in &dfiles {
        let p = repo.providers(d, &all_files);
        prov.extend(p);
    }

    let dot_call_re = Regex::new(&format!(r"\.{}\b", regex::escape(name))).unwrap();

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

        let rel_imps = repo.imports(rel);
        let mut near = dfiles.contains(rel) || rel_imps.iter().any(|i| prov.contains(i));
        if !near && !strong.is_empty() && strong.iter().any(|(_, tx, _)| dot_call_re.is_match(tx)) {
            for g in &rel_imps {
                let g_imps = repo.imports(g);
                if g_imps.iter().any(|i| prov.contains(i)) {
                    near = true;
                    break;
                }
            }
        }

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

    let groups = if dfiles.len() > 1 {
        let mut gmap = BTreeMap::new();
        for d in &dfiles {
            let d_prov = repo.providers(d, &all_files);
            let mut hit = Vec::new();
            for rel in tiers[0].keys() {
                let imps = repo.imports(rel);
                if rel == d || imps.iter().any(|i| d_prov.contains(i)) {
                    hit.push(rel.clone());
                }
            }
            gmap.insert(d.clone(), hit);
        }
        Some(gmap)
    } else {
        None
    };

    let t1_keys: BTreeSet<String> = tiers[0].keys().cloned().collect();
    let t1_dirs: BTreeSet<String> = t1_keys
        .iter()
        .map(|f| {
            Path::new(f)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    let mut flow = Vec::new();
    for f in &all_files {
        if !t1_keys.contains(f) {
            let f_dir = Path::new(f)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if t1_dirs.contains(&f_dir) {
                let imps = repo.imports(f);
                if imps.iter().any(|i| t1_keys.contains(i)) {
                    flow.push(f.clone());
                }
            }
        }
    }
    flow.sort();

    let rg_lines = byfile.values().map(Vec::len).sum();
    TsRecommendResult {
        name: name.to_string(),
        declarations: decls,
        tiers,
        rg_files: byfile.len(),
        rg_lines,
        groups,
        flow,
    }
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

pub fn render(res: &TsRecommendResult, show_all: bool, show_flow: bool) -> String {
    let decl_str = if res.declarations.is_empty() {
        "not found in non-test .ts/.tsx".to_string()
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
    if res.name.chars().next().is_some_and(char::is_uppercase)
        && res.declarations.iter().any(|(f, _)| f.ends_with(".tsx"))
    {
        o.push(format!(
            "NOTE: {} looks like a React component. A new prop is a JSX attribute, not a positional argument: change the props type in the declaring file, then every JSX use. This tool did worse than plain rg on components in our tests.",
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

    if !res.flow.is_empty() {
        if show_flow {
            o.push(format!(
                "FLOW: {} files import a TIER 1 file from the same folder without naming {} (a type may flow through them):",
                res.flow.len(),
                res.name
            ));
            o.extend(res.flow.iter().cloned());
        } else {
            o.push(format!(
                "flow: {} more files import a TIER 1 file without naming {} (use --flow to list them)",
                res.flow.len(),
                res.name
            ));
        }
    }

    for (k, t, label) in [
        (2, t2, "TIER 2: strong match in a file that does not import the declaring file"),
        (3, t3, "TIER 3: weak match (comment, string, key)"),
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
