# Documentation

Everything written about Kalamar, by reader. New here? Start with the [README](../README.md), then the page for you.

## Players

| Page | What |
|---|---|
| [Getting started](getting-started.md) | the first install: what you need, the install wizard step by step, playing offline, settings, where files are, what to do when something fails |
| [Adding a community server](servers.md) | what a server adds, what the launcher sends and never sends, privacy |
| [Security model](../launcher/docs/security-model.md) | what the launcher protects you from, what it cannot, the risks that remain when you play online |
| [Security policy](../SECURITY.md) | how to report a vulnerability privately, and what is in scope |
| [Engine: controls](../engine/docs/controls.md) | keyboard, mouse (as the gyro), gamepads, remapping |
| [Engine: player settings](../engine/kalamar.ini.example) | every setting of the game's settings file, explained |
| [Support](../SUPPORT.md) | where bugs and ideas go, and what the project does not offer |

## Running a community server

The reference server, its installer and the operator guides are in the server repository,
[kalamar-server](https://github.com/CyriacFR/kalamar-server):
`README.md` (what it offers, the installer, the operator's reference: configuration, reloads, the admin command
line, backups), `docs/operator-first-steps.md` (from an empty machine to a server launchers can add) and
`PRIVACY-TEMPLATE.md`. From the player's side: [servers.md](servers.md), "For operators".

## Writing a server or another launcher

| Page | What |
|---|---|
| [Writing your own server](write-a-server.md) | from zero, readable on its own; the normative specification is in the server repository, `docs/protocol/` |
| [The engine's interface for a launcher](../engine/docs/launcher-interface.md) | files, command line, environment, status and result files, capabilities |
| [Automatic matches](../engine/docs/auto-match.md) | how the game plays one match for a launcher: settings, status file, result file, rounds |
| [LAN play](../engine/docs/lan-play.md) and [the relay protocol](../engine/docs/lan-relay-protocol.md) | the game's network side, off by default |

## Contributors

| Page | What |
|---|---|
| [Contributing](../CONTRIBUTING.md) | rules, prerequisites, building, tests, style, how to propose a change |
| [Instructions for coding assistants](../AGENTS.md) | what an AI assistant and its user must know: the map, the checks CI runs, the rules, AI-assisted contributions ([engine/AGENTS.md](../engine/AGENTS.md) for the engine) |
| [Architecture in 10 minutes](architecture.md) | the engine, the launcher, the server and the protocol, and how they talk |
| [Glossary](glossary.md) | the words of this project |
| [Engine: architecture](../engine/docs/architecture.md) | how the game's code becomes a native program: lifter, host, graphics, toolchain |
| [Engine: building from source](../engine/docs/building.md) | the toolchain, building with your own files, the checks |
| [Engine: contributing](../engine/CONTRIBUTING.md) | checks by area, proof and performance measurements for engine changes |
| [Engine: settings reference](../engine/docs/settings.md) | developer settings, diagnostics, test hooks |
| [Engine: save profile](../engine/docs/save-profile.md) | the character profile applied to the save, start scenes |
| [Engine: mods](../engine/docs/mods.md), [cheats](../engine/docs/cheats.md), [profile-guided build](../engine/docs/pgo.md) | the engine's other features |
| [Engine: GPU behavior notes](../engine/docs/maxwell.md) | what the Vulkan path must reproduce |
| [Engine: review kit](../engine/docs/pr-review-kit/README.md) and [agents' guides](../engine/docs/agents/) | how engine pull requests are reviewed; running and debugging the game unattended (from the upstream project: each page says at its top what was checked on this fork) |
| [Launcher: README](../launcher/README.md) | building and testing the launcher, its layout, the index of its pages |
| [Launcher: network accesses](../launcher/docs/network-accesses.md), [security model](../launcher/docs/security-model.md) | every network access, what the launcher protects from, the risks that remain |
| [Launcher: install wizard](../launcher/docs/install-wizard.md) | the build from the player's files: steps, resume, updates, the bundled archives |
| [Launcher: launching](../launcher/docs/launching.md), [game data and profile](../launcher/docs/game-data.md), [interface](../launcher/docs/interface.md) | play modes and the command line, names and icons from the player's files, navigation |
| [Launcher: community client](../launcher/docs/community-client.md) | servers, the protocol client, private matches, tests against a local server |
| [Launcher: releases and self-update](../launcher/docs/releasing.md) | signed releases, the keys, the manual release workflow |
| [Launcher: Discord presence](../launcher/docs/discord-presence.md) | what the optional presence shows, and the maintainers' setup |

Some engine pages come from the upstream project and describe its original game target (the upstream's installer,
the tester guide); they are kept for the engine's history and its other targets.

## Legal

| Page | What |
|---|---|
| [Legal notes](LEGAL-NOTES.md) | what the repository contains and does, names, network, licenses (not legal advice) |
| [Third-party notices](../THIRD_PARTY_NOTICES.md) | the components each part takes from others, with their licenses |
| [Third-party dependencies](third-party-dependencies.md) | every Rust crate and npm package of the launcher, with its license (generated) |
