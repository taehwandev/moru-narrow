"""Project dependencies that Gradle convention plugins (build-logic / buildSrc) add to the modules applying them."""
import re
from pathlib import Path

SRC_DIRS = ("build-logic", "buildSrc")
PROJECT = re.compile(r'project\(["\'](:[\w:.-]+)["\']\)')
REGISTER = re.compile(r'\bid\s*=\s*(?:"([^"]+)"|libs\.plugins\.([\w.]+)\.get\(\)\.pluginId)[^}]{0,200}?implementationClass\s*=\s*"(\w+)"', re.S)
APPLY = re.compile(r'\bapply\((?:plugin\s*=\s*)?"([^"]+)"\)')
USES = re.compile(r'\balias\(libs\.plugins\.([\w.]+)\)|\bid\(\s*"([^"]+)"\s*\)')


def _norm(alias): return re.sub(r"[-_]", ".", alias)


def _toml_plugins(root):
    out, cur = {}, None
    toml = root / "gradle" / "libs.versions.toml"
    if not toml.is_file(): return out
    for line in toml.read_text(errors="ignore").splitlines():
        sec = re.match(r"\s*\[(\w+)\]", line)
        if sec: cur = sec.group(1); continue
        m = re.match(r'\s*([\w.-]+)\s*=\s*\{[^}]*\bid\s*=\s*"([^"]+)"', line)
        if cur == "plugins" and m: out[_norm(m.group(1))] = m.group(2)
    return out


def _sources(root):
    for sub in SRC_DIRS:
        for pat in ("*.kt", "*.kts"):
            for f in (root / sub).rglob(pat):
                if "/build/" not in f.as_posix(): yield f


def convention_deps(root, module_builds):
    """module_builds: {module dir: build file text}. Returns ({module: {dependency module paths as ':a:b'}}, resolved?)."""
    aliases = _toml_plugins(root); files = list(_sources(root))
    texts = {f: f.read_text(errors="ignore") for f in files}
    plugin_class = {}
    for f, t in texts.items():
        if f.name.startswith("build.gradle"):
            for lit, alias, cls in REGISTER.findall(t):
                pid = lit or aliases.get(_norm(alias))
                if pid: plugin_class[pid] = cls
    if not plugin_class: return {}, False

    def deps_of(pid, seen):
        if pid in seen or pid not in plugin_class: return set()
        seen.add(pid)
        body = next((t for f, t in texts.items() if re.search(rf"\bclass\s+{plugin_class[pid]}\b", t)), "")
        out = set(PROJECT.findall(body))
        for nxt in APPLY.findall(body): out |= deps_of(nxt, seen)
        return out

    per = {}
    for mod, text in module_builds.items():
        ids = {aliases.get(_norm(a)) if a else lit for a, lit in USES.findall(text)}
        per[mod] = set().union(*(deps_of(i, set()) for i in ids if i)) if ids else set()
    return per, True
