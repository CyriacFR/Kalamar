# Writing your own server

You can run the reference server of the server repository (`kalamar-server`) as it is, configure it into a very
different offering, or write your own server in any language. The protocol was designed so that **the only limits
are what the game itself can do**: your server chooses its formats (only 1v1 if you like), modes, stages, rating
system, tournaments, leagues, themed evenings and rules, and the Kalamar launcher shows them without any change on
its side.

This page starts from zero and stays readable on its own. The **normative** specification is in the server
repository, `docs/protocol/` (version 1.1, currently revision 1.1.13, with its OpenAPI file); where this page and the specification differ, the
specification wins.

## 1. The pieces

```
                    HTTPS (JSON API) + one WebSocket                        UDP (relay protocol v1)
  launcher  <------------------------------------------>  your service      relay  <----------------> game
  (the player's menu)                                     (accounts, rooms,  (forwards datagrams
     |                                                     queues, results)   inside a room)
     |  starts the game with KLM_* settings for one match,
     |  reads its status and result files
     +-----------------------------------------------------------------------------------------> game
```

- **Launcher**: the player's menu. A data-driven client: it shows only the areas, queues, room options, ratings and
  rules your server declares, and renders your data without interpreting it.
- **Your service**: an HTTPS/JSON API plus one WebSocket per logged-in launcher. It owns accounts, rooms, queues,
  matchmaking, the rules of every match, the decision on every result, ratings and moderation.
- **Relay**: a small UDP forwarder with no accounts and no database. Each match gets its own unguessable room on it.
  You can run the reference relay (`kalamar-relay`) or any implementation of the relay protocol
  ([engine/docs/lan-relay-protocol.md](../engine/docs/lan-relay-protocol.md)). **It is UDP: it cannot go through an
  HTTP reverse proxy.**
- **Game**: started by the launcher for one match. It plays the match over its own LAN mode, tunnelled through your
  relay, then writes a result file. It never talks to your service: the launcher does.

## 2. What a match is, seen from the game

The game has an **automatic match** mode ([engine/docs/auto-match.md](../engine/docs/auto-match.md)): started with
a few environment variables, it creates or joins a private battle by itself over the relay, plays one match (or
several rounds in the same room), writes a status file at every change of state and a result file at the end, and
closes. Your server never sets these variables itself; it fills a `game` block in the match object and the launcher
turns it into them ("match contract `lan-auto-match`", versions 1 and 2):

| The server decides (match object) | The launcher passes to the game |
|---|---|
| the relay (`host`, `port`) and an unguessable `room` | `KLM_LAN=relay:<IPv4>:<port>` (the launcher resolves the name), `KLM_LAN_ROOM`, `KLM_MATCH_ID` |
| each player's `member_name` (1 to 10 characters, unique in the match) | `KLM_LAN_NAME`, `KLM_NICKNAME` |
| who hosts | `KLM_MATCH=host` or `join`, `KLM_MATCH_HOST` (guests) |
| the rules string: `players=<n>,teams=<name>:<A\|B>,...,mode=<mode key>[,stage=<scene>][,<room setting>=0\|1][,results_hold=<s>]` | `KLM_MATCH_RULES`, verbatim |
| step time limits (optional) | `KLM_MATCH_TIMEOUTS` |
| an imposed loadout per player, team or everyone (contract 2, optional) | `KLM_MATCH_LOADOUT` (a weapon and gear profile applied to the match's copy of the save, never to the player's) |
| more rounds in the same room (contract 2, series) | `KLM_MATCH_CMD` (a command file: `next ...`, `quit`) |

The launcher also sets the status and result file paths, `KLM_START=lobby-lan`, `KLM_MATCH_EXIT=1`,
`KLM_MATCH_ON_ERROR=exit` and `KLM_MATCH_REVEAL=intro` (the window stays hidden until the stage's intro).

**What the game can and cannot do**, so you only offer feasible things: 2 to 8 players, at most 4 per team, uneven
teams; the five modes of the game's private battles, by key; a chosen stage (by its scene identifier) or the game's
draw; the room's four other settings; a hold time on the result screens; rounds in one room (about 20 s from the
command to the next intro, against about 50 s for a fresh start). **Not possible**: setting the match duration, a
password or session name (the relay room is the isolation), a third team, more than 8 players, 1-player matches,
weapon rules enforced by the game itself (an imposed loadout is applied by the engine and checked in the results).
The engine has spectator seats and the launcher declares them (`engine.spectator_seats`); spectators were not
verified end to end with a server.

**Mode, stage, weapon and gear identifiers come from each player's own copy.** No list of them is published
anywhere: the engine prints them from the player's game files (`--list-match-options`), the launcher forwards that
list to your server after login (`PUT /v1/me/game`, with the player's capabilities), and your server uses the
identifiers as opaque strings (it may give them display names of its own). A server that never chooses a stage or a
loadout needs none of this.

**What comes back.** The launcher mirrors every status change (`booting`, `lobby`, `waiting-for-players`, `in-room`,
`loading-stage`, `in-match`, `results`, `finished`, `error` with a stable code) and uploads the result file as the
game wrote it: mode, stage, winner, each team's points and each player's name, team, paint, kills, assists, deaths,
specials, disconnection flag, and (version 2) weapon and gear. Every participant's file agrees on all these shared
fields, so your server cross-checks the reports of one match under a **result policy** it declares in advance. A
player who leaves a match is reported as `left-by-player`, with the game's `in_play` when the match was being played;
since 1.1.11 the policy may make that player's team lose by forfeit (`leave`, the server repository's
`docs/protocol/leaving.md`).

**Trust.** Results are computed on the players' machines. Cross-checking catches one player lying alone; it cannot
catch players who collude, or a modified game that lies to everyone during the match. Say so on your rules page;
moderation is the remedy.

## 3. Conformance levels

The protocol is split into **areas**. Every server implements the **core**: discovery, `GET /v1/meta`, at least one
login, the account (display name, settings, export, erasure), the event WebSocket, and the match lifecycle (launch,
status mirror, result intake, decision). The optional areas are `rooms`, `queues`, `parties`, `friends`, `presence`,
`ratings`, `leaderboards`, `stats`, `moderation`, and since 1.1 `activities`, `series`, `badges`, `profiles`. You
declare the areas you implement in `GET /v1/meta`; the launcher hides every other area and never calls it.

A server is **conformant at its level** when the core and every area it declares pass the conformance suite. A
server with private rooms only, a server with only a 1v1 ranked ladder, and a server with everything are all
conformant.

## 4. The smallest useful server

The core plus one way to create matches. In order:

1. **Discovery.** Serve `/.well-known/kalamar-server.json` (name, operator, API address, protocol versions, relay) on
   HTTPS, and `GET /v1/meta`: `protocol.versions: ["1.0"]` (or `"1.1"`), `features: ["rooms"]`,
   `game_contract: {"id": "lan-auto-match", "versions": [1]}`, one login provider, one result policy, limits, rules.
2. **Errors.** One JSON error shape; malformed input answers 4xx, never 5xx.
3. **Login.** The simplest provider is `direct` with one `display_name` field: create an account, return an access
   and a refresh token (store only their hashes), implement refresh and logout.
4. **Account.** `GET`/`PATCH /v1/me`, `PUT /v1/me/game` (you may ignore the body), `GET /v1/state`, export and erase,
   `GET /v1/users/{id}`.
5. **Events.** The WebSocket: `hello`, heartbeat, numbered events. Answering every resume with `resumed: false` is
   conformant (launchers then refetch the state).
6. **Rooms.** Declare the creation form (for example a `teams` field with the formats you allow); implement create,
   join by code, teams, ready, start.
7. **Matches.** When a room starts, create a match object whose `game` block follows the table above: a fresh room
   code of at least 120 random bits for your relay, member names, the rules string. Accept `launch`, `status` and
   `result` uploads and decide with your declared policy (the reference policy and its decision procedure are in
   the specification, `api.md`, "Result decision").
8. **A relay**, on an open UDP port, named in your matches.

Storage can even live in memory for a small group (after a restart, tokens are unknown and launchers log in again).
Everything else — friends, parties, queues, ratings, leaderboards, statistics, moderation, activities — is optional:
declare an area only when all of it works. The specification's `writing-your-own-server.md` has two complete worked
examples (a private-rooms-only server and a 1v1 ladder with its own points system) and four servers that only the
imagination limits (an elimination tournament, a league with stage pick and ban, an imposed-weapons evening, a clan
server), none of which needs a launcher change.

## 5. Testing it

The server repository has a test kit, `kalamar-testkit`, that plays several simulated launchers at once (it never
starts the game: it writes status and result files exactly as the game does).

```sh
# a simulated match in a private room, one of the four players lying about the result
kalamar-testkit simulate --api http://127.0.0.1:8080 --players 4 --cheat 1
# the conformance suite, for the areas you declare (or --areas core,rooms)
kalamar-testkit conformance --api http://127.0.0.1:8080 --max-wait 5
kalamar-testkit conformance --api https://test.example.org/api --tokens tokens.txt --json report.json
```

With a `direct` login it creates its own accounts; otherwise give it test accounts' tokens. **Run it only against a
test instance**: it creates accounts, rooms and matches, files a report and erases accounts.

Then try the real launcher: it accepts plain `http` for a server on this computer, so type `127.0.0.1:8080` in *Add a
server*. The launcher's own tests can drive a local server too ([launcher/docs/community-client.md](../launcher/docs/community-client.md),
"Tests").

## 6. Rules every server keeps

From the specification (`extending.md`): a server never asks a launcher to run code, scripts or HTML, or to load a
URL without the player's click; never makes the game connect anywhere but to the relay of a match; never asks to
disable TLS or certificate checks; never handles money (the protocol has no payment, shop or currency); never uses
game content (it names game things only by the identifiers the players' copies give). And it stores as little
personal data as it can: the specification's privacy section and the server repository's `PRIVACY-TEMPLATE.md`.

## 7. Writing another launcher instead

The launcher side is specified too: the specification's `match-contract.md`, `match-contract-2.md`,
`server-card.md` and `for-launcher-implementers-1.1.md`, and the engine's side —
files, command line, environment, status and result files, capabilities — in
[engine/docs/launcher-interface.md](../engine/docs/launcher-interface.md).

## 8. What has been verified

End to end with the real game: two games on one PC with the reference server and relay on loopback (private 1v1
rooms on a chosen mode and stage, a ranked duel from a queue, a best-of-3 event, a game killed during a match). **Not
verified**: more than two players in one match, spectators, AMD graphics cards, a real Discord login (the OAuth flow
is tested against a local stand-in only), large numbers of players on one server.
