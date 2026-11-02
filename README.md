# moru-narrow

**English** | [한국어](README.ko.md)

> **rg tells you every line that mentions a name. moru-narrow tells you which of those lines you must edit.** It keeps every hit; it only changes the order and what is shown first.

`moru-narrow` post-processes `rg -w <Name>` for coding agents and humans. It keeps every hit, ranks them, and prints the
files and **line ranges** to check first. No index, no database, no model calls, no daemon. Python standard library and
[ripgrep](https://github.com/BurntSushi/ripgrep) only.

> Scope: **signature changes** (add or change a parameter, then find what breaks). It is not a general `rg` replacement: for renaming a variable, tracing a logic bug or searching a config string, use plain `rg`.
>
> Status: alpha / experimental. MIT license. The examples use the `moru-narrow` command (alias `mnarrow`); from a clone, run `python3 -m moru_narrow` instead.

```text
$ moru-narrow getGlobalTheme --repo ./my-next-app
getGlobalTheme  declared: src/lib/theme-settings.ts:28
TIER 1: check these (3 files). rg found 4 files, 6 lines.
src/app/[locale]/(walled)/manage/(protected)/admin/settings/page.tsx
  5: import { getGlobalTheme, saveGlobalTheme, type ThemeMode } from '@/lib/theme-settings';
  19-22: getGlobalTheme({ fresh: true }).then(t => {
...
hidden: tier 2 has 0 files (use --all to list them)
```

## Quick start

Needs Python 3.9+ and [ripgrep](https://github.com/BurntSushi/ripgrep) (`rg`) on your `PATH`. No other dependency.

```bash
git clone https://github.com/taehwandev/moru-narrow
cd moru-narrow
python3 -m moru_narrow <Name> --repo /path/to/your/project   # Kotlin+Gradle or TypeScript, auto-detected
```

Not published on PyPI or npm. `pyproject.toml` declares the `moru-narrow` and `mnarrow` console scripts, but installing it with pip is untested.

## Why

Changing a function signature means finding every place that breaks. `rg -w name` finds all of them, plus comments,
strings, same-named functions in other modules and unrelated files. On a Kotlin monorepo it returned **60 files for a
change that broke 3 to 4**. An agent then reads all of them.

A code-graph index was the obvious fix. We measured a syntax-level code-graph index on the same Kotlin repo and it did not
pay off at the agent level: no measurable cost reduction with location-only output, and +45.5% cost when it returned source code.
It also needs an index build and a sync after every edit (stale until synced). `moru-narrow` has no build step and no
staleness, because it runs `rg` live.

## How it works

1. `rg -nw <Name>` collects hits (test files are excluded).
2. Each line gets a role: `use`, `import`, `override`, `declaration`, `comment`, `string`, `named-arg`, ...
3. A file is **tier 1** when it has a strong role (`use`/`import`/`override`) **and** can see the declaration:
   - Kotlin: the module is reachable through Gradle `project()` edges, and the file is in the same package or imports it.
   - TypeScript: the file imports the declaring file (relative paths and `tsconfig` aliases, through re-exporting
     barrel files), or imports a wrapper that does and calls `X.name`, or is the declaring file itself.
4. Everything else is demoted to tier 2/3. **Filters only demote. `--all` always lists everything `rg` found.**
5. Multi-line calls are returned as ranges (`19-22`), because compilers report errors at argument lines.

```text
moru-narrow <Name> [--repo DIR] [--decl FILE] [--all] [--json] [--lang kotlin|ts]
```

Language is auto-detected (`tsconfig.json`, or a Gradle build).

## Does it miss files?

Ground truth is the **compiler**: add a required first parameter `traceId: String` / `traceId: string` to a function (or
Kotlin constructor), compile, and take the production files that fail as the truth.

| Set | Tasks | `rg -w` files / lines | moru-narrow tier 1 files / lines | Truth files recalled |
| --- | ---: | --- | --- | ---: |
| Kotlin, 23 selected noisy functions | 23 | 60.4 / 139 | 8.8 / 18 | 23/23 |
| Kotlin, held-out functions | 120 | 68.3 / 281 | 3.3 / 5 | 120/120 |
| Kotlin, held-out noisy subset (rg >= 20 files and >= 2x the truth) | 57 | 132.9 / 571 | 3.8 / 7 | 57/57 |
| Kotlin, constructor changes | 74 | 14.9 / 36 | 8.8 / 17 | 74/74 |
| TypeScript, **fresh** functions (v3 rules frozen first) | 194 | 3.5 / 7 | 2.5 / 6 | 428/431 (99.3%) |
| TypeScript, fresh, noisy functions (rg >= 5 files) | 28 | 9.5 / 20 | 8.2 / 18 | 193/193 |

Read these carefully:

- Kotlin rules were adjusted after the first held-out misses (two regex bugs), so the Kotlin sets are **not fully
  independent**. The TypeScript fresh set is: rules were frozen (sha256 recorded) before it was scored.
- TypeScript misses that remain are cases where the failing file **never mentions the name** (the type flows through
  another function). No `rg`-based tool can find those.
- Line level: error lines inside the recommended ranges were 90% (Kotlin functions), 96% within +-8 lines (Kotlin
  constructors) and 90% (TypeScript functions). Use ranges as hints, files as the guarantee.
- The TypeScript compiler truth is built with the TypeScript API in memory against one private Next.js repo; Kotlin on
  one private Compose Multiplatform repo. Neither repo is published here.

## Does it save an agent anything?

Same task, same prompt template, two conditions, task-paired bootstrap (10,000 resamples, 95% interval). Local logins
only. Cost is the API-equivalent estimate reported by the CLI.

| Setup | Compile pass (baseline -> with moru-narrow) | Context tokens | Cost |
| --- | --- | --- | --- |
| Kotlin, Haiku, 23 tasks x 2 | 46/46 -> 46/46 | **-34.5%** [-107k, -37k] | **-24.2%** |
| Kotlin, Haiku, 57 tasks x 2 | 112/114 -> 114/114 | **-19.2%** [-36k, -18k] | **-16.4%** |
| Kotlin, Codex terra, 23 x 2 | 44/46 -> 45/46 | **-14.2%** [-22.8%, -6.2%] | n/a |
| Kotlin, Codex terra, 57 x 2 | 112/114 -> 114/114 | **-9.8%** [-14.6%, -5.3%] | n/a |
| TypeScript, Haiku, 28 x 1 | 28/28 -> 28/28 | **-23.0%** [-35.7%, -10.0%] | -11.4% (interval includes 0) |
| TypeScript, Codex terra, 28 x 1 | 28/28 -> 28/28 | -0.8% (interval includes 0) | n/a |

**The agent instruction matters as much as the tool.** In the first Kotlin experiment the agent got the same list but
kept re-running `rg` afterwards (3.6 to 4.5 extra `rg` calls per run), and cost went **up** 10.3% on the larger set. Adding one
sentence, "edit the call sites from this list without re-running rg for the same name; use --all only if the list looks
incomplete", cut those calls to 0.7 and produced the table above. That revised prompt was written after seeing the first
results, so treat the savings as exploratory, and ship the instruction together with the tool.

**What we learned the hard way.** The first TypeScript agent run failed 10 more tasks than the baseline (33/44 vs 43/44).
Every file the compiler flagged was in the list; the miss was calls *inside the declaring file*, which the first
version hid because a file does not import itself. The offline truth had also excluded the declaring file, so the offline
check could not see it. Fix: v3 keeps those lines in tier 1. Lesson: validate with the real agent loop, not only offline.

## Limits

- Languages: Kotlin + Gradle and TypeScript. Each language needs a small profile (role patterns, how to see a
  declaration). Dynamic languages need a different ground truth.
- Verified on **one change shape** (adding a required first parameter) and **one repository per language**.
- React components are out of scope (a positional first parameter does not exist in JSX).
- Savings depend on the model. Codex on the TypeScript repo showed no reduction: `rg` was already precise there
  (3.4 files on average) and the agent ran only ~3 commands per task.
- Sample sizes are small (28 to 57 tasks, 1 to 2 repetitions per task).
- The agent tests sent private source to the model providers (Anthropic, OpenAI) through the author's own local CLI logins.

## Reproduce

The evaluation harnesses (agent runners, compiler-truth generators, scorers) are not in this repository yet: they carry local paths and private repository details and need a scrub first. `results/` holds the aggregate tables. The method is described above: compiler errors after adding a required first parameter are the ground truth; rules were frozen (hash recorded) before the fresh TypeScript set was scored.

## Status

Alpha. Not on PyPI or npm. No unit tests yet (it was validated against compiler ground truth, see above). Issues and results from other repositories are welcome.

License: [MIT](LICENSE).
