#!/usr/bin/env python3
"""moru-narrow: recommend the files and lines to check after `rg -w <Name>` (Kotlin + Gradle, TypeScript).

Static, no model calls, no index, standard library + ripgrep only. Filters only demote; --all lists everything rg found.
  moru-narrow <Name> [--repo DIR] [--decl FILE] [--all] [--json] [--lang kotlin|ts]
  moru-narrow --install [--target claude|agents|both] [--project] [--agents-md] [--force]
"""
import importlib, sys
from pathlib import Path


def detect(repo):
    root = Path(repo)
    if (root / "tsconfig.json").exists(): return "ts"
    if any(any(root.glob(g)) for g in ("settings.gradle*", "build.gradle*", "*/build.gradle*")): return "kotlin"
    if (root / "package.json").exists(): return "ts"
    sys.exit("moru-narrow: cannot detect the language (looked for tsconfig.json or a Gradle build at the repo root); pass --lang kotlin|ts")


def main():
    argv = sys.argv[1:]; lang = None; repo = "."
    if argv[:1] == ["--install"]:
        from moru_narrow import install
        return install.main(argv[1:])
    if "--lang" in argv:
        i = argv.index("--lang"); lang = argv[i + 1]; del argv[i:i + 2]
    if "--repo" in argv: repo = argv[argv.index("--repo") + 1]
    lang = lang or detect(repo)
    mod = importlib.import_module({"kotlin": "moru_narrow.kotlin", "ts": "moru_narrow.typescript"}[lang])
    sys.argv = [f"moru-narrow({lang})"] + argv
    mod.main()


if __name__ == "__main__": main()
