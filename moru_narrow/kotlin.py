#!/usr/bin/env python3
"""moru-narrow: recommend the files and lines to check after `rg -w <name>`, without missing any.

Static, no model calls, standard library only. Kotlin + Gradle first.
Filters only DEMOTE files into tiers; `--all` always lists everything rg found.

  moru-narrow <Name> [--repo DIR] [--decl FILE] [--all] [--json]
"""
import argparse, json, os, re, subprocess, sys
from pathlib import Path

from moru_narrow.gradleplugins import convention_deps

STRONG = ("use", "override", "import")
TEST = re.compile(r"/(src/)?\w*[Tt]est\w*/")
PKG = re.compile(r"^package\s+([\w.]+)", re.M)


def is_test(rel): return bool(TEST.search("/" + rel)) or rel.endswith("Test.kt")


class Repo:
    def __init__(self, root):
        self.root = Path(root).resolve(); self.src = {}
        self.build = {}                      # module dir -> set(dependency dirs)
        self.accessors = {}; self.builds = {}
        for g in list(self.root.rglob("build.gradle.kts")) + list(self.root.rglob("build.gradle")):
            if "/build/" in str(g): continue
            d = g.parent; base = self._settings_root(d)
            txt = g.read_text(errors="ignore")
            deps = {(base / p.strip(":").replace(":", "/")).relative_to(self.root).as_posix()
                    for p in re.findall(r'project\((?:path\s*=\s*)?["\'](:[\w:.-]+)["\']', txt)}
            self.build[d.relative_to(self.root).as_posix()] = deps
            self.accessors[g] = (base, re.findall(r'\bprojects\.([A-Za-z][\w.]*)', txt))
            self.builds[g] = d.relative_to(self.root).as_posix()
        for g, (base, accs) in self.accessors.items():
            self.build[g.parent.relative_to(self.root).as_posix()] |= {
                m for m in (self._accessor_module(base, a) for a in accs) if m}
        # Convention plugins (buildSrc / build-logic): deps they add to the modules that apply them. If their
        # registrations cannot be resolved, the deps they mention are treated as available to every module.
        per, resolved = convention_deps(self.root, {m: g.read_text(errors="ignore") for g, m in self.builds.items()})
        ambient = set() if resolved else self._ambient_deps()
        for g, mod in self.builds.items():
            base = self._settings_root(g.parent)
            extra = {(base / p.strip(":").replace(":", "/")).relative_to(self.root).as_posix() for p in per.get(mod, ())}
            self.build[mod] |= (extra | ambient) - {mod}
        self.reach = {m: self._closure(m) for m in self.build}

    def _ambient_deps(self):
        deps = set()
        for sub in ("build-logic", "buildSrc"):
            for f in list((self.root / sub).rglob("*.kt")) + list((self.root / sub).rglob("*.kts")):
                if "/build/" in str(f): continue
                for p in re.findall(r'project\(["\'](:[\w:.-]+)["\']\)', f.read_text(errors="ignore")):
                    m = p.strip(":").replace(":", "/")
                    if m in self.build: deps.add(m)
        return deps

    @staticmethod
    def _norm(s): return re.sub(r'[-_.]', '', s).lower()

    def _accessor_module(self, base, acc):
        """projects.core.ui.api -> the longest prefix that names a module dir (type-safe accessors)."""
        toks = acc.split(".")
        for n in range(len(toks), 0, -1):
            want = [self._norm(t) for t in toks[:n]]
            for m in self.build:
                p = Path(m).relative_to(base.relative_to(self.root)).parts if base != self.root else Path(m).parts
                if [self._norm(x) for x in p] == want: return m
        return None

    def _settings_root(self, d):
        cur = d
        while True:
            if (cur / "settings.gradle.kts").exists() or (cur / "settings.gradle").exists(): return cur
            if cur == self.root or cur == cur.parent: return self.root
            cur = cur.parent

    def _closure(self, m):
        seen, st = set(), [m]
        while st:
            for y in self.build.get(st.pop(), ()):
                if y not in seen: seen.add(y); st.append(y)
        return seen

    def text(self, rel):
        if rel not in self.src: self.src[rel] = (self.root / rel).read_text(errors="ignore")
        return self.src[rel]

    def module(self, rel):
        cur = Path(rel).parent
        while str(cur) != ".":
            if cur.as_posix() in self.build: return cur.as_posix()
            cur = cur.parent

    def package(self, rel):
        m = PKG.search(self.text(rel)); return m.group(1) if m else ""


def role(line, name):
    s = line.strip()
    if s.startswith(("//", "*", "/*")): return "comment"
    if re.match(r"import\s", s): return "import" if s.endswith("." + name) or " as " in s else "import-other"
    if re.search(rf"\b(val|var)\s+{name}\b", s) and not re.search(rf"\b{name}\s*\(", s): return "property"
    if re.search(rf"\bfun\s+(?:<[^>]*>\s*)?(?:[\w.<>?]+\.)?{name}\s*[(<]", s) or re.search(rf"\b(?:class|interface|object|typealias)\s+{name}\b", s):
        return "override" if "override" in s else "declaration"
    if re.search(rf"(?<![\w\"'])(?:\.|::|\b){name}\s*(?:\(|\{{|$|\)|,)", s): return "use"
    if re.search(rf"\b{name}\s*=", s): return "named-arg"
    return "other"


def call_range(lines, i):
    """1-based line i; extend to the closing paren of the call that starts there (max 40 lines)."""
    depth, seen = 0, False
    for j in range(i - 1, min(len(lines), i + 39)):
        for ch in lines[j]:
            if ch == "(": depth += 1; seen = True
            elif ch == ")": depth -= 1
        if seen and depth <= 0: return i, j + 1
        if not seen and j > i - 1: break
    return i, i


def rg_hits(repo, name):
    out = subprocess.run(["rg", "-nw", "--glob", "*.kt", name, "."], cwd=repo.root, capture_output=True, text=True).stdout.splitlines()
    byfile = {}
    for l in out:
        m = re.match(r"\./(.+?\.kt):(\d+):(.*)", l)
        if m and not is_test(m.group(1)): byfile.setdefault(m.group(1), []).append((int(m.group(2)), m.group(3)))
    return byfile


def recommend(repo, name, decl_files=None):
    byfile = rg_hits(repo, name)
    decls = []
    for rel, hits in byfile.items():
        for ln, tx in hits:
            if role(tx, name) == "declaration" and (decl_files is None or rel in decl_files): decls.append((rel, ln))
    if decl_files: decls = [(f, next((ln for ln, tx in byfile.get(f, []) if role(tx, name) == "declaration"), 0)) for f in decl_files]
    dfiles = {d[0] for d in decls}
    dmods = {repo.module(f) for f in dfiles}; dpkgs = {repo.package(f) for f in dfiles}
    tiers = {1: {}, 2: {}, 3: {}}
    for rel, hits in byfile.items():
        lines = repo.text(rel).split("\n")
        rows = [(ln, tx, role(tx, name)) for ln, tx in hits]
        strong = [r for r in rows if r[2] in STRONG]
        if rel in dfiles and (decl_files is not None or not strong): continue   # a file that only declares it needs no check; with --decl the user already knows
        m = repo.module(rel)
        in_mod = m in dmods or any(dm in repo.reach.get(m, ()) for dm in dmods)
        pk = repo.package(rel); src = repo.text(rel)
        near = pk in dpkgs or any(re.search(r"^import\s+" + re.escape(p) + r"[.\w]*", src, re.M) for p in dpkgs if p)
        tier = 1 if (strong and in_mod and near) else 2 if (strong and in_mod) else 3
        tiers[tier][rel] = [{"line": ln, "to": call_range(lines, ln)[1], "role": r, "text": tx.strip()[:160]} for ln, tx, r in (strong or rows)]
    res = {"name": name, "declarations": decls, "tiers": tiers, "rg_files": len(byfile),
           "rg_lines": sum(len(v) for v in byfile.values())}
    if len(dfiles) > 1: res["groups"] = _groups(repo, dfiles, tiers[1])
    return res


def _groups(repo, dfiles, t1):
    """tier-1 files that can see each declaring file (module reachable and same package or import)."""
    out = {}
    for d in sorted(dfiles):
        dm, dp = repo.module(d), repo.package(d)
        hit = []
        for rel in sorted(t1):
            m = repo.module(rel); src = repo.text(rel)
            sees = (m == dm or dm in repo.reach.get(m, ())) and (repo.package(rel) == dp or bool(dp and re.search(r"^import\s+" + re.escape(dp) + r"[.\w]*", src, re.M)))
            if rel == d or sees: hit.append(rel)
        out[d] = hit
    return out


def _tier1_lines(res, t1):
    """TIER 1 files with their lines; grouped by declaration when several declaring files exist."""
    def one(rel):
        out = [rel]
        for r in t1[rel]: out.append(f"  {r['line']}" + (f"-{r['to']}" if r["to"] > r["line"] else "") + f": {r['text']}")
        return out
    groups = res.get("groups")
    if not groups: return [x for rel in sorted(t1) for x in one(rel)]
    out, seen = [], set()
    for d, rels in groups.items():
        out.append(f"-- sees the declaration in {d}:")
        for rel in rels: out += one(rel); seen.add(rel)
    rest = [rel for rel in sorted(t1) if rel not in seen]
    if rest:
        out.append("-- cannot tell which declaration:")
        for rel in rest: out += one(rel)
    return out


def render(res, show_all):
    o = [f"{res['name']}  declared: " + ("; ".join(f"{f}:{ln}" for f, ln in res["declarations"]) or "not found in non-test .kt")]
    if len(res["declarations"]) > 1:
        o.append(f"WARNING: {len(res['declarations'])} declarations of {res['name']}; TIER 1 merges the callers of all of them. Pass --decl <FILE> for the one you change.")
    t1, t2, t3 = (res["tiers"][k] for k in (1, 2, 3))
    o.append(f"TIER 1: check these ({len(t1)} files). rg found {res['rg_files']} files, {res['rg_lines']} lines.")
    o.extend(_tier1_lines(res, t1))
    for k, t, label in ((2, t2, "TIER 2: strong match, outside the declaring package or its imports"), (3, t3, "TIER 3: weak match or unreachable module")):
        if show_all:
            o.append(f"{label} ({len(t)} files)")
            for rel, ls in sorted(t.items()):
                o.append(rel)
                for r in ls[:5]: o.append(f"  {r['line']}: {r['text']}")
        else:
            o.append(f"hidden: tier {k} has {len(t)} files (use --all to list them)")
    return "\n".join(o)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("name"); ap.add_argument("--repo", default="."); ap.add_argument("--decl", action="append")
    ap.add_argument("--all", action="store_true"); ap.add_argument("--json", action="store_true")
    a = ap.parse_args(); repo = Repo(a.repo)
    res = recommend(repo, a.name, set(a.decl) if a.decl else None)
    print(json.dumps(res, ensure_ascii=False) if a.json else render(res, a.all))


if __name__ == "__main__": main()
