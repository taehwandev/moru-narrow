use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_json::Value;

pub fn norm_rel_posix(raw: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for comp in Path::new(raw).components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.last().is_some_and(|p| p != "..") {
                    parts.pop();
                } else {
                    parts.push("..".to_string());
                }
            }
            Component::Normal(os) => parts.push(os.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

pub fn strip_jsonc(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    let trailing = Regex::new(r",(\s*[}\]])").unwrap();
    trailing.replace_all(&out, "$1").into_owned()
}

fn parse_jsonc_file(path: &Path) -> Value {
    let Ok(raw) = fs::read_to_string(path) else {
        return Value::Object(serde_json::Map::new());
    };
    let clean = strip_jsonc(&raw);
    serde_json::from_str(&clean).unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
}

#[derive(Default)]
struct TsOptions {
    base_url: Option<String>,
    paths: Option<(serde_json::Map<String, Value>, String)>,
}

pub struct TsResolver {
    pub root: PathBuf,
    tsc_cache: HashMap<String, Vec<(String, String)>>,
    pkgs: BTreeMap<String, (String, Value)>,
}

impl TsResolver {
    pub fn new(root: PathBuf) -> Self {
        let mut pkgs = BTreeMap::new();
        let out = Command::new("rg")
            .args(["--files", "-g", "package.json", "-g", "!node_modules"])
            .current_dir(&root)
            .output();
        if let Ok(o) = out {
            for line in String::from_utf8_lossy(&o.stdout).split_whitespace() {
                let rel = line.trim_start_matches("./");
                if let Ok(txt) = fs::read_to_string(root.join(rel)) {
                    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&txt) {
                        if let Some(name) = map.get("name").and_then(Value::as_str) {
                            let dir = Path::new(rel)
                                .parent()
                                .map(|p| {
                                    let s = p.to_string_lossy().into_owned();
                                    if s.is_empty() { ".".to_string() } else { s }
                                })
                                .unwrap_or_else(|| ".".to_string());
                            pkgs.insert(name.to_string(), (dir, Value::Object(map)));
                        }
                    }
                }
            }
        }
        let mut res = Self {
            root,
            tsc_cache: HashMap::new(),
            pkgs,
        };
        let _ = res.paths_for(".");
        res
    }

    fn config_file(&self, spec: &str, frm_dir: &str) -> Option<PathBuf> {
        let cand_base = if spec.starts_with('.') {
            let joined = if frm_dir == "." {
                spec.to_string()
            } else {
                format!("{frm_dir}/{spec}")
            };
            self.root.join(norm_rel_posix(&joined))
        } else {
            let mut pkg_names: Vec<&String> = self.pkgs.keys().collect();
            pkg_names.sort_by_key(|k| std::cmp::Reverse(k.len()));
            let mut matched = None;
            for name in pkg_names {
                if spec == name.as_str() || spec.starts_with(&format!("{name}/")) {
                    let (d, _) = &self.pkgs[name];
                    let rest = spec[name.len()..].trim_start_matches('/');
                    let joined = if d == "." {
                        rest.to_string()
                    } else if rest.is_empty() {
                        d.clone()
                    } else {
                        format!("{d}/{rest}")
                    };
                    matched = Some(self.root.join(norm_rel_posix(&joined)));
                    break;
                }
            }
            matched?
        };

        let with_json = PathBuf::from(format!("{}.json", cand_base.display()));
        let nested_tsc = cand_base.join("tsconfig.json");
        [cand_base, with_json, nested_tsc]
            .into_iter()
            .find(|cand| cand.is_file())
    }

    fn options(&self, cfg: &Path, depth: usize) -> TsOptions {
        let j = parse_jsonc_file(cfg);
        let here = cfg
            .parent()
            .and_then(|p| p.strip_prefix(&self.root).ok())
            .map(|p| {
                let s = p.to_string_lossy().replace('\\', "/");
                if s.is_empty() { ".".to_string() } else { s }
            })
            .unwrap_or_else(|| ".".to_string());

        let mut opts = TsOptions::default();
        if depth < 6 {
            let ext_list: Vec<String> = match j.get("extends") {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            };
            for e in ext_list {
                if let Some(parent_cfg) = self.config_file(&e, &here) {
                    let parent_opts = self.options(&parent_cfg, depth + 1);
                    if parent_opts.base_url.is_some() {
                        opts.base_url = parent_opts.base_url;
                    }
                    if parent_opts.paths.is_some() {
                        opts.paths = parent_opts.paths;
                    }
                }
            }
        }

        if let Some(co) = j.get("compilerOptions").and_then(Value::as_object) {
            if let Some(bu) = co.get("baseUrl").and_then(Value::as_str) {
                let joined = if here == "." {
                    bu.to_string()
                } else {
                    format!("{here}/{bu}")
                };
                opts.base_url = Some(norm_rel_posix(&joined));
            }
            if let Some(pmap) = co.get("paths").and_then(Value::as_object) {
                opts.paths = Some((pmap.clone(), here));
            }
        }
        opts
    }

    pub fn paths_for(&mut self, dir: &str) -> Vec<(String, String)> {
        let mut cur = if dir.is_empty() { "." } else { dir }.to_string();
        let found_key = loop {
            let cfg = if cur == "." {
                self.root.join("tsconfig.json")
            } else {
                self.root.join(&cur).join("tsconfig.json")
            };
            if cfg.is_file() {
                break Some((cur, cfg));
            }
            if cur == "." {
                break None;
            }
            cur = Path::new(&cur)
                .parent()
                .map(|p| {
                    let s = p.to_string_lossy().into_owned();
                    if s.is_empty() { ".".to_string() } else { s }
                })
                .unwrap_or_else(|| ".".to_string());
        };

        let Some((key, cfg)) = found_key else {
            return Vec::new();
        };
        if let Some(cached) = self.tsc_cache.get(&key) {
            return cached.clone();
        }
        let o = self.options(&cfg, 0);
        let mut out = Vec::new();
        if let Some((pmap, defdir)) = o.paths {
            let base = o.base_url.unwrap_or(defdir);
            for (k, v) in pmap {
                let first = v
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(Value::as_str);
                if let Some(target_pat) = first {
                    let prefix = k.trim_end_matches('*').to_string();
                    let stripped = target_pat.trim_end_matches('*');
                    let resolved = if matches!(stripped, "" | "." | "./") {
                        norm_rel_posix(&base)
                    } else if base == "." {
                        norm_rel_posix(stripped)
                    } else {
                        norm_rel_posix(&format!("{base}/{stripped}"))
                    };
                    let clean = if resolved == "." {
                        String::new()
                    } else {
                        resolved.trim_start_matches("./").to_string()
                    };
                    out.push((prefix, clean));
                }
            }
        }
        self.tsc_cache.insert(key, out.clone());
        out
    }

    pub fn workspace_candidates(&self, spec: &str) -> Vec<String> {
        let mut pkg_names: Vec<&String> = self.pkgs.keys().collect();
        pkg_names.sort_by_key(|k| std::cmp::Reverse(k.len()));

        for name in pkg_names {
            if spec != name.as_str() && !spec.starts_with(&format!("{name}/")) {
                continue;
            }
            let (d, j) = &self.pkgs[name];
            let sub = format!(".{}", &spec[name.len()..]);
            let mut outs: Vec<String> = Vec::new();

            fn pick(v: &Value) -> Option<String> {
                match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(map) => {
                        for k in ["import", "default", "types", "require"] {
                            if let Some(inner) = map.get(k).and_then(pick) {
                                return Some(inner);
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }

            if let Some(ex) = j.get("exports") {
                let tbl: serde_json::Map<String, Value> = match ex {
                    Value::String(_) => {
                        let mut m = serde_json::Map::new();
                        m.insert(".".to_string(), ex.clone());
                        m
                    }
                    Value::Object(map) => {
                        if map.keys().any(|k| k.starts_with('.')) {
                            map.clone()
                        } else {
                            let mut m = serde_json::Map::new();
                            m.insert(".".to_string(), ex.clone());
                            m
                        }
                    }
                    _ => serde_json::Map::new(),
                };

                if let Some(val) = tbl.get(&sub).and_then(pick) {
                    outs.push(val);
                }
                for (k, v) in &tbl {
                    if let Some((a, b)) = k.split_once('*') {
                        if let Some(picked) = pick(v) {
                            if sub.starts_with(a)
                                && sub.ends_with(b)
                                && sub.len() >= a.len() + b.len()
                            {
                                let mid = &sub[a.len()..sub.len() - b.len()];
                                outs.push(picked.replace('*', mid));
                            }
                        }
                    }
                }
            }

            if sub == "." {
                for k in ["types", "module", "main"] {
                    if let Some(s) = j.get(k).and_then(Value::as_str) {
                        outs.push(s.to_string());
                    }
                }
                outs.push("src/index".to_string());
                outs.push("index".to_string());
            } else {
                let tail = sub.trim_start_matches("./");
                outs.push(format!("src/{tail}"));
                outs.push(tail.to_string());
            }

            return outs
                .into_iter()
                .map(|o| {
                    if d == "." {
                        norm_rel_posix(&o)
                    } else {
                        norm_rel_posix(&format!("{d}/{o}"))
                    }
                })
                .collect();
        }
        Vec::new()
    }
}
