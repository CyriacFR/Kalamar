# Glossary

The words this project uses, in alphabetical order. The server repository has its own glossary for the protocol's
terms; the main ones are repeated here.

**Activity** — something a community server invents and describes as data (a tournament, a challenge, a themed
evening); the launcher draws it from a closed list of components, never from code sent by the server.

**Area** — a part of the community protocol a server may implement (`rooms`, `queues`, `friends`, `ratings`...). A
server declares its areas; the launcher hides every other one and never calls it.

**Automatic match** — the engine's mode for one match started by a launcher: the game enters a LAN room by itself,
plays, writes its status and result files and closes. [engine/docs/auto-match.md](../engine/docs/auto-match.md).

**Bundle** (runtime bundle) — the engine's source embedded in the launcher (`runtime-src.tar.gz` and its manifest),
which the install wizard compiles on the player's PC.

**Capabilities** — what an engine build can do in a match, printed by `--list-match-options` and forwarded by the
launcher to a server, so that a server offers only matches every player's game can play.

**Community server** — a server run by anyone for their community: a *service* and one or more *relays*. None is
built in, suggested or run by this project.

**Conformance suite** — the tests of `kalamar-testkit` that check a server against the protocol, for the areas it
declares.

**Dump** — the player's own copy of the game (and of its update), read from the player's own console. Kalamar never
provides one and explains nowhere how to make one.

**ExeFS** — the part of the game's files that holds its program code; the recompiler's input.

**Forfeit** — a match lost by the team of a player who left it while it was being played, when the server's result
policy says so (protocol 1.1.11); a crash or a lost connection is never a forfeit.

**Generated C** — the recompiler's output: the game's code as C. It is derived from the game, stays on the player's
PC, and carries a marker that the content checks refuse.

**Install wizard** — the launcher's screens that build the game from the player's files (checks, tools, extraction,
recompilation, compilation, self test).

**LAN play** — the game's own local-network mode, off by default. The engine can carry it over a *relay* so that
players on different networks meet.

**Lift, lifter** — translating one ARM64 instruction (or function) into C; the part of the recompiler that does it.

**Locker** — the launcher page where the player sets a name, a character, gear and a weapon (the *profile*).

**Match contract** — what a server must put in a match object and what the launcher passes to the game for it
(`lan-auto-match`, versions 1 and 2).

**Member name** — the name a player has inside one match (1 to 10 characters), chosen by the server.

**Mock mode** (simulation mode) — `npm run dev:mock`: the launcher's interface in a browser on made-up data, with no
game, no server and no back end.

**NVN** — the console's graphics API, which the runtime implements on Vulkan.

**Probe** — a self test mode of the game executable (`--selftest`, `--probe-*`): it needs a built game.

**Profile** — the player's choices from the Locker (`profile.json` of the launcher), applied to the game's save when
the game starts.

**Relay** — a small UDP forwarder (`kalamar-relay`): players' games send their LAN datagrams to it, and it forwards
them only between members of the same room. No accounts, nothing on disk.

**Result file** — the JSON file the game writes at the end of an automatic match (mode, stage, teams, points,
per-player statistics), which the launcher uploads unchanged.

**Result policy** — how a server decides a match from the players' result files (how many must agree, what a
spectator's report weighs, whether leaving a match in play loses it); declared in advance and shown to players.

**RomFS** — the part of the game's files that holds its data (models, textures, texts). The wizard merges the base
game's and the update's into one image on the player's PC.

**Room code** — the secret, unguessable name of a match's room on a relay. A relay bound to a service opens only codes
*sealed* with their shared secret.

**Runtime** — the engine's C and C++ code that the generated C runs on: memory, system calls, services, graphics,
audio, input, network.

**Save profile** — the engine's side of the profile: a JSON file the runtime applies to the encrypted save at start.
[engine/docs/save-profile.md](../engine/docs/save-profile.md).

**Service** — the HTTPS part of a community server: accounts, rooms, queues, results, ratings (`kalamar-service` is
the reference one).

**Status file** — the JSON file the game rewrites at each change of state during an automatic match (booting, lobby,
in match, results...), which the launcher mirrors to the server.

**Target** — one build of a supported game that the recompiler knows (its version, the hashes of its executables, its
patches): `recomp/targets.py`.

**Upstream** — totk-pc-recomp, the MIT project the engine was forked from.
