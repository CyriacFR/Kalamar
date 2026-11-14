# The engine's interface for a launcher

What a program that starts the game needs to know: the files it finds, the command line, the environment it sets,
what the game writes back. The Kalamar launcher ([`launcher/`](../../launcher/README.md) of this repository) is one such program; this
page is for anyone who writes another one, or a script. It summarizes and links the detailed pages; where they and
this page differ, the detailed page and the code win.

Stable means: documented here or in the linked page with a version, kept compatible within that version. The run log
is **not** a stable interface (its lines are for people), except the few lines named below as read by the launcher.

## 1. Files of a built game

A game root `<W>` (the install wizard's is `<install>\work\splatoon3-11.2.0`):

| Path | What |
|---|---|
| `<W>\build\splat3.exe` | the game, built on the player's machine from the player's own files (the target's `exe`, [building.md](building.md), "Building Splatoon 3") |
| `<W>\romfs\romfs.bin` + `index.json` | the player's merged RomFS image and its index; read in place, never unpacked |
| `<W>\exefs\` | the player's extracted ExeFS (input of the build) |
| `<W>\userdata\` | the player's data: `profile\` (saves), `logs\`, `shader-cache\`, `pipeline-cache.bin` |
| `<W>\..\xlate\totk_xlate.exe` | the shader translator (`tools/build_xlate.py`), named by `KLM_XLATE` |
| `kalamar.ini` next to the executable | settings ([kalamar.ini.example](../kalamar.ini.example) lists every key; an older `totk.ini` is read when there is no `kalamar.ini`) |

Everything under `<W>` except `userdata` and the settings file is derived from the player's game and must stay on
that machine.

## 2. Starting the game

The Kalamar launcher starts it this way:

- working directory `<W>\userdata`;
- arguments `--profile <W>\userdata\profile` (the save folder), then any extra arguments;
- stderr to a log file of the launcher's choice (`<W>\userdata\logs\run-<time>.log`), stdout next to it;
- the environment of section 3 on top of the inherited one, after removing every variable the launcher owns, so a
  stray `KLM_LAN` in the shell cannot change a launch (the list is `OWNED_ENV` in the launcher's `launch.rs`).

Other command-line modes (no window, used by tools and the install wizard): `--list-match-options` (section 5),
`--selftest`, `--probe-profile`, `--probe-lan` and the other probes of [building.md](building.md), "Checks".

Exit codes: 0 after a normal close (window closed, *Quit to launcher*, or the end of an automatic match with
`KLM_MATCH_EXIT=1`); 3 when an automatic match stops on an error ([auto-match.md](auto-match.md)); anything else
is a failure or a crash (the launcher reports it with the last lines of the log).

## 3. Environment

Settings are `name = value` lines in the settings file; an environment variable overrides one key for one launch
([settings.md](settings.md) for the developer keys, [controls.md](controls.md) for input). The variables below are
the ones a launcher sets per launch.

| Group | Variables | Reference |
|---|---|---|
| Paths | `KLM_SHADER_CACHE`, `KLM_PIPELINE_CACHE`, `KLM_ROMFS_PROOF_DIR`, `KLM_XLATE`, `KLM_PROFILE_DIR` | [settings.md](settings.md), "Paths and files" |
| Start scene | `KLM_START` = `title` (the untouched start), `plaza` (free play in the hub), `range` (the test range), `lobby-lan` (the local lobby in LAN mode) | [save-profile.md](save-profile.md), "Start scene" |
| Player profile | `KLM_SAVE_PROFILE` (a `profile.json`: appearance, gear, weapon, name card, first-launch flags), `KLM_PLAYER_NAME`, `KLM_NICKNAME` | [save-profile.md](save-profile.md) |
| LAN play | `KLM_LAN` (`lan`, `lan:<ipv4>/<prefix>` or `relay:<ipv4>[:<port>]`), `KLM_LAN_ROOM`, `KLM_LAN_NAME`, `KLM_LAN_BIND`, `KLM_NET_LOG` | [lan-play.md](lan-play.md), [lan-relay-protocol.md](lan-relay-protocol.md) |
| Automatic match | `KLM_MATCH`, `KLM_MATCH_ID`, `KLM_MATCH_RULES`, `KLM_MATCH_HOST`, `KLM_MATCH_STATUS`, `KLM_MATCH_RESULT`, `KLM_MATCH_EXIT`, `KLM_MATCH_ON_ERROR`, `KLM_MATCH_REVEAL`, `KLM_MATCH_TIMEOUTS`, `KLM_MATCH_LOADOUT`, `KLM_MATCH_STAGES`, `KLM_MATCH_CMD`, `KLM_MATCH_SILENT` | [auto-match.md](auto-match.md), "Settings" |
| Window | `KLM_WINDOW_TITLE` (a second local instance gets its own title) | `runtime/` |

The game takes IPv4 addresses only and never resolves names: a launcher resolves a relay's host name itself and
passes the address.

## 4. What the game writes back

- **Status file** (`KLM_MATCH_STATUS`): one JSON object rewritten whole at every change of state (temporary file
  then rename), `"version": 1`; states, errors and fields in [auto-match.md](auto-match.md), "Status file". `role` is
  the player's part in the session (`host`, `join`), `seat` its part in the match (`player`, `spectator`, `unknown`).
- **Result file** (`KLM_MATCH_RESULT`): written only when the game ended a match; version 2 adds `outcome`,
  `round`, `requested`, `settings_ok`, `room`, each player's `weapon` and `gear`, and `loadout`
  ([auto-match.md](auto-match.md), "The result"). A spectator's file has `"seat": "spectator"` and `"outcome":
  "spectated"` (never `completed`): it is a witness of the match, not a player's report; forward it as such or keep it
  ([auto-match.md](auto-match.md), "Spectators").
- **Run log**: free text. The Kalamar launcher reads two kinds of lines to explain a launch to the player: `save
  profile:` (applied, already up to date, refused with the reason) and `automatch:` (match progress when there is no
  status file). Treat them as hints, not as a contract.

## 5. Capabilities: `--list-match-options`

`splat3.exe --list-match-options` prints one JSON object (`"format": "splat3-match-options", "version": 1`) and exits,
without a window and without starting the game: the mode keys, the stages of the game's stage table with their scene
names and labels, the team limits, spectators (`spectators`: how many, in which modes, what was run), the room's other
settings, rounds, result version, loadout support. Mode and stage
names come from the player's own game files at run time (`KLM_LIST_LANG` picks the labels' language); nothing of
that table is in this repository. Members: [auto-match.md](auto-match.md), "Capabilities". A launcher that talks to
a community server forwards this object as it is (`PUT /v1/me/game` of the Kalamar protocol, see the server
repository's `docs/protocol/match-contract-2.md`).

## 6. What the engine never does

- It never contacts the game publisher's servers. Without `KLM_LAN` no socket is opened and no network library is
  loaded; with LAN play it only reaches the local subnet or the relay given to it ([lan-play.md](lan-play.md),
  "Safety").
- It resolves no host names and sends no telemetry.
- It writes the player's save only through the game itself, or through the profile applied at start
  ([save-profile.md](save-profile.md), with a backup); an automatic match plays on a copy of the save.

## 7. Verified and not verified

Measured with the Splatoon 3 11.2.0 build: two players in an automatic match (two copies on one PC, lan and relay
mode), rounds in one session, imposed loadouts on the match's copy of the save, one and two spectators besides two
players (three and four copies, relay mode; the host as a spectator too; a spectator leaving mid-match; rounds with a
spectator). **Not verified**: more than two players in one match, AMD graphics cards with this target (a crash at load and graphical defects were
reported on one AMD card; no fix is confirmed), the Linux back end with this target.
