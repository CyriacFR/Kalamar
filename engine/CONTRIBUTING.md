# Contributing

This page covers changes to the engine. In the Kalamar repository, the project-wide rules (what is not accepted, the
network rule, prerequisites, how to propose a change, sign-off) are in the top-level
[CONTRIBUTING.md](../CONTRIBUTING.md); this page adds what engine changes need on top of them.

## The one hard rule

Nothing that came out of the game goes into the repository: no game files, keys, extracted data, recompiled C, compiled binaries, shaders, shader caches, fonts, audio or movies. Not in commits, not in issues, not in pull requests. Only information about the game goes in: addresses, hashes, patch code we wrote, shader and bind-state identifiers.

CI runs `tools/check_content.py` on every push and pull request. Run `python tools/check_content.py` from `engine/`
before you commit, as CI does (`--staged` run from `engine/` skips the file checks: use the full check).

A failing check is never bypassed with `--no-verify`. If it flags a file by mistake, change the check in a separate pull request and say why.

Links to game files, keys or firmware in issues or pull requests get the issue closed and the content removed.

## Code from other projects

Code may be copied only from projects under MIT, BSD, ISC, zlib or Apache-2.0. Each copy keeps its original header and gets a row in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) with the source commit.

GPL projects (yuzu, suyu, OpenSmash-Ultimate, Unicorn inside the game binary) can be read to understand behavior, but their code is not copied. Unicorn is fine as a separate test tool.

## Reporting bugs

Use the bug report form of the Kalamar repository; it says what to attach. For the engine, the useful parts are the game update, your GPU and driver, the game's log (started from the launcher: `userdata\logs\run-<date>-<time>.log` in the game folder; started on its own: `logs\totk.log` next to the executable; after a crash, copy it before you start the game again, and remove personal data from it), and the steps to reproduce. Screenshots of the launcher are fine; never attach screenshots or recordings of the game.

## Pull requests

### Branch

Open pull requests against `main`. Branch from the current `main` and rebase on it before you open the pull request.

### Before you start

- Search the open pull requests and issues on the topic. When another pull request already covers it, improve that one or say in yours how they differ.
- Fixes are reviewed before features, in order of how many players the bug hits. For a feature, open an issue first.
- Find out where the bug reproduces: OS, GPU vendor and generation, driver, game update, settings. Put that in the description.

### What the change must keep

- **The default for everyone.** A fix for one setup (a GPU vendor, a memory size, a display) is gated on that setup. Players without the problem see no change.
- **A switch.** New behavior on the frame path or in rendering sits behind a `kalamar.ini` setting or a `KLM_*` variable, so a player can turn it off and a reviewer can compare. A new setting is described in [kalamar.ini.example](kalamar.ini.example), [totk.ini.example](totk.ini.example) (the upstream targets' copy) and [docs/settings.md](docs/settings.md).
- **The fix, not a disabled feature.** A crash that goes away with a feature off points at the bug in that feature. Turning the feature off for a vendor is not a fix.
- **The frame time on weak hardware.** Game threads and the render thread do no file I/O, no GPU allocations, no per-draw state copies and no global atomic counters. Memory kept per frame or per object is freed. The full list with measured costs: [docs/pr-review-kit/02-performance.md](docs/pr-review-kit/02-performance.md), "What the hot path must not do".
- **Other GPUs.** New Vulkan code queries each feature, extension, format and limit it uses and keeps a working path without it. Nothing is hardcoded from what one GPU reports.
- **Text in the catalogs.** Text a player sees gets an id in `runtime/ui_text.h` and an entry in `runtime/ui_text.c` for every menu language (the tables of `runtime/ui/`: English, Russian, Brazilian Portuguese, Simplified Chinese, Spanish and Japanese in full, French in part); installer text goes into `installer/text/*.json`.
- **The generator owns generated C.** Change `recomp/`, never `generated/`.

### Checks by what you changed

Run every row that matches your diff and list the results in the description. Commands and details: [docs/building.md](docs/building.md), "Checks". `totk.exe` is the executable of the upstream's TotK targets; a Splatoon 3 build is `splat3.exe`, with the same modes.

| You changed | Run |
|---|---|
| anything | `python tools/check_content.py`; `build\<ver>\splat3.exe --selftest` (or `totk.exe`) |
| `recomp/`, `runtime/cpu.h`, `runtime/simd.h` | the differential tests against Unicorn: `tests\diff\lift.py`, `simd.py`, `atomic.py`, `func.py`, `tests\flags_check.py`, `tests\excl_mt.py`; the same with a `--cpu v2` build |
| `recomp/jumptables.py` | `tests\jumptables_check.py`: 0 disagreements on the target you build (the upstream's 1.2.1 and 1.4.3 too when you have them) |
| `runtime/` | `--probe-kernel`, `--probe-vkexec`, `--probe-nvn-tex`, `--probe-shader`, plus the probe for the file you touched (list at the top of `runtime/main.c`) |
| settings | `totk.exe --probe-cfg`; `python tests\settings_doc_check.py`; `python tests\env_names_check.py` (also for a page that names a variable) |
| button prompts | `.venv-py314\Scripts\python tests\romfs_prompts_test.py` |
| mods, ExeFS patches | `.venv\Scripts\python tests\exefs_patches_test.py`; `totk.exe --probe-mods` |
| `installer/` | `python tests\installer_test.py`; `python tests\installer_gui_test.py` |
| shader translator | `tools\xlate_smoke.py`; `tools\shader_census.py --version <ver> --translate` with no new failure groups |
| rendering | a run in a scene where the change acts, with `KLM_VK_VALIDATION=1`: no new validation messages; screenshots before and after, kept on your machine and described in the pull request (images of the game are never posted) |
| frame path, timing, scheduling | the measurements below |

Checks that need game files run on your own copy of the game. Results go into the pull request as text: pass or fail, counts, log lines. Never attach logs or dumps that contain game data.

### Proof

- **A bug fix** shows the bug before the change and gone after it, on the same repro: a log line, a probe or test that fails before and passes after, or a screenshot pair you describe (images of the game stay on your machine). A scripted repro (`KLM_INPUT_RECORD`, [docs/controls.md](docs/controls.md), "Scripted input") is the strongest. If the bug cannot be reproduced on your hardware, say so and point at the code that causes it.
- **A crash fix** names the crash: the log's crash report, `vk: pass marks` lines for a lost device.
- **Parts you could not run** (Linux-only code, HDR, vendor-only paths) are listed as unrun and gated away from everyone else.

### Performance measurements

A change on the frame path, in timing or in scheduling carries these numbers, or it waits until someone measures it.

1. **Ceiling first.** Before the work, estimate the share of the frame the changed code holds on the critical path (`KLM_CRIT` + `tools/crit_path.py`). A ceiling of a few percent is not worth a hot-path change.
2. **Build the target** (the upstream's reference is its 1.4.3 build) with prepared shaders in that build folder (`tools/prepare_shaders.py`, [docs/building.md](docs/building.md) step 6). Caches copied from another build folder do not count.
3. **A heavy scene** where the change acts (many characters and effects on screen), with `KLM_FPS_LIMIT=360`.
4. **Whole runs, switch off and on, at least two each**, the same save and the same input script, timed by game seconds:

   ```powershell
   .venv\Scripts\python tools\run_game.py build\<ver> --script <input.txt> --seconds 60 --quiet --env KLM_TRACE=perf,sched --env KLM_FPS_LIMIT=360 --env <SWITCH>=0
   ```

   `--quiet` waits until no other game or build runs. Close everything heavy on the machine. Before reading the numbers, check that each run reached the world and showed the same scene.
5. **Report per run**: average fps, 1% low or worst frame, the number of `perf: slow frame` lines, VRAM; `tools/perf_summary.py` condenses the log. Add CPU, GPU, driver, RAM, resolution and the settings you changed.

A switch toggled inside one run on a moving scene gives false results: compare whole runs. Zero new slow frames is a hard bar; an fps gain does not buy back new hitches. Background and the reasons behind each rule: [docs/pr-review-kit/02-performance.md](docs/pr-review-kit/02-performance.md).

### Description and commits

Fill in the pull request template: problem, change, where it reproduces, checks, measurements, what stayed unrun. One topic per pull request.

Commit messages are plain English sentences, without `feat:`, `fix:` or other prefixes. A commit that works on an issue ends its subject with the issue numbers: `Render the handheld scene at the asked size (#12, #34)`.

### Review and merge

The maintainers review every pull request against [docs/pr-review-kit/](docs/pr-review-kit/): read it to see what gets a pull request rejected, which bug classes are already fixed and which invariants a change must keep. A good pull request that misses the project's conventions is fixed up while merging rather than sent back. Part of a pull request can be taken; the reason for what was left out or rejected is posted on the pull request.

Accepted work merges into `main` and reaches players with the next launcher that bundles the engine.

## Coding agents

AI-assisted code is welcome under the top-level [CONTRIBUTING.md](../CONTRIBUTING.md), "AI-assisted contributions": you are responsible for having read and run it, and the pull request says which checks ran on your machine.

Point your agent at the repository's top-level [AGENTS.md](../AGENTS.md) and the engine's [AGENTS.md](AGENTS.md) (Claude Code reaches them through `CLAUDE.md`), this file and [docs/pr-review-kit/](docs/pr-review-kit/) before it writes code. [docs/agents/](docs/agents/) holds what it needs to run the game unattended, measure, and debug: the method, the bug classes of a recompiled Switch game, and a symptom catalog (pages from the upstream, written for its TotK target; each says at its top what was checked on this target). The rules in them bind the agent as they bind you:

- Key values stay out of prompts, logs, commands and files. The agent passes the keys file path to the tools.
- Results in the pull request come from runs that happened. A check the agent did not run is listed as not run.
- The agent writes the description in its own words about this diff. A review kit checklist pasted in with every box ticked tells the reviewer nothing.

## Code and docs

- Code, comments, commit messages and documentation are in English.
- Comments say what the code does and why. Cite our code by file and function, upstream code by project, file and function.
