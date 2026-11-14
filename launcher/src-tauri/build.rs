fn main() {
    update_channel();
    discord_application();
    dll_search();
    tauri_build::build()
}

/// The DLLs the launcher's executable imports are looked up in System32 only (`/DEPENDENTLOADFLAG`, value
/// `LOAD_LIBRARY_SEARCH_SYSTEM32`), never in the executable's own folder: a DLL dropped next to a portable launcher (the
/// Downloads folder) is never loaded. Those it loads later follow `SetDefaultDllDirectories` (`winutil::harden`).
fn dll_search() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}

/// The Discord application whose name and pictures the opt-in presence shows (`src/discord/`), fixed at build time:
/// `KALAMAR_DISCORD_CLIENT_ID`, the public "application id" of the maintainers' application on Discord's developer portal
/// (`docs/discord-presence.md`). Empty by default: that build has no Discord presence at all (nothing in the
/// interface, no pipe opened). A value that is not an application id fails the build.
fn discord_application() {
    println!("cargo:rerun-if-env-changed=KALAMAR_DISCORD_CLIENT_ID");
    let id = std::env::var("KALAMAR_DISCORD_CLIENT_ID").unwrap_or_default().trim().to_string();
    if !id.is_empty() {
        let valid = (17..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit());
        assert!(valid, "KALAMAR_DISCORD_CLIENT_ID='{id}' is not a Discord application id (17 to 20 digits)");
    }
    println!("cargo:rustc-env=KALAMAR_DISCORD_CLIENT_ID={id}");
}

/// The launcher's self-update source, fixed at build time (`src/update/channel.rs`): `KALAMAR_UPDATE_REPO`
/// (`owner/repo`, empty by default: no update feature at all) and the public keys of `update-keys.txt`. A repository
/// without a valid key, or a malformed one, fails the build rather than producing a launcher that cannot verify.
fn update_channel() {
    println!("cargo:rerun-if-env-changed=KALAMAR_UPDATE_REPO");
    println!("cargo:rerun-if-changed=update-keys.txt");
    let repo = std::env::var("KALAMAR_UPDATE_REPO").unwrap_or_default().trim().to_string();
    let text = std::fs::read_to_string("update-keys.txt").unwrap_or_default();
    let keys: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    for k in &keys {
        // the base64 line of a minisign public key: "Ed" + 8-byte key id + 32-byte key = 42 bytes = 56 characters
        let shape = k.len() == 56 && k.starts_with("RW") && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/');
        assert!(shape, "update-keys.txt: '{k}' is not a minisign public key line");
    }
    if !repo.is_empty() {
        let ok_part = |p: &str, max: usize| !p.is_empty() && p.len() <= max && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) && p != "." && p != "..";
        let valid = repo.split_once('/').is_some_and(|(o, r)| ok_part(o, 39) && ok_part(r, 100) && !o.contains('.') && !o.contains('_'));
        assert!(valid, "KALAMAR_UPDATE_REPO='{repo}' is not owner/repo");
        assert!(!keys.is_empty(), "KALAMAR_UPDATE_REPO is set but src-tauri/update-keys.txt holds no public key");
    }
    println!("cargo:rustc-env=KALAMAR_UPDATE_REPO={repo}");
    println!("cargo:rustc-env=KALAMAR_UPDATE_KEYS={}", if repo.is_empty() { String::new() } else { keys.join(";") });
}
