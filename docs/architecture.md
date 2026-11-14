# Architecture in 10 minutes

A map for a new contributor: the four pieces of Kalamar, what each one does, how they talk, and where to read more.
Words in *italics* are in the [glossary](glossary.md).

```
  the player's PC                                                              anywhere (optional)
 +-----------------------------------------------------------------+         +---------------------------+
 |  launcher (launcher/)              engine build (engine/)       |  HTTPS  |  community service        |
 |  Tauri: Rust back end  -- builds --> the native game            | <-----> |  (kalamar-server)         |
 |         + React UI     -- starts -->   (recompiled code         |  + WS   |  accounts, rooms, queues, |
 |                        <-- status,     + runtime)               |         |  results, ratings         |
 |                            result --      |                     |         +---------------------------+
 |                                           | UDP (only when LAN play is on)        | sealed room codes
 +-------------------------------------------|---------------------+         +---------------------------+
                                             +-------------------------------> |  relay (kalamar-relay)    |
                                                                               +---------------------------+
```

## 1. The engine (`engine/`)

The engine turns the program of the player's own copy of the game into a native Windows program. It has two halves.

**The static recompiler** (`engine/recomp/`, Python). The game's code is ARM64 machine code for the console, in the
*ExeFS* of the player's dump. The recompiler reads its modules, finds every function and jump table, and *lifts* each
instruction to equivalent C, applying the per-build patches of `recomp/patches-*.txt`. Its output, the generated C,
carries a marker so that it can never be committed by mistake. `recomp/targets.py` knows each supported build by the
hashes of its executables, so a wrong file is refused before any long step.

**The runtime** (`engine/runtime/`, C and some C++). The generated C runs on top of it. The runtime gives the game what
the console gave it:

- memory laid out as the game expects, threads, and the console's system calls (`kernel.c`, `svc.c`);
- the system services the game asks for over IPC, and host versions of library functions (`ipc.c`, `hostfn.c`);
- graphics: the console's graphics API (*NVN*) implemented on Vulkan (`nvn_*.c`, `vk_*.c`); the game's GPU shaders
  are translated to SPIR-V by a separate tool built from Ryujinx's shader recompiler (`engine/tools/xlate/`);
- audio, input (keyboard, mouse as the gyro, gamepads through SDL 3), the game's files read from the player's merged
  *RomFS* image;
- the in-game settings menu (Dear ImGui) and the optional Discord presence of a game started on its own, both from the
  upstream project and extended here;
- additions of this project: the *save profile* applied at start, LAN play over UDP and the *relay* client
  (`lan_net.c`), *automatic matches* driven by a launcher (`auto_match.c`).

`tools/build_game.py` compiles the generated C and the runtime with Clang and Ninja into the game executable. The
engine's own design notes: [engine/docs/architecture.md](../engine/docs/architecture.md).

**What it never does**: contact the publisher, resolve a host name, or open a socket when LAN play is off.

## 2. The launcher (`launcher/`)

The program players install. A Tauri 2 app: a Rust back end (`launcher/src-tauri/src/`) and a React + TypeScript
interface (`launcher/src/`) in a WebView2 window, talking through typed commands (`src/api.ts`, `commands.rs`).

- **The install wizard** (`installer/`) builds the game on the player's PC. The engine's source is embedded in the
  launcher (`resources/runtime-src.tar.gz`, made by `scripts/bundle-runtime.ps1` from `engine/`), with two prebuilt
  helper programs (`resources/kalamar-tools.tar.gz`: hactool and the shader translator). The wizard checks the
  player's files, downloads the build tools it lists (pinned by SHA-256), extracts the ExeFS, merges the base game and
  its update into one RomFS image, runs the recompiler, compiles, and runs a self test. Every step resumes after a
  failure, and an update redoes only what its inputs changed.
- **Launching** (`launch.rs`, `startmode.rs`): the game is started with environment variables (`KLM_*`; an older
  build reads only the older names, which the launcher sets too) that choose the start scene, the profile and, for a
  match, everything the match needs. The list is the engine's
  [interface for a launcher](../engine/docs/launcher-interface.md).
- **The player's data** (`profile.rs`, `game/`): the Locker edits a profile that the engine applies to the save; names
  and icons are read from the player's own RomFS at run time and cached on the PC, never shipped.
- **Settings** (`ini.rs`, `settings.rs`): the game's settings file, edited from a schema
  (`resources/settings-schema.json`).
- **The community client** (`proto/`, page `src/community/`): a data-driven client of the protocol (next section). It
  shows only what a server declares.
- **Self-update** (`update/`): signed releases from GitHub ([launcher/docs/releasing.md](../launcher/docs/releasing.md)).

In depth: [launcher/README.md](../launcher/README.md) and its documentation pages; every network access is listed in
[launcher/docs/network-accesses.md](../launcher/docs/network-accesses.md).

## 3. The server repository (`kalamar-server`)

A separate repository, so that anyone can run or rewrite a server without the game-side code.

| Crate | What |
|---|---|
| `kalamar-protocol` | the normative pieces as pure functions: match contract helpers, result validation, the result decision |
| `kalamar-service` | the reference service: HTTPS JSON API, the event WebSocket, matchmaking, decisions, ratings, an admin command line; SQLite storage |
| `kalamar-relay` | the UDP relay matches run through; also embeddable in the service for small servers |
| `kalamar-testkit` | simulated launchers and a simulated game, and the conformance suite any server can be tested with |

## 4. How the pieces talk

| Between | How | Specified in |
|---|---|---|
| launcher and engine | the launcher starts the game with environment variables and a command line; the game writes a *status file* at each change of state and a *result file* at the end; `--list-match-options` prints the engine's capabilities | [engine/docs/launcher-interface.md](../engine/docs/launcher-interface.md), [engine/docs/auto-match.md](../engine/docs/auto-match.md) |
| launcher and service | HTTPS + JSON, one WebSocket for events; discovery at `/.well-known/kalamar-server.json`, then `GET /v1/meta`; protocol 1.1, revision 1.1.13 | the server repository, `docs/protocol/` (summary: [write-a-server.md](write-a-server.md)) |
| game and relay | UDP, the relay protocol version 1: a stateless handshake, then the game's own LAN datagrams forwarded inside a room | [engine/docs/lan-relay-protocol.md](../engine/docs/lan-relay-protocol.md) |
| service and relay | nothing at run time: a relay bound to a service opens only room codes sealed with their shared secret | the server repository, `docs/protocol/relay.md` |

**A community match, end to end.** The service forms a match (from a room or a queue) and gives each launcher a
*match object*: the relay, an unguessable room code, the players' member names, the rules. Each launcher checks it
against what its engine can do, resolves the relay's name, and starts the game for that one match. The games meet in
the relay room through the game's own LAN mode, play, and write their result files; each launcher uploads its file,
and the service cross-checks the reports under the *result policy* it declared, then updates ratings. A player who
leaves a match while it is played is reported as such (`left-by-player`, with the game's `in_play`), and the policy may
make that player's team lose by *forfeit* (1.1.11); a crash or a lost connection never does. The service
never talks to a game, and a game never talks to the service.

## 5. Where the player's files go

They stay on the player's PC. The wizard only reads the dumps and the keys file; the extracted code, the merged RomFS,
the generated C and the built game live in the install folder. Nothing of the game is uploaded, and nothing of it is
in this repository: the content checks (`scripts/check-content.py`, `engine/tools/check_content.py`) refuse it.

## 6. Where to start reading code

| You want to... | Start at |
|---|---|
| fix a screen of the launcher | `launcher/src/pages/`, `launcher/src/community/`, with `npm run dev:mock` |
| change what the launcher sends a server | `launcher/src-tauri/src/proto/` |
| change a step of the install wizard | `launcher/src-tauri/src/installer/steps.rs`, `installer/mod.rs` |
| add a setting | `engine/runtime/cfg.c`, `engine/docs/settings.md`, `launcher/src-tauri/resources/settings-schema.json` |
| fix a rendering bug | `engine/runtime/vk_*.c`, and [engine/docs/agents/symptoms.md](../engine/docs/agents/symptoms.md) first |
| change how a match is played | `engine/runtime/auto_match.c` and [engine/docs/auto-match.md](../engine/docs/auto-match.md) |
| change the protocol | the server repository's `CONTRIBUTING.md`, "Changing the protocol" |
