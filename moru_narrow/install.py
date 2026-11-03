"""`moru-narrow --install`: register moru-narrow with AI coding agents (agent-skills `SKILL.md`, or an `AGENTS.md` block).

  moru-narrow --install [--target claude|agents|both] [--project] [--agents-md] [--force]
"""
import argparse, re
from pathlib import Path

RULE = ("When a task changes a function or constructor signature (adds or changes a parameter), run "
        "`moru-narrow <Name> --repo <project root>` once and edit the call sites it lists under TIER 1. "
        "Do not run `rg` again for the same name. Use `--all` only if the list looks incomplete. "
        "For anything else (renames, logic bugs, config strings), or if moru-narrow reports an error or an unsupported language, use plain `rg`.")

SKILL = f"""---
name: moru-narrow
description: After a Kotlin (Gradle) or TypeScript function/constructor signature change, list the few files and line ranges that break, instead of reading every `rg` hit.
---

# moru-narrow

{RULE}

```bash
moru-narrow getGlobalTheme --repo .     # TIER 1 = files to edit; tier 2/3 are hidden, --all shows them
```

It post-processes `rg -nw`, so every hit is still reachable with `--all`. No index, no network, no model calls.
If `moru-narrow` is not on PATH, use `python3 -m moru_narrow`.
"""

BEGIN, END = "<!-- moru-narrow:begin -->", "<!-- moru-narrow:end -->"
BLOCK = f"{BEGIN}\n## moru-narrow\n\n{RULE}\n{END}\n"


def skill_dirs(target, project):
    base = Path(".") if project else Path.home()
    return [base / d / "skills" / "moru-narrow" for d in ((".claude",) if target == "claude" else (".agents",) if target == "agents" else (".claude", ".agents"))]


def write_skill(d, force):
    f = d / "SKILL.md"
    if f.exists() and not force: return f"kept     {f} (exists, use --force to overwrite)"
    d.mkdir(parents=True, exist_ok=True); f.write_text(SKILL)
    return f"wrote    {f}"


def write_agents_md(path):
    text = path.read_text() if path.exists() else ""
    pat = re.compile(re.escape(BEGIN) + r".*?" + re.escape(END) + r"\n?", re.S)
    new = pat.sub(lambda _: BLOCK, text) if pat.search(text) else (text + ("\n" if text and not text.endswith("\n\n") else "") + BLOCK)
    path.write_text(new)
    return f"updated  {path}"


def main(argv):
    ap = argparse.ArgumentParser(prog="moru-narrow --install", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", choices=("claude", "agents", "both"), default="both")
    ap.add_argument("--project", action="store_true", help="write into ./.claude and ./.agents instead of your home directory")
    ap.add_argument("--agents-md", action="store_true", help="insert a marked block into ./AGENTS.md instead of installing a skill")
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args(argv)
    if a.agents_md: print(write_agents_md(Path("AGENTS.md")))
    else:
        for d in skill_dirs(a.target, a.project): print(write_skill(d, a.force))
