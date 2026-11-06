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
- **Does it miss files?** Checked against the compiler: in all 217 Kotlin tasks every truth file was listed (rules were partly tuned on them), and 191 of 194 tasks (99.3% of files) on a fresh TypeScript set. The rare misses are files that never mention the name. On the public `create-t3-turbo` (compiler truth, 31 tasks) 52 of 54 truth files were in tier 1 (96.3%; 25.9% before the workspace-import support), but that support was written while looking at this same repository, so treat the number as optimistic.
- **Does it help a coding agent?** On function and constructor signature changes, context tokens fell by 9.8% to 34.5% in five of six setups; in the sixth (Codex on TypeScript) the change cannot be told apart from noise. Compile pass counts were equal or higher, but the one Kotlin difference (112 -> 114 of 114) is a single task. **React components are the exception:** with the first tool version Haiku failed 6 of 10 component tasks against 1 of 10 for plain `rg` (see the limits).
- **The catch:** the savings appeared only when the agent was also told not to re-run `rg`. Without that sentence, cost went up 10.3% [+0.0003, +0.0009 USD per task] on one of two Kotlin sets (57 tasks, Haiku); on the other set (23 tasks) the same comparison was -0.9%, indistinguishable from zero. The sentence was added after seeing these results, and there is no run of the sentence alone without the tool.

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
moru-narrow --install        # optional: tell your coding agent to use it (see "Use with your coding agent")
```

Without installing, from a clone: `git clone https://github.com/taehwandev/moru-narrow && cd moru-narrow && python3 -m moru_narrow <Name> --repo ...`.

Published on [PyPI](https://pypi.org/project/moru-narrow/); not on npm. `pip install moru-narrow` was tested in a clean virtualenv on macOS (version 26.45.0, Python 3.9.6 and 3.13 builds); other platforms are untested. To follow the latest commit instead, install from Git: `pipx install git+https://github.com/taehwandev/moru-narrow`.

## Use with your coding agent

The tool only helps an agent when the agent is told how to use it. In one of our two Kotlin sets, giving the agent the tool alone raised cost by 10.3% (the other set: -0.9%); the savings appeared only with the instruction "do not re-run `rg` for the same name" (details in the savings section). One command registers that instruction:

```bash
moru-narrow --install                # agent skill in ~/.claude/skills/ (Claude Code) and ~/.agents/skills/ (Codex and other agent-skills tools)
moru-narrow --install --project      # same, but into ./.claude and ./.agents so your team shares it via git
moru-narrow --install --agents-md    # no skill support (e.g. Cursor): add a marked block to ./AGENTS.md
```

Re-running is safe: an existing `SKILL.md` is kept (`--force` overwrites it), and the `AGENTS.md` block is replaced in place without touching your other text. `--target claude|agents` limits the skill install to one location. The installed text is based on the instruction used in the agent tests (worded differently, and extended as noted below):

> When a task changes a function or constructor signature (adds or changes a parameter), run `moru-narrow <Name> --repo <project root>` once and edit the call sites it lists under TIER 1. Do not run `rg` again for the same name. Use `--all` only if the list looks incomplete. If the output has a WARNING about several declarations, rerun with `--decl <file>` for the one you change. If it says the name looks like a React component, read the props type first and check the list against `rg` before trusting it. For anything else (renames, logic bugs, config strings), or if moru-narrow reports an error or an unsupported language, use plain `rg`.

The exact wording was not the one measured, and the last sentence (use plain `rg` for other tasks and unsupported languages) is new: the tests only contained signature-change tasks and a prompt written for them.

**Other tools.** `--install` only writes agent-skills files and `AGENTS.md`. If your tool has its own instruction file (Cursor rules, Copilot instructions, and similar), copy the quoted text above into it. Any agent that can run a shell command can call `moru-narrow` directly.

**Check that it works.** Start a new session (a skill is picked up at session start), ask for a signature change such as "add a required `traceId` parameter to `<function>`", and look at the commands the agent runs: it should call `moru-narrow <function>` once and not follow with `rg` for the same name. If it does not call the tool, the instruction was not loaded.

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
   - Kotlin: the module is reachable through Gradle dependencies (`project(":x")`, `project(path = ":x")`, type-safe
     `projects.x.y` accessors, and dependencies added by convention plugins in `build-logic`/`buildSrc`), and the file is
     in the same package or imports it.
   - TypeScript: the file imports the declaring file (relative paths, `tsconfig` aliases including `extends`, workspace
     packages through their `exports`, re-exporting barrel files, `.js`/`.jsx` specifiers), or imports a wrapper that
     does and calls `X.name`, or is the declaring file itself.
4. Everything else is demoted to tier 2/3. **Filters only demote. `--all` always lists everything `rg` found.**
5. Multi-line calls are returned as ranges (`19-22`), because compilers report errors at argument lines.

```text
moru-narrow <Name> [--repo DIR] [--decl FILE] [--all] [--flow] [--json] [--lang kotlin|ts]
```

Two more behaviours worth knowing:

- If the name has declarations in more than one file, tier 1 is grouped by the declaration each file can see, with a
  warning to pass `--decl <FILE>` for the one you change.
- TypeScript only: a type can flow through a file that never names the symbol. The output ends with a one-line
  hint (`flow: N more files ...`) and `--flow` lists those files: they import a tier-1 file from the same folder. On
  our fresh TypeScript set this found 3 of the 9 files tier 1 missed, at 1.6 extra files per task, which is why it is
  not in the default list.

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

**What we learned the hard way.** The first TypeScript agent run (Haiku, 44 tasks: 34 functions and 10 React components) failed
11 tasks with the tool against 1 with plain `rg`. Taking the failures apart from the stored logs:

- **5 function tasks:** every file the compiler flagged was edited; the only error left was inside the declaring file. The first
  version hid calls *inside the declaring file* because a file does not import itself, and the offline truth had excluded
  that file too, so the offline check could not see it. Fix (v3): those lines stay in tier 1. The 28 fresh function tasks
  then passed 56/56 in both conditions.
- **6 component tasks:** 3 never changed the declaration, 3 left call sites. The list named the right files; the agent
  stopped on the mismatch between a "first parameter" and JSX attributes. Plain `rg` failed 1 of the same 10.

Components were then left out of the agent tests. That decision came after seeing the failures, so the 6/10 against 1/10
stays in the record. Lesson: validate with the real agent loop, not only offline.

## Limits

- Languages: Kotlin + Gradle and TypeScript. Each language needs a small profile (role patterns, how to see a
  declaration). Dynamic languages need a different ground truth.
- Verified on **one change shape** (adding a required first parameter) and **one repository per language**.
- **React components: no measured gain.** With the first version, a *positional first parameter* on a component made Haiku fail 6 of 10 tasks against 1 of 10 for plain `rg`. That shape does not exist in JSX. With the right shape (a required prop) on 24 fresh tasks, both Haiku and Codex passed as often with the tool as without (Haiku 48/48 in all three conditions; Codex 45/48 baseline, 47/48 with the sentence only, 48/48 with the tool), but there was no saving: `rg` already returned about 2.7 files, and Haiku's cost was 7.2% higher with the tool [+0.0001, +0.0006 USD per task]. The tool prints a note for capitalised names in `.tsx` files.
- Declaration-based detection was extended after these experiments (type-safe `projects.*` accessors, convention plugins, tsconfig `extends`, pnpm workspaces). It was checked offline on public repositories, not with an agent. The Kotlin check counts import lines as truth, which overlaps with the tool's own signal. On `nowinandroid` (rules tuned there) the share of truth files in tier 1 rose from 41.1% to 99.3%; on `kmp-production-sample`, a repository the rules were never tuned on, from 70.8% (17/24) to 100% (24/24) over 12 tasks, a small sample.
- **Small result sets gain little.** Kotlin set 1, Haiku: with 10 or fewer `rg` files the tool changed context by +6.1% (4 tasks); with 11 to 40 files -37.0% (7 tasks), with 41 or more -39.9% (12 tasks). TypeScript function tasks (4+ files) saved 22% (Haiku) and 6 to 8% (Codex). Agent runs are short: about 4 model turns, parallel reads and edits, so there is little left to remove when `rg` is already precise.
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
