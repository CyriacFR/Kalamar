# Kalamar engine

The engine of Kalamar: a **static recompiler** that turns the program code of the player's own copy of a Nintendo
Switch game into C, and the **native runtime** (system services, Vulkan renderer, input, audio, LAN play over UDP)
that runs the result on Windows. It targets *Splatoon 3* version 11.2.0. Design: [docs/architecture.md](docs/architecture.md).

The engine is a fork of [totk-pc-recomp](https://github.com/valentderah/totk-pc-recomp) by Valiantsin Dzerakh
(MIT), imported at upstream commit `ae30a61` (0.3.13), with the upstream's changes up to its release 0.4.0 taken in
since ([NOTICE](NOTICE)). The upstream's *Tears of the Kingdom* targets, its documentation and many of its names
(`totk.exe`, the `TOTK_*` setting names, which the documentation gives as `KLM_*`) are still in the code.

> [!IMPORTANT]
> **No game files are included**: no code, data, artwork, fonts or keys of any game. You need your own copy of the
> game and its 11.2.0 update, dumped from your own console, with your console's keys. Instructions for obtaining them
> are not provided.
>
> The game is built from your files on your machine, so the result contains the game. It is for your own use. Do not
> share it.

## Playing

Players install the **Kalamar launcher** ([`launcher/`](../launcher/) of this repository): its install wizard builds the
game from the player's files with this engine, then starts it offline. Nothing else is needed, and no account.

## Building from source

[docs/building.md](docs/building.md) lists the toolchain and the steps ("Building Splatoon 3" for this target).
Everything the build writes stays under `build\` and `generated\`, which git ignores; run `tools/check_content.py`
before a commit.

## Documentation

| Page | Contents |
|---|---|
| [docs/launcher-interface.md](docs/launcher-interface.md) | what a launcher needs: files, command line, environment, status and result files, capabilities |
| [kalamar.ini.example](kalamar.ini.example), [docs/settings.md](docs/settings.md) | player settings; developer settings, diagnostics and test hooks |
| [docs/controls.md](docs/controls.md) | keyboard, mouse (as the gyro), gamepads, remapping |
| [docs/save-profile.md](docs/save-profile.md) | the player profile applied to the save at start, start scenes |
| [docs/lan-play.md](docs/lan-play.md), [docs/lan-relay-protocol.md](docs/lan-relay-protocol.md) | LAN play (off by default) and the UDP relay protocol it can tunnel through |
| [docs/auto-match.md](docs/auto-match.md) | automatic matches for a launcher: settings, status file, result file, rounds, capabilities |
| [docs/building.md](docs/building.md), [docs/installer.md](docs/installer.md) | building, checks, the upstream's Python installer |
| [docs/maxwell.md](docs/maxwell.md), [docs/mods.md](docs/mods.md), [docs/cheats.md](docs/cheats.md) | GPU behavior notes, emulator-format mods, cheats |

## Network

Offline by default: without `KLM_LAN` the game opens no socket and loads no network library,
and it never contacts the game publisher's servers. LAN play, when the player turns it on, reaches only the local
subnet or the relay the player (or the launcher) names ([docs/lan-play.md](docs/lan-play.md)).

## Status

Splatoon 3 11.2.0 builds and plays offline (start-up, character creation, the hub, the test range), LAN play works
on one network and through a relay, and automatic matches for a launcher were measured with two players, alone or
with one or two spectators (three and four copies on one PC). Not verified: matches with more than two players, AMD
graphics cards with this target, Linux with this target. Visual bugs and crashes are still expected.

## Sources and credits

**The upstream.** This engine is a fork of [totk-pc-recomp](https://github.com/valentderah/totk-pc-recomp) by
Valiantsin Dzerakh, under the MIT License, a static recompilation of *Tears of the Kingdom*: imported at upstream
commit `ae30a61` (0.3.13), with the upstream's changes up to its release 0.4.0 taken in since, each with its author
([NOTICE](NOTICE)). From it come the recompiler, the runtime (system services, the console's graphics API on Vulkan,
audio, input, the in-game menu, the Discord presence), the build, extraction and analysis tools, the *Tears of the
Kingdom* targets and most of the documentation. Added by Kalamar: the *Splatoon 3* 11.2.0 target, the offline
answers of the network services, LAN play and its relay client, automatic matches for a launcher, the save profile and
start scenes, the mouse as the gyro, and the menu's Spanish and Japanese texts and ink layout. The upstream's
[LICENSE](LICENSE) is kept unchanged.

**Code, tools and fonts from other projects.** What was taken from each, at which commit and into which files:
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), which wins over this summary.

| Project | License | Used for |
|---|---|---|
| [LAS Recomp](https://github.com/peiikai/LASRecomp) | MIT | the lifter, the code generator, the CPU headers and the differential tests; parts of the host runtime and of the Vulkan back end |
| [MacNativeGames](https://github.com/ly05010419/MacNativeGames) | MIT | the extraction scripts (vendored unmodified), much of the host runtime, the RomFS index, the shader variant list, hactool's Windows patch |
| [d2r-mobile](https://github.com/jhaizhou-ops/d2r-mobile), [nxnative](https://github.com/seedds/nxnative) | MIT | the host thread environment, the graphics API's object layouts and command records, the spin-wait search |
| [SavageWindRecomp](https://github.com/Revivedx/SavageWindRecomp) | MIT | the menu's option rows and style, the performance overlay |
| [Ryujinx](https://github.com/Ryujinx/Ryujinx) | MIT | the shader translator (vendored unmodified, with MacNativeGames' patches) and the audio voice code |
| [deko3d](https://github.com/devkitPro/deko3d) | zlib | the block-linear image layout |
| NVN type definitions by h1k421, via MacNativeGames | MIT | `runtime/nvn.h` |
| Weeping of the Empire; RenoDX | MIT | the TotK menu cursor; the approaches of the mouse as the gyro and of the HDR tonemap (no code copied for those two) |
| musl | MIT | the software fma of the build for CPUs without FMA3 |
| [Dear ImGui](https://github.com/ocornut/imgui) | MIT | the in-game menu |
| [AMD FSR 1](https://github.com/GPUOpen-Effects/FidelityFX-FSR); AMD Display Library headers | MIT | upscaling; the GPU sensors of the performance overlay |
| [SDL 3](https://libsdl.org) | zlib | gamepads, rumble and motion |
| [libopus](https://opus-codec.org), [libvpx](https://chromium.googlesource.com/webm/libvpx), [Zstandard](https://github.com/facebook/zstd) | BSD-3-Clause | the game's Opus audio, VP9 movies and zstd data, decoded on the host |
| [Vulkan Memory Allocator](https://github.com/GPUOpen-LibrariesAndSDKs/VulkanMemoryAllocator); Vulkan headers and loader | MIT; Apache-2.0 OR MIT | GPU memory; the Vulkan API |
| [hactool](https://github.com/SciresM/hactool), with its bundled mbedtls | ISC; Apache-2.0 | decrypting the player's own game containers with the player's own keys |
| Capstone; Unicorn | BSD-3-Clause; GPL-2.0 | disassembly in the analysis tools; the reference CPU of the differential tests, never linked into the game |
| Roboto; JetBrains Mono | Apache-2.0; SIL OFL 1.1 | the menu's fonts |
| TOTK-Ultrawide, a community mod | GPL | numeric layout values in the upstream's TotK-only ultrawide tool: data, no code |

The toolchain a build uses or downloads (MSYS2 UCRT64 or llvm-mingw, Ninja, Python and its packages, the .NET SDK for
the shader translator) and the upstream's installer releases are listed in the second half of the notices.

**Formats and documentation the work relies on.** The cheat-file format of Atmosphère and the Switch emulators' mod
layout ([docs/cheats.md](docs/cheats.md), [docs/mods.md](docs/mods.md)); the GPU behavior notes of
[docs/maxwell.md](docs/maxwell.md), which name their sources (NVIDIA's open-gpu-doc, Mesa's nouveau headers, the
Vulkan and SPIR-V specifications, Ryujinx, deko3d, nxnative's notes, yuzu's progress reports for behavior only);
Discord's documented local IPC for the presence (`runtime/discord_rpc.c`); RFC 1951 (DEFLATE) and RFC 8259 (JSON) in
the runtime's comments.

**No game content.** No code, file or data of any game or of its publisher is included: the player's own copy
provides them at build time. An independent, non-commercial fan project, not affiliated with or endorsed by Nintendo.
*Splatoon*, *The Legend of Zelda*, *Tears of the Kingdom* and *Nintendo Switch* are trademarks of Nintendo, named only
to say which game the software works with.

Most of the code and documentation is written by AI coding agents (Claude Code) under the maintainer's direction; the
maintainer plays the builds and decides what is merged. [AGENTS.md](AGENTS.md) holds the rules they work under.

## License

The project's own code is under the [MIT License](LICENSE), kept from the upstream; third-party code keeps its own
license ([THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). The license grants nothing for any game.
