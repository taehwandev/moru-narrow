use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

const SRC_DIRS: &[&str] = &["build-logic", "buildSrc"];

fn norm_alias(alias: &str) -> String {
    alias.replace(['-', '_'], ".")
}

fn toml_plugins(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let toml_path = root.join("gradle").join("libs.versions.toml");
    let Ok(content) = fs::read_to_string(&toml_path) else {
        return out;
    };
    let sec_re = Regex::new(r"^\s*\[(\w+)\]").unwrap();
    let plug_re = Regex::new(r#"^\s*([\w.-]+)\s*=\s*\{[^}]*\bid\s*=\s*"([^"]+)""#).unwrap();
    let mut cur_sec = String::new();
    for line in content.lines() {
        if let Some(caps) = sec_re.captures(line) {
            cur_sec = caps[1].to_string();
            continue;
        }
        if cur_sec == "plugins" {
            if let Some(caps) = plug_re.captures(line) {
                out.insert(norm_alias(&caps[1]), caps[2].to_string());
            }
        }
    }
    out
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) {
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
            collect_sources(&p, out);
        } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".kt") || name.ends_with(".kts") {
                out.push(p);
            }
        }
    }
}

pub fn convention_deps(
    root: &Path,
    module_builds: &BTreeMap<String, String>,
) -> (BTreeMap<String, BTreeSet<String>>, bool) {
    let aliases = toml_plugins(root);
    let mut files = Vec::new();
    for sub in SRC_DIRS {
        collect_sources(&root.join(sub), &mut files);
    }
    let mut texts: Vec<(PathBuf, String)> = Vec::new();
    for f in files {
        if let Ok(t) = fs::read_to_string(&f) {
            texts.push((f, t));
        }
    }

    let reg_re = Regex::new(
        r#"(?s)\bid\s*=\s*(?:"([^"]+)"|libs\.plugins\.([\w.]+)\.get\(\)\.pluginId)[^}]{0,200}?implementationClass\s*=\s*"(\w+)""#,
    )
    .unwrap();

    let mut plugin_class: BTreeMap<String, String> = BTreeMap::new();
    for (f, t) in &texts {
        let fname = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname.starts_with("build.gradle") {
            for caps in reg_re.captures_iter(t) {
                let pid = if let Some(lit) = caps.get(1) {
                    Some(lit.as_str().to_string())
                } else {
                    caps.get(2)
                        .and_then(|al| aliases.get(&norm_alias(al.as_str())).cloned())
                };
                if let Some(pid) = pid {
                    plugin_class.insert(pid, caps[3].to_string());
                }
            }
        }
    }

    if plugin_class.is_empty() {
        return (BTreeMap::new(), false);
    }

    let proj_re = Regex::new(r#"project\(["'](:[\w:.-]+)["']\)"#).unwrap();
    let apply_re = Regex::new(r#"\bapply\((?:plugin\s*=\s*)?"([^"]+)"\)"#).unwrap();
    let uses_re = Regex::new(r#"\balias\(libs\.plugins\.([\w.]+)\)|\bid\(\s*"([^"]+)"\s*\)"#).unwrap();

    fn deps_of(
        pid: &str,
        seen: &mut BTreeSet<String>,
        plugin_class: &BTreeMap<String, String>,
        texts: &[(PathBuf, String)],
        proj_re: &Regex,
        apply_re: &Regex,
    ) -> BTreeSet<String> {
        if seen.contains(pid) {
            return BTreeSet::new();
        }
        let Some(cls) = plugin_class.get(pid) else {
            return BTreeSet::new();
        };
        seen.insert(pid.to_string());
        let cls_re = Regex::new(&format!(r"\bclass\s+{}\b", regex::escape(cls))).unwrap();
        let body = texts
            .iter()
            .find(|(_, t)| cls_re.is_match(t))
            .map(|(_, t)| t.as_str())
            .unwrap_or("");

        let mut out: BTreeSet<String> = proj_re
            .captures_iter(body)
            .map(|c| c[1].to_string())
            .collect();
        for caps in apply_re.captures_iter(body) {
            let nxt = &caps[1];
            out.extend(deps_of(nxt, seen, plugin_class, texts, proj_re, apply_re));
        }
        out
    }

    let mut per: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (mod_dir, text) in module_builds {
        let mut ids = BTreeSet::new();
        for caps in uses_re.captures_iter(text) {
            if let Some(al) = caps.get(1) {
                if let Some(resolved) = aliases.get(&norm_alias(al.as_str())) {
                    ids.insert(resolved.clone());
                }
            } else if let Some(lit) = caps.get(2) {
                ids.insert(lit.as_str().to_string());
            }
        }
        let mut deps = BTreeSet::new();
        for id in ids {
            let mut seen = BTreeSet::new();
            deps.extend(deps_of(
                &id,
                &mut seen,
                &plugin_class,
                &texts,
                &proj_re,
                &apply_re,
            ));
        }
        per.insert(mod_dir.clone(), deps);
    }

    (per, true)
}
