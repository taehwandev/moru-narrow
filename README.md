# moru-narrow

**English** | [한국어](README.ko.md)

> **rg tells you every line that mentions a name. moru-narrow tells you which of those lines you must edit.** It keeps every hit; it only changes the order and what is shown first.

`moru-narrow` post-processes `rg -w <Name>` for coding agents and humans. It keeps every hit, ranks them, and prints the
files and **line ranges** to check first. No index, no database, no model calls, no daemon. Python standard library and
[ripgrep](https://github.com/BurntSushi/ripgrep) only.

> Scope: **signature changes** (add or change a parameter, then find what breaks). It is not a general `rg` replacement: for renaming a variable, tracing a logic bug or searching a config string, use plain `rg`.
>
> Status: alpha / experimental. MIT license. The examples use the `moru-narrow` command (alias `mnarrow`); from a clone, run `python3 -m moru_narrow` instead.

## In short

- **What it does:** after `rg -w name`, it lists the few files and line ranges you must edit, ranked, and hides the rest behind `--all`. Nothing is dropped.
- **Does it miss files?** Checked against the compiler: in all 217 Kotlin tasks every truth file was listed (rules were partly tuned on them), and 191 of 194 tasks (99.3% of files) on a fresh TypeScript set. The rare misses are files that never mention the name.
- **Does it help a coding agent?** In 6 test setups the compile success rate never dropped below the baseline. Context tokens fell by 9.8% to 34.5% in five setups; in the sixth (Codex on TypeScript) the change cannot be told apart from noise.
- **The catch:** the savings appeared only when the agent was also told not to re-run `rg`. Without that sentence, cost went up 10.3% [+0.0003, +0.0009 USD per task] in one test (Kotlin, Haiku, 57 tasks).

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

Needs Python 3.9+ and [ripgrep](https://github.com/BurntSushi/ripgrep) (`rg`) on your `PATH`. No other dependency. It does not need Moru Core or any other tool.

```bash
pipx install moru-narrow     # or: uv tool install moru-narrow / pip install moru-narrow
moru-narrow <Name> --repo /path/to/your/project                # Kotlin+Gradle or TypeScript, auto-detected
```

Without installing, from a clone: `git clone https://github.com/taehwandev/moru-narrow && cd moru-narrow && python3 -m moru_narrow <Name> --repo ...`.

Published on [PyPI](https://pypi.org/project/moru-narrow/); not on npm. `pip install moru-narrow` was tested in a clean virtualenv on macOS (version 26.45.0, Python 3.9.6 and 3.13 builds); other platforms are untested. To follow the latest commit instead, install from Git: `pipx install git+https://github.com/taehwandev/moru-narrow`.

## Use with your coding agent

The tool only helps an agent when the agent is told how to use it (see the savings section: without the "do not re-run rg" sentence cost went up). One command registers that instruction:

```bash
moru-narrow --install                # agent skill in ~/.claude/skills/ (Claude Code) and ~/.agents/skills/ (Codex and other agent-skills tools)
moru-narrow --install --project      # same, but into ./.claude and ./.agents so your team shares it via git
moru-narrow --install --agents-md    # no skill support (e.g. Cursor): add a marked block to ./AGENTS.md
```

Re-running is safe: an existing `SKILL.md` is kept (`--force` overwrites it), and the `AGENTS.md` block is replaced in place without touching your other text. `--target claude|agents` limits the skill install to one location. The installed text is based on the instruction used in the agent tests (worded differently, and extended as noted below):

> When a task changes a function or constructor signature (adds or changes a parameter), run `moru-narrow <Name> --repo <project root>` once and edit the call sites it lists under TIER 1. Do not run `rg` again for the same name. Use `--all` only if the list looks incomplete. For anything else (renames, logic bugs, config strings), or if moru-narrow reports an error or an unsupported language, use plain `rg`.

The exact wording was not the one measured, and the last sentence (use plain `rg` for other tasks and unsupported languages) is new: the tests only contained signature-change tasks and a prompt written for them. Any agent that can run a shell command can also call `moru-narrow` directly.

## Why

Changing a function signature means finding every place that breaks. `rg -w name` finds all of them, plus comments,
strings, same-named functions in other modules and unrelated files. On a Kotlin monorepo it returned **60 files for a
change that broke 3 to 4**. An agent then reads all of them.

A code-graph index was the obvious fix. We measured a syntax-level code-graph index on the same Kotlin repo and it did not
pay off at the agent level: no measurable cost difference with location-only output, and about +45.5% cost when it returned source code (that run also carried MCP connection overhead, so part of the increase is not the graph itself).
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

## How to read the numbers

| Term | What it means here |
| --- | --- |
| **Truth files** | Production files that stop compiling after we add a required first parameter (`traceId`) to a function. The compiler decides, not us. |
| **Recall** | Whether the truth files appear in tier 1. The table counts tasks where all of them do (`23/23`); TypeScript rows also give files (`428/431` = 3 truth files were not listed). |
| **Tier 1** | The files moru-narrow tells you to read first. |
| **`rg -w` files / lines** | What plain ripgrep returns for the same name. This is the baseline to beat. |
| **Fresh set** | Candidates drawn after the rules were frozen. Nothing was tuned on them, so they are the fair test. |
| **Baseline vs. with moru-narrow** (agent tests) | Same coding agent, same task, same prompt. Baseline may use `rg`, `ls` and file reads. The other condition also gets the tool and the sentence "edit from the list, do not re-run rg for the same name". |
| **Compile pass** | The project compiles after the agent's edit (Kotlin: Gradle, TypeScript: `tsc --noEmit`). TypeScript runs also require that the declaration really gained the parameter, because an agent that changes nothing would otherwise pass. |
| **Context tokens** | Input tokens the model processed during the whole run, cached ones included. A proxy for how much it read. |
| **Cost** | API-equivalent price estimate reported by the Claude CLI, not an actual bill. Codex does not report it, shown as n/a. |
| **`[a, b]` interval** | 95% range from a task-paired bootstrap (10,000 resamples): tasks are resampled and the two conditions are compared on the same task. If the range includes 0, treat the difference as noise. |
| **tasks x reps** | Number of tasks times repetitions per task. Repetitions of one task are averaged before the comparison. |

## Does it miss files?

Ground truth is the **compiler**: add a required first parameter `traceId: String` / `traceId: string` to a function (or
Kotlin constructor), compile, and take the production files that fail as the truth.

| Set | Tasks | `rg -w` files / lines | moru-narrow tier 1 files / lines | Tasks with every truth file in tier 1 |
| --- | ---: | --- | --- | ---: |
| Kotlin, 23 selected noisy functions | 23 | 60.4 / 139 | 8.8 / 18 | 23/23 |
| Kotlin, held-out functions | 120 | 68.3 / 281 | 3.3 / 5 | 120/120 |
| Kotlin, held-out noisy subset (rg >= 20 files and >= 2x the truth) | 57 | 132.9 / 571 | 3.8 / 7 | 57/57 |
| Kotlin, constructor changes | 74 | 14.9 / 36 | 8.8 / 17 | 74/74 |
| TypeScript, **fresh** functions (v3 rules frozen first) | 194 | 3.5 / 7 | 2.5 / 6 | 191/194 (files: 428/431, 99.3%) |
| TypeScript, fresh, noisy functions (rg >= 5 files) | 28 | 9.5 / 20 | 8.2 / 18 | 28/28 (files: 193/193) |

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
| TypeScript, Haiku, 28 x 2 | 56/56 -> 56/56 | **-22.2%** [-29.4%, -15.3%] | **-11.1%** [-17.3%, -4.3%] |
| TypeScript, Codex terra, 28 x 2 | 56/56 -> 56/56 | -6.5% [-13.5%, 0.0%] (touches 0) | n/a |

**The agent instruction matters as much as the tool.** In the first Kotlin experiment the agent got the same list but
kept re-running `rg` afterwards, and cost went **up** 10.3% (context +12.8%) on the larger Haiku set. Adding one
sentence, "edit the call sites from this list without re-running rg for the same name; use --all only if the list looks
incomplete", changed that: with it, `rg` calls per run fell from 2.4 to 0.02 (Kotlin, Codex, 23 tasks) and from 2.8 to 0.5
(TypeScript, Haiku), and it produced the table above. That revised prompt was written after seeing the first
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
- Savings depend on the model. Codex on the TypeScript repo showed no clear reduction (-6.5%, interval touches 0). There
  `rg` was already precise (3.5 files on average for functions) and the agent ran only ~3 commands per task; we did not test why.
- Sample sizes are small (28 to 57 tasks, 2 repetitions per task). In the baseline every run passed in TypeScript, so pass rate cannot show an improvement there, only that nothing broke.
- The agent tests sent private source to the model providers (Anthropic, OpenAI) through the author's own local CLI logins.

## Reproduce

The evaluation harnesses (agent runners, compiler-truth generators, scorers) are not in this repository yet: they carry local paths and private repository details and need a scrub first. `results/` holds the aggregate tables. The method is described above: compiler errors after adding a required first parameter are the ground truth; rules were frozen (hash recorded) before the fresh TypeScript set was scored.

## Versioning

Weekly CalVer `YY.WW.N` (scheme `calver-weekly`), for example `26.45.0`: two-digit year, week number, and a release count that starts at `0` each week and counts published packages. Weeks follow the Korean calendar (Asia/Seoul, Monday-start weeks). Git tags use a `v` prefix (`v26.45.0`); the package metadata omits it. Python normalises a zero-padded week, so week 5 appears as `26.5.0` on PyPI. This is a date-based version, not a compatibility signal: while the tool is alpha, read the commit log before upgrading.

## Status

Alpha. On PyPI, not on npm. No unit tests yet. The ranking was validated against compiler ground truth (see above); `--install` was only checked by hand in a temporary home directory (fresh install, re-run, `--project`, `--agents-md` idempotence). Issues and results from other repositories are welcome.

License: [MIT](LICENSE).
