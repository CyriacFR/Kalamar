# Third-party notices

This repository has two parts, `engine/` and `launcher/`; each lists what it takes from others, with the source, the exact version or
commit, the license and the files concerned.

| Part | Notices | License of the project's own code |
|---|---|---|
| `engine/` | [engine/THIRD_PARTY_NOTICES.md](engine/THIRD_PARTY_NOTICES.md) and [engine/NOTICE](engine/NOTICE): the upstream totk-pc-recomp, LAS Recomp, MacNativeGames, d2r-mobile, nxnative, SavageWindRecomp, Ryujinx's shader translator, deko3d, Dear ImGui, AMD FSR 1, SDL 3, libopus, libvpx, Zstandard, Vulkan Memory Allocator, musl, the fonts of the in-game menu, and the tools the build uses or downloads | MIT ([engine/LICENSE](engine/LICENSE)) |
| `launcher/` | [launcher/THIRD_PARTY_NOTICES.md](launcher/THIRD_PARTY_NOTICES.md): React, Tauri, the interface fonts, the Rust crates, and what the install wizard downloads on the player's machine | MIT ([launcher/LICENSE](launcher/LICENSE)) |
| `launcher/kit-tools/` | the install wizard's helper scripts (extraction, RomFS merge, save tool): this project's own code; they import the engine's vendored MacNativeGames scripts (MIT) at run time | MIT |
| `docs/` | this project's own documentation | MIT ([LICENSE](LICENSE)) |

License texts shipped with third-party files stay next to them (for example
`engine/tools/vendor/ryujinx/LICENSE.txt`, `engine/tools/vendor/macnativegames/LICENSE`,
`engine/runtime/third_party/imgui/LICENSE.txt`, `engine/runtime/shaders/ffx/LICENSE.txt`,
`engine/runtime/ui/fonts/LICENSE-*.txt`). No GPL code is included ([docs/LEGAL-NOTES.md](docs/LEGAL-NOTES.md),
section 5). Nothing in these licenses covers any game.

Every Rust crate and npm package of the launcher, with its license, is listed in
[docs/third-party-dependencies.md](docs/third-party-dependencies.md), generated from the lock files and checked by CI.
