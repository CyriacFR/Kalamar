# Contributing to Kalamar

Thank you for helping. This repository holds the **engine** (`engine/`: the static recompiler and the native runtime)
and the **launcher** (`launcher/`: the Tauri app players install). The protocol specification and the reference
community server live in the companion server repository (`kalamar-server`), which has its own contributing guide.

Security problems are reported privately: [SECURITY.md](SECURITY.md).

**Conduct.** Be respectful and stay on the subject: the code, the documentation, the bug. Maintainers may edit or
close issues and pull requests, and block accounts, when someone is abusive, harasses another person or keeps
posting off-topic. Abusive content is reported to GitHub with the **Report content** link of the issue, comment or
profile.

- [What is not accepted, and why](#what-is-not-accepted-and-why)
- [The game contacts no one by itself](#the-game-contacts-no-one-by-itself)
- [Prerequisites](#prerequisites)
- [Building and testing the launcher](#building-and-testing-the-launcher)
- [The launcher without the game: simulation mode](#the-launcher-without-the-game-simulation-mode)
- [Engine checks that need no game files](#engine-checks-that-need-no-game-files)
- [Repository checks](#repository-checks)
- [Where things are](#where-things-are)
- [Style](#style)
- [Proposing a change](#proposing-a-change)
- [AI-assisted contributions](#ai-assisted-contributions)
- [Adding a language](#adding-a-language)
- [Sign-off](#sign-off)

## What is not accepted, and why

The project exists only because it ships nothing of the game and asks each player for their own files. Anything that
weakens that is refused, however useful it looks, and is removed from issues and pull requests:

- **Anything derived from the game or from a console**: game files or parts of them, extracted or decrypted data,
  generated C, compiled builds, shaders or shader caches, saves, screenshots or recordings of the game, sounds,
  fonts, texts, and tables of game data copied out of it (lists of names, items, stages, weapons, messages). The same
  for keys, certificates, firmware, serial numbers and network addresses of a console or of the publisher's services.
  What the code may hold are facts needed to run the player's own copy, as described in
  [docs/LEGAL-NOTES.md](docs/LEGAL-NOTES.md) (function addresses and offsets, the hashes that recognize a build): the
  engine's [CONTRIBUTING.md](engine/CONTRIBUTING.md) explains where they go. Identifiers that the engine reads from the
  player's own files at run time stay there.
- **Help to obtain the game, keys or firmware, and ways around protection measures.** The install wizard reads the
  player's own dump with the player's own keys; no change may add a way to get those files from anywhere else, and no
  issue or pull request may link to one.
- **Code copied from a project whose license is not compatible with MIT.** Code may be copied only from projects under
  MIT, BSD, ISC, zlib or Apache-2.0; it keeps its original header and gets a row in the third-party notices of its
  part. Copyleft projects (several console emulators among them) may be read to understand a behavior, never copied.
  [docs/LEGAL-NOTES.md](docs/LEGAL-NOTES.md), section 5, names them.
- **Cheating.** Nothing that gives a player an advantage over other players in LAN or community matches, changes
  what a game writes in its result file, or hides from a server's checks. The engine's support for cheat files
  ([engine/docs/cheats.md](engine/docs/cheats.md)) is for playing alone.
- **The game's brand in the interface.** The launcher shows no logo, artwork or trademark of the game or its
  publisher; names and pictures of in-game items are read from the player's own files at run time and never shipped.
- **Personal data.** No real names, e-mail addresses, user names in paths, IP addresses or unredacted logs in commits,
  issues or pull requests.

An automated check (`scripts/check-content.py`) runs on every pull request; it does not replace your judgment. When in doubt, ask in an issue before you write the code.

## The game contacts no one by itself

The game opens no network socket unless LAN play is turned on, by the player or by the launcher for a match, and then
reaches only the local subnet or the relay it was given (it never resolves a host name). It never contacts the
publisher's servers. The launcher has no telemetry: besides the build tools the install wizard lists and downloads
after the player confirms, and the check for its own updates in published builds, it talks only to the community
servers the player added, and only in the ways listed in [launcher/docs/network-accesses.md](launcher/docs/network-accesses.md).

A change that adds a network access of any kind (a request, a lookup, a datagram, a download, a loaded image or page)
is a big change: open an issue first, and update that list and [docs/servers.md](docs/servers.md) in the same pull
request. Crash reporting, analytics, remote configuration and automatic downloads of content are not accepted.

## Prerequisites

Development happens on **Windows 10 or 11, x64** (the launcher is a Windows app; the engine builds for Windows).

| Tool | Version | For |
|---|---|---|
| Git | any recent | the repository |
| [Node.js](https://nodejs.org) with npm | 24 LTS (tested with 24.16) | the launcher's interface |
| [Rust](https://rustup.rs) through rustup, MSVC toolchain (`x86_64-pc-windows-msvc`) with `clippy` | stable, tested with 1.97.1 | the launcher's back end |
| Microsoft C++ Build Tools ("Desktop development with C++") | 2022 or newer | the linker Rust uses on Windows |
| Microsoft Edge WebView2 runtime | preinstalled on Windows 11 | the launcher's window |
| [Python](https://www.python.org) | 3.12 or newer | the repository checks and the engine's tools |
| Microsoft Edge (or Chrome) | any recent | `npm run shots` only |

Building the **game** (not needed to work on the launcher) needs the engine's toolchain and your own game files:
[engine/docs/building.md](engine/docs/building.md).

## Building and testing the launcher

From a fresh clone, in `launcher/`:

```powershell
npm ci                     # the exact versions of package-lock.json
npx tsc --noEmit           # type check
npm run build              # type check and production build of the interface (dist/)
npm run test:ui            # the interface's logic tests (node --test)
cd src-tauri
cargo test                 # the Rust unit and integration tests (a few hundred; some are ignored and need a server)
cargo clippy --all-targets -- -D warnings
```

`npm run tauri dev` opens the real launcher with hot reload, and `npm run tauri build` makes a release build (it first
bundles `engine/` into the launcher: `scripts/bundle-runtime.ps1`; the program is `src-tauri/target/release/kalamar.exe`).
The real launcher reads and writes its data folder (`%APPDATA%\dev.kalamar.launcher`) and can find a real installation;
at its first start it also takes over the data folder of a pre-release launcher (`dev.splat3recomp.launcher`). When you
try it, set `KALAMAR_APPDATA` to a throwaway folder (it stands for `%USERPROFILE%\AppData`, the WebView's profile
included), or at least `KALAMAR_DATA` to a throwaway data folder: [launcher/README.md](launcher/README.md) ("Develop").

Tests that need more than the repository (`--ignored` tests) are described where they live: the community protocol
tests against a local reference server ([launcher/docs/community-client.md](launcher/docs/community-client.md), "Tests"), the update tests ([launcher/docs/releasing.md](launcher/docs/releasing.md), "Tests"), and the real-data check
of the player's game files (`cargo test --test game_data -- --ignored`, which needs your own install).

## The launcher without the game: simulation mode

Most interface work needs neither the game nor a server:

```powershell
npm run dev:mock           # http://localhost:1430: the interface on made-up data (src/dev/mock/), never in a build
```

The URL chooses the situation: `?game=crash`, `?server=service&page=community`, `?install=running&ist=compile`,
`?lup=available&page=settings&section=about`, `&lang=es`, `&adv=1` (advanced mode), and many more listed at the top of
`src/dev/mock/backend.ts` and in [launcher/docs/interface.md](launcher/docs/interface.md) ("Browser preview"). `?gallery=stages` shows the stage
cards in every state.

With the preview running, `npm run shots -- --out <folder> --lang en,fr` captures every screen listed in
`scripts/ui-shots.list.mjs` with the installed Edge, in both interface modes and two widths. Write them **outside the
repository**; a screenshot is added to the repository only on purpose, shows only the preview's made-up data, and is
reviewed before publication like any other image.

## Engine checks that need no game files

Most engine work needs your own game files, and the engine's [CONTRIBUTING.md](engine/CONTRIBUTING.md) lists the checks
by area. These run anywhere, and CI runs them:

```powershell
cd engine
python tools/check_content.py          # nothing derived from the game in engine/ (its own rules)
python tests/settings_doc_check.py     # every setting is documented
```

With the Python packages `capstone`, `unicorn`, `lz4` and `numpy` (the versions of `engine/tools/requirements.txt`) and
Clang on `PATH` (MSYS2 UCRT64), CI also runs, from `engine/`: `tests/exefs_patches_test.py`,
`tests/cheats_vm_check.py`, `tests/spin_waits_test.py`, `tests/flags_check.py`, `tests/excl_mt.py`,
`tests/diff/atomic.py` and `tests/diff/simd.py` (the last one takes about three minutes).

Nothing of the runtime compiles without the player's game: even the runtime alone (`tools/build_game.py
--make-runtime-lib`) needs the module layout the code generator derives from the game's modules.

## Repository checks

CI also runs four checks of the repository itself; run them before you push:

```powershell
python scripts/check-english.py        # text is in English, outside translations and test data (scripts/english-only.json)
python scripts/check-content.py        # nothing derived from the game, no key or secret, binaries reviewed (scripts/content-rules.json)
python scripts/check-links.py --skip "engine/tools/vendor/**" --skip "engine/runtime/third_party/**"
python scripts/third-party-inventory.py --check   # docs/third-party-dependencies.md matches the lock files
```

`check-english.py` explains its rules at the top of the file. An exception (a translation file, test data that checks
another language, a proper name) goes into `scripts/english-only.json` with its reason.

## Where things are

A ten-minute tour is in [docs/architecture.md](docs/architecture.md). In short:

| Path | What |
|---|---|
| `engine/recomp/` | the static recompiler: ARM64 game code to C |
| `engine/runtime/` | the native runtime: system services, Vulkan renderer, input, audio, LAN play, the in-game menu |
| `engine/tools/`, `engine/tests/` | build, extraction and analysis tools; tests |
| `launcher/src/` | the interface (React + TypeScript): pages, the install wizard, the community client, translations |
| `launcher/src-tauri/src/` | the back end (Rust): launching, settings, the install wizard, the community protocol client, updates |
| `launcher/scripts/` | bundling the engine into the launcher, releases, screenshots |
| `docs/` | guides for players, operators and contributors |

## Style

- **English** everywhere: code, comments, documentation, commit messages, log and error messages for developers.
  Text a player sees goes through the translations (below). One English: American spelling.
- **Comments** say what the code does and why, in words that stand on their own. Cite this project's code by file and
  function, other projects' code by project, file and function.
- **Rust**: `cargo clippy --all-targets -- -D warnings` must pass. The launcher's Rust code is not `rustfmt`-formatted
  yet: format new files with `rustfmt`, and do not reformat lines you do not otherwise change (it hides the real diff).
- **TypeScript**: `strict` mode; `npx tsc --noEmit` must pass. Logic worth testing goes in plain modules tested with
  `node --test` (`*.test.mjs`, listed in the `test:ui` script).
- **C and C++ (engine)**: follow the surrounding code; the engine's [CONTRIBUTING.md](engine/CONTRIBUTING.md) lists what
  the frame path must not do.
- **Python**: standard library where possible; scripts state what they do in their first lines.
- **Commit messages**: a plain English sentence in the imperative that says what the change does for the user or the
  code, without `feat:`, `fix:` or other prefixes, for example `Show the server's player count next to its name`. The
  body, when needed, says why. A commit about an issue ends its subject with its number: `... (#123)`.

## Proposing a change

1. **Open an issue first** for anything big: a new feature, a new dependency, a new network access, a change to the
   engine's interface for launchers, a new setting players see, a redesign. Small fixes can go straight to a pull
   request.
2. **One topic per pull request**, as small as it can be. Branch from `main`; rebase on it before asking for review.
3. **Tests come with the change.** New behavior has a test that fails without it (Rust unit tests, `*.test.mjs` for
   interface logic, the engine's tests and probes); a bug fix shows the bug before and gone after. Say which checks
   you ran and which you could not run (for example those that need game files or hardware you do not have).
4. **Docs come with the change**: the README or guide that describes the behavior, the settings references for a new
   setting, the network list for a new access, and every interface language for new text.
5. Fill in the pull request template. CI must pass. A maintainer reviews; part of a pull request can be taken, and the
   reason for what is left out is written on the pull request.

## AI-assisted contributions

Welcome, under the same rules. The person who opens the pull request is its author: you have read the whole diff, you
ran the checks yourself, and the pull request says in your own words what was run and what was not. An assistant
never invents a test result and never pastes game content into the work "for reference". `Co-Authored-By` lines are
welcome, never required; the `Signed-off-by` line is yours (below). The instructions coding assistants read, with
what they cannot guess from the code, are in [AGENTS.md](AGENTS.md) ([CLAUDE.md](CLAUDE.md) points to it), and
[engine/AGENTS.md](engine/AGENTS.md) for the engine.

## Adding a language

**Launcher** ([launcher/src/locales/index.ts](launcher/src/locales/index.ts) says the same at its top):

1. Copy `launcher/src/locales/en/` to `launcher/src/locales/<code>/` (four files: `app`, `community`, `servers`,
   `install`) and translate the values; keep every `{variable}` and every `|` separator.
2. Add one entry to `LOCALES` in `launcher/src/locales/index.ts` (code, the language's own name, BCP 47 tag, title
   word order).
3. Add the code to `LANGUAGES` in `launcher/src-tauri/src/config.rs`, and to the label maps of
   `launcher/src-tauri/resources/settings-schema.json`.
4. `npx tsc --noEmit` refuses a missing or extra key; `npm run test:ui` checks the variables and separators. Look at
   the screens with `npm run dev:mock` and `&lang=<code>`.

**Engine's in-game menu**: a table per language in `engine/runtime/ui/` (`text_<code>.inc`, `ink_<code>.inc`), an
entry in `engine/runtime/ui_text.c` and an id in `engine/runtime/ui_text.h`; the menu probe in
`engine/runtime/ov_probe.c` fails when a menu string is missing. The menu's fonts must cover the language's
characters.

Translations are the only files in other languages; they are listed in `scripts/english-only.json`.

## Sign-off

Kalamar asks for no contributor license agreement. Each commit is signed off under the
[Developer Certificate of Origin 1.1](https://developercertificate.org/): by adding a `Signed-off-by: Your Name
<you@example.org>` line (`git commit -s`), you state that you wrote the change or have the right to submit it under
the project's license. The name and address are those of your git configuration; a pseudonym you use consistently is
fine.
