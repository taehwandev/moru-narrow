#!/usr/bin/env python3
"""moru-narrow for TypeScript (v3: two-hop wrapper rule + uses inside the declaring file): recommend the files and lines to check after `rg -w <name>`, without missing any.

Static, no model calls, standard library only. Filters only DEMOTE files into tiers; `--all` lists everything rg found.
Tier 1 = a strong use/import AND the file imports the declaring file (directly or through re-exporting barrel files).

  moru-narrow <Name> [--repo DIR] [--decl FILE] [--all] [--json]
"""
import argparse, json, os, re, subprocess
from pathlib import Path

from moru_narrow.tsresolve import Resolver

STRONG = ("use", "import")
TEST = re.compile(r"(\.test\.|\.spec\.|/__tests__/|/__mocks__/)")
EXTS = (".ts", ".tsx", ".mts", ".js", ".jsx", ".mjs")
FROM = re.compile(r"""(?:import|export)\s[^;'"]*?\bfrom\s*['"]([^'"]+)['"]""", re.S)
BARE = re.compile(r"""\bimport\s*['"]([^'"]+)['"]""")
DYN = re.compile(r"""(?:\bimport|\brequire)\s*\(\s*['"]([^'"]+)['"]\s*\)""")
REEXPORT = re.compile(r"""\bexport\s+(?:type\s+)?(?:\*(?:\s+as\s+\w+)?|\{[^}]*\})\s*from\s*['"]([^'"]+)['"]""", re.S)


def is_test(rel): return bool(TEST.search("/" + rel))


class Repo(Resolver):
    def __init__(self, root):
        self.root = Path(root).resolve(); self.src = {}; self._imp = {}; self._rex = {}
        self.aliases = self._load_aliases()

    def text(self, rel):
        if rel not in self.src: self.src[rel] = (self.root / rel).read_text(errors="ignore")
        return self.src[rel]

    def resolve(self, frm, spec):
        cands = []
        if spec.startswith("."): cands.append(os.path.normpath(Path(frm).parent / spec))
        else:
            for pre, tgt in self._paths_for(Path(frm).parent.as_posix()):
                if spec.startswith(pre): cands.append(os.path.normpath(Path(tgt) / spec[len(pre):]))
            cands += self._workspace(spec)
        for c in cands:
            c = str(Path(c))
            if re.search(r"\.(?:m|c)?jsx?$", c): cands.append(re.sub(r"\.(?:m|c)?jsx?$", "", c))
            for suffix in ("", *EXTS, ".d.ts", "/index.ts", "/index.tsx", "/index.js"):
                p = c + suffix
                if p.endswith(EXTS) and (self.root / p).is_file(): return p
        return None

    def imports(self, rel):
        if rel not in self._imp:
            t = self.text(rel); specs = set(FROM.findall(t)) | set(BARE.findall(t)) | set(DYN.findall(t))
            self._imp[rel] = {r for r in (self.resolve(rel, s) for s in specs) if r}
        return self._imp[rel]

    def reexports(self, rel):
        if rel not in self._rex:
            self._rex[rel] = {r for r in (self.resolve(rel, s) for s in REEXPORT.findall(self.text(rel))) if r}
        return self._rex[rel]

    def providers(self, decl, files):
        """The declaring file plus every file that re-exports it, transitively."""
        prov, grew = {decl}, True
        while grew:
            grew = False
            for f in files:
                if f not in prov and self.reexports(f) & prov: prov.add(f); grew = True
        return prov


def role(line, name):
    s = line.strip()
    if s.startswith(("//", "*", "/*")): return "comment"
    if re.search(rf"\b(?:function\s*\*?|class|interface|type|enum)\s+{name}\b", s) or re.search(rf"\b(?:const|let|var)\s+{name}\b", s):
        return "declaration"
    if re.match(r"(?:import|export)\b", s) and (re.search(r"\bfrom\b", s) or s.endswith(("{", ","))):
        return "import" if re.search(rf"\b{name}\b", s.split("from")[0]) else "import-other"
    if re.search(rf"""(?<=['"`]){name}(?=['"`])""", s) and not re.search(rf"""(?<![\w'"`.]){name}(?![\w'"`])""", s): return "string"
    if re.match(rf"{name}\s*\??:(?!:)", s) or re.search(rf"[{{,(]\s*{name}\s*\??:(?!:)", s): return "named-arg"
    return "use"


def call_range(lines, i):
    """1-based line i; extend to the closing paren (or the end of a JSX opening tag) that starts there (max 40 lines)."""
    depth, seen = 0, False
    for j in range(i - 1, min(len(lines), i + 39)):
        for ch in lines[j]:
            if ch == "(": depth += 1; seen = True
            elif ch == ")": depth -= 1
        if seen and depth <= 0: return i, j + 1
        if not seen and j > i - 1: break
    if re.search(r"<\w", lines[i - 1]) and ">" not in lines[i - 1].replace("=>", ""):
        for j in range(i, min(len(lines), i + 39)):
            if ">" in lines[j].replace("=>", ""): return i, j + 1
    return i, i


def rg_hits(repo, name):
    out = subprocess.run(["rg", "-nw", "-g", "*.ts", "-g", "*.tsx", "-g", "*.mts", "-g", "*.js", "-g", "*.jsx", "-g", "*.mjs", "-g", "!node_modules", "-g", "!.next", "--", name, "."],
                         cwd=repo.root, capture_output=True, text=True).stdout.splitlines()
    byfile = {}
    for l in out:
        m = re.match(r"\./(.+?\.(?:ts|tsx|mts|js|jsx|mjs)):(\d+):(.*)", l)
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
    prov = set()
    for d in dfiles: prov |= repo.providers(d, _all_files(repo))
    tiers = {1: {}, 2: {}, 3: {}}
    for rel, hits in byfile.items():
        lines = repo.text(rel).split("\n")
        rows = [(ln, tx, role(tx, name)) for ln, tx in hits]
        strong = [r for r in rows if r[2] in STRONG]
        if rel in dfiles and not strong: continue          # v3: a declaring file with other uses is kept, even when --decl names it
        near = rel in dfiles or bool(repo.imports(rel) & prov)   # v3: calls inside the declaring file are near
        # v2: a file that imports a wrapper (a file importing the declaring file) and calls `X.name` is also near
        if not near and strong and any(re.search(rf"\.{re.escape(name)}\b", r[1]) for r in strong):
            near = any(repo.imports(g) & prov for g in repo.imports(rel))
        tier = 1 if (strong and near) else 2 if strong else 3
        tiers[tier][rel] = [{"line": ln, "to": call_range(lines, ln)[1], "role": r, "text": tx.strip()[:160]} for ln, tx, r in (strong or rows)]
    res = {"name": name, "declarations": decls, "tiers": tiers, "rg_files": len(byfile),
           "rg_lines": sum(len(v) for v in byfile.values())}
    if len(dfiles) > 1:
        res["groups"] = {d: [rel for rel in sorted(tiers[1]) if rel == d or repo.imports(rel) & repo.providers(d, _all_files(repo))] for d in sorted(dfiles)}
    res["flow"] = _flow(repo, tiers[1])
    return res


def _flow(repo, t1):
    """Files that never name the symbol but import a tier-1 file from the same folder (a type can flow through them)."""
    dirs = {os.path.dirname(f) for f in t1}
    return sorted(f for f in _all_files(repo) if f not in t1 and os.path.dirname(f) in dirs and repo.imports(f) & set(t1))


_ALL = {}


def _all_files(repo):
    if repo.root not in _ALL:
        out = subprocess.run(["rg", "--files", "-g", "*.ts", "-g", "*.tsx", "-g", "*.mts", "-g", "*.js", "-g", "*.jsx", "-g", "*.mjs", "-g", "!node_modules", "-g", "!.next"], cwd=repo.root, capture_output=True, text=True).stdout.split()
        _ALL[repo.root] = [f for f in out if not is_test(f) and "export" in repo.text(f)]
    return _ALL[repo.root]


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


def render(res, show_all, show_flow=False):
    o = [f"{res['name']}  declared: " + ("; ".join(f"{f}:{ln}" for f, ln in res["declarations"]) or "not found in non-test .ts/.tsx")]
    if len(res["declarations"]) > 1:
        o.append(f"WARNING: {len(res['declarations'])} declarations of {res['name']}; TIER 1 merges the callers of all of them. Pass --decl <FILE> for the one you change.")
    if res["name"][:1].isupper() and any(f.endswith(".tsx") for f, _ in res["declarations"]):
        o.append(f"NOTE: {res['name']} looks like a React component. A new prop is a JSX attribute, not a positional argument: change the props type in the declaring file, then every JSX use. This tool did worse than plain rg on components in our tests.")
    t1, t2, t3 = (res["tiers"][k] for k in (1, 2, 3))
    o.append(f"TIER 1: check these ({len(t1)} files). rg found {res['rg_files']} files, {res['rg_lines']} lines.")
    o.extend(_tier1_lines(res, t1))
    flow = res.get("flow") or []
    if flow and show_flow: o.append(f"FLOW: {len(flow)} files import a TIER 1 file from the same folder without naming {res['name']} (a type may flow through them):"); o.extend(flow)
    elif flow: o.append(f"flow: {len(flow)} more files import a TIER 1 file without naming {res['name']} (use --flow to list them)")
    for k, t, label in ((2, t2, "TIER 2: strong match in a file that does not import the declaring file"), (3, t3, "TIER 3: weak match (comment, string, key)")):
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
    ap.add_argument("--all", action="store_true"); ap.add_argument("--flow", action="store_true"); ap.add_argument("--json", action="store_true")
    a = ap.parse_args(); repo = Repo(a.repo)
    res = recommend(repo, a.name, set(a.decl) if a.decl else None)
    print(json.dumps(res, ensure_ascii=False) if a.json else render(res, a.all, a.flow))


if __name__ == "__main__": main()
