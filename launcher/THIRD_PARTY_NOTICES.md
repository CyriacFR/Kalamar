# Third-party notices (launcher)

The launcher's own code is under the MIT License ([LICENSE](LICENSE)). It copies no third-party source file into this
repository: everything below is a dependency fetched by `npm` or `cargo` at the pinned versions of
`package-lock.json` and `src-tauri/Cargo.lock`, and linked or bundled unmodified into the built launcher. A release of
the launcher carries these notices and the license texts of the components it bundles.

The runtime source the launcher bundles (`src-tauri/resources/runtime-src.tar.gz`, also compiled into `Kalamar.exe`) is the engine's own repository
with its own [THIRD_PARTY_NOTICES.md](../engine/THIRD_PARTY_NOTICES.md) (in the combined repository: `engine/`), plus
the helper scripts of `kit-tools/` (this project's own code, MIT).

## Bundled into the launcher's interface (npm `dependencies`)

| Component | Version | License | Use |
|---|---|---|---|
| React, React DOM | 19.3.1 | MIT | interface |
| `@tauri-apps/api`, `@tauri-apps/plugin-dialog` | 2.12.4, 2.8.4 | Apache-2.0 OR MIT | calls into the Rust side, file dialogs |
| Archivo (variable), via `@fontsource-variable/archivo` | 5.2.8 | SIL Open Font License 1.1 | interface font, bundled (no font CDN) |
| Rubik (variable), via `@fontsource-variable/rubik` | 5.2.8 | SIL Open Font License 1.1 | display font, bundled |
| JetBrains Mono, via `@fontsource/jetbrains-mono` | 5.2.8 | SIL Open Font License 1.1 | monospace font, bundled; the engine's in-game menu also uses its ExtraBold face |

The fonts are open fonts from their authors (Google Fonts / JetBrains releases packaged by Fontsource); none comes
from a game.

## Build tools (npm `devDependencies`, not shipped)

Vite 8.3.7 (MIT), `@vitejs/plugin-react` 6.3.0 (MIT), TypeScript 6.0.3 (Apache-2.0), `@tauri-apps/cli` 2.12.4
(Apache-2.0 OR MIT), React type definitions (MIT). Their own dependencies include MPL-2.0 packages (the CSS tooling),
used unmodified at build time only.

## Rust crates linked into the launcher (`src-tauri`)

Direct dependencies, all from crates.io:

| Crate | License | Use |
|---|---|---|
| `tauri`, `tauri-build`, `tauri-plugin-dialog` | Apache-2.0 OR MIT | application shell (WebView2 window, IPC, dialogs) |
| `serde`, `serde_json`, `chrono`, `thiserror`, `futures-util`, `getrandom`, `sha2`, `flate2`, `tar`, `png`, `windows-sys`, `tokio-rustls`, `rustls-platform-verifier`, `reqwest`, `texture2ddecoder` | MIT OR Apache-2.0 | data formats, hashing, archives, PNG output of the player's own icons, Windows calls, HTTPS client |
| `rustls` | Apache-2.0 OR ISC OR MIT | TLS (no OpenSSL) |
| `tokio`, `tokio-tungstenite`, `ruzstd`, `tegra_swizzle` | MIT | async runtime, WebSocket, zstd decoding and texture layout of the player's own game data |
| `minisign-verify` 0.3.0 | MIT | verification of the minisign (Ed25519) signature of the launcher's own release manifests; no dependencies |

Development only (`[dev-dependencies]`, never linked into the launcher): `minisign` 0.10.0 (MIT), for the tests and
the release signing tool `examples/release_sign.rs`, with its dependencies `scrypt`, `salsa20`, `pbkdf2`, `hmac`,
`ct-codecs` (MIT OR Apache-2.0 / MIT) and `rpassword` (Apache-2.0).

The whole graph for Windows (about 310 crates) is under permissive licenses: MIT and/or Apache-2.0 for most, plus
Unicode-3.0 (ICU data crates), ISC (`rustls-webpki`, `untrusted`), BSD-3-Clause (`subtle`, `alloc-*`), Zlib
(`zlib-rs`, `foldhash`), and MPL-2.0 for `cssparser`, `selectors`, `dtoa-short` and `option-ext` (used unmodified;
MPL-2.0 is a per-file license: their source is the published crate). No GPL or LGPL crate is linked. Every crate and
npm package with its license: `docs/third-party-dependencies.md` of the combined repository, generated from the lock
files by `scripts/third-party-inventory.py` and checked by CI.

## Programs shipped ready-built (`src-tauri/resources/kalamar-tools.tar.gz`, also compiled into `Kalamar.exe`)

Built from the engine repository's sources (`tools/build_hactool.py`, `tools/build_xlate.py`) at the commit named in
`kalamar-tools.json`, so a player's PC needs no compiler, .NET SDK, Visual Studio or Git for them. The archive carries
each license text below under `licenses/`, and the wizard copies them to `<install>\tools\licenses\`. Nothing in them
comes from a game.

| Program | Made of | Licenses |
|---|---|---|
| `hactool.exe` | SciresM/hactool at 1d64a83 with its bundled mbedtls 2.6.1; MacNativeGames' Windows changes (`portable.patch`); the engine's `unicode.patch`, `mbedtls-asm-clobbers.patch` and `range.patch` (an NCA read in place inside its NSP) | ISC (hactool), Apache-2.0 (mbedtls), MIT (MacNativeGames, the engine) |
| `totk_xlate.exe` (shader translator) | the engine's `tools/xlate` and patches, Ryujinx's shader recompiler (`tools/vendor/ryujinx`, Ryujinx 1.3.2), the .NET 10 NativeAOT runtime linked in | MIT (the engine, tools/xlate, Ryujinx, .NET runtime; the .NET runtime's own third-party notices are included as `dotnet-runtime-THIRD-PARTY-NOTICES.txt`) |

## Downloaded by the install wizard on the player's machine (not part of this repository, not redistributed)

Each file comes from its project's official download over https and is checked against the SHA-256 pinned in
`src-tauri/src/installer/portable.rs` before it is unpacked into `<install>\tools\` (nothing is installed on the
system). They run as separate programs, or are linked into the game the player builds on their own PC.

| Download | Source | License |
|---|---|---|
| Python 3.14.9, embeddable build | python.org | PSF License |
| numpy 2.5.4, lz4 4.4.5, cryptography 50.0.2, cffi 2.1.1, pycparser 3.11 (wheels) | pypi.org | BSD-3-Clause AND 0BSD AND MIT AND Zlib AND CC0-1.0 (numpy), BSD-3-Clause (lz4, pycparser), Apache-2.0 OR BSD-3-Clause (cryptography), MIT-0 (cffi), as their PyPI metadata states |
| llvm-mingw 20261020 (Clang/LLD 23.1.4, libc++, mingw-w64 headers and CRT) | github.com/mstorsjo/llvm-mingw releases | Apache-2.0 WITH LLVM-exception (LLVM), ZPL-2.1 and public domain (mingw-w64), MIT (winpthreads) |
| Ninja 1.13.2 | github.com/ninja-build/ninja releases | Apache-2.0 |
| SDL 3.4.20 (DLL and MinGW development files) | github.com/libsdl-org/SDL releases | Zlib |
| Opus 1.6.1, libvpx 1.17.0, Vulkan headers and loader import library 1.4.363, Vulkan Memory Allocator 3.5.0, zstd 1.5.7 (MSYS2 UCRT64 packages, unpacked, MSYS2 itself is not installed) | repo.msys2.org | BSD-3-Clause (Opus, libvpx with its patent grant, zstd), Apache-2.0 (Vulkan), MIT (VMA) |

Installs made before these tools were portable may still use what an older wizard installed on the system (MSYS2,
Git, Python, the .NET SDK, the Visual Studio Build Tools); the wizard keeps using MSYS2 for an install whose objects it
compiled, and never installs any of them any more.