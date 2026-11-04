"""tsconfig (`extends`, paths) and workspace-package resolution for the TypeScript profile."""
import json, os, re, subprocess
from pathlib import Path


class Resolver:
    """Mixin: needs self.root. Fills self._tsc / self._pkgs."""

    def _load_aliases(self):
        self._tsc = {}; self._pkgs = {}
        out = subprocess.run(["rg", "--files", "-g", "package.json", "-g", "!node_modules"], cwd=self.root, capture_output=True, text=True).stdout.split()
        for f in out:
            try: j = json.loads((self.root / f).read_text(errors="ignore"))
            except ValueError: continue
            if isinstance(j, dict) and isinstance(j.get("name"), str): self._pkgs[j["name"]] = (Path(f).parent.as_posix(), j)
        return self._paths_for(".")

    @staticmethod
    def _jsonc(path):
        raw = re.sub(r'("(?:\\.|[^"\\])*")|//[^\n]*|/\*.*?\*/', lambda m: m.group(1) or "", path.read_text(errors="ignore"), flags=re.S)
        try: return json.loads(re.sub(r",(\s*[}\]])", r"\1", raw))
        except ValueError: return {}

    def _config_file(self, spec, frm_dir):
        """tsconfig `extends` target: relative path, or a workspace package file."""
        if spec.startswith("."):
            c = Path(os.path.normpath(self.root / frm_dir / spec))
        else:
            for name, (d, _) in sorted(self._pkgs.items(), key=lambda kv: -len(kv[0])):
                if spec == name or spec.startswith(name + "/"):
                    c = self.root / d / spec[len(name):].lstrip("/"); break
            else: return None
        for cand in (c, Path(str(c) + ".json"), c / "tsconfig.json"):
            if cand.is_file(): return cand
        return None

    def _options(self, cfg, depth=0):
        """compilerOptions paths/baseUrl with `extends` merged; each value tagged with the directory it was declared in."""
        j = self._jsonc(cfg); here = cfg.parent.relative_to(self.root).as_posix() if cfg.parent != self.root else "."
        opts = {}
        ext = j.get("extends")
        for e in ([ext] if isinstance(ext, str) else ext or []):
            c = depth < 6 and self._config_file(e, here)
            if c: opts.update(self._options(c, depth + 1))
        co = j.get("compilerOptions") or {}
        if "baseUrl" in co: opts["baseUrl"] = os.path.normpath(Path(here) / co["baseUrl"])
        if "paths" in co: opts["paths"] = (co["paths"], here)
        return opts

    def _paths_for(self, d):
        """Aliases visible from directory d: the nearest tsconfig.json upward, extends followed."""
        cur = Path(d)
        while True:
            cfg = self.root / cur / "tsconfig.json"
            if cfg.is_file(): break
            if str(cur) == ".": return []
            cur = cur.parent
        key = cur.as_posix()
        if key not in self._tsc:
            o = self._options(cfg); out = []
            if "paths" in o:
                pmap, defdir = o["paths"]
                base = o.get("baseUrl", defdir)
                for k, v in pmap.items():
                    if v: out.append((k.rstrip("*"), os.path.normpath(Path(base) / v[0].rstrip("*")).lstrip("./") if v[0].rstrip("*") not in ("", ".", "./") else os.path.normpath(base).lstrip("./")))
            self._tsc[key] = out
        return self._tsc[key]

    def _workspace(self, spec):
        for name, (d, j) in sorted(self._pkgs.items(), key=lambda kv: -len(kv[0])):
            if spec != name and not spec.startswith(name + "/"): continue
            sub = "." + spec[len(name):]; ex = j.get("exports"); outs = []
            def pick(v):
                if isinstance(v, str): return v
                if isinstance(v, dict):
                    for k in ("import", "default", "types", "require"):
                        if k in v and (r := pick(v[k])): return r
                return None
            if isinstance(ex, (str, dict)):
                tbl = {".": ex} if isinstance(ex, str) or not any(k.startswith(".") for k in ex) else ex
                if sub in tbl and pick(tbl[sub]): outs.append(pick(tbl[sub]))
                for k, v in tbl.items():
                    if "*" in k and pick(v):
                        a, b = k.split("*", 1)
                        if sub.startswith(a) and sub.endswith(b) and len(sub) >= len(a) + len(b):
                            outs.append(pick(v).replace("*", sub[len(a):len(sub) - len(b)]))
            if sub == ".":
                outs += [j[k] for k in ("types", "module", "main") if isinstance(j.get(k), str)] + ["src/index", "index"]
            else:
                outs += ["src/" + sub[2:], sub[2:]]
            return [os.path.normpath(Path(d) / o) for o in outs]
        return []
