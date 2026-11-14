# Game data and the player profile

How the launcher reads names and icons from the player's own game files, and how the Locker's profile reaches the game.

## Names and icons come from your own files, at run time

The Locker shows the game's own names and icons. They are read from **your** RomFS image
(`<game folder>\romfs\romfs.bin` + `index.json`) when the page opens and are never shipped, bundled or committed:

- `game/romfs.rs` random access through `index.json` (no extraction) + zstd (`ruzstd`).
- `game/byml.rs`, `sarc.rs`, `msbt.rs` parsers for the parameter tables (`RSDB/*.rstbl.byml.zs`), the message
  archives (`Mals/EUfr|EUen|USes|JPja`, one per interface language) and their texts. `game/bntx.rs` BNTX textures: Tegra block-linear deswizzle
  (`tegra_swizzle`), ASTC/BCn decode (`texture2ddecoder`), PNG (`png`).
- `game/catalogue.rs` builds the typed catalog; `game/service.rs` caches it as JSON, keyed by image size and
  index hash, in `<data dir>\game-cache\<key>\`; `game/icons.rs` decodes icons on demand (custom protocol
  `gameicon`, 4 worker threads) into `<data dir>\game-cache\<key>\icons\`. The page can also extract every icon in
  the background. If the RomFS is missing, the page says what is needed and the Character tab falls back to a neutral
  built-in list (ids only).
- Selectability rules (what counts as a real player item) are documented at the top of `game/catalogue.rs`.
- **Stages.** The catalog also lists the versus stages (`RSDB/VersusSceneInfo`: the scene name the engine, the
  protocol and the result files use), named from `CommonMsg/VS/VSStageName.msbt` in the four interface languages and
  pictured by the 640x360 screenshots of the game's stage menus (`UI/Icon/StageL/Vss_<label>.bntx.zs`, ASTC 4x4). The
  pictures are served scaled (`gameicon` key `<key>@480` for a card, `<key>@160` for a miniature: `icons::SIZES`, area
  average in linear light); building the cache decodes them all into `icons\` (about 7 MB). The page asks for them
  with `gamedata_stages` (`src/stages.tsx`: `StageCard`, `StagePicker` to choose or ban, `StageField`, `StageTag`,
  `StageStrip`), only on a screen that shows a stage. Without game data the cards are neutral, named by the engine's
  list, the server, or the id made readable, and the room form keeps its drop-down list.
- `cargo test --test game_data -- --ignored --nocapture` with `KALAMAR_GAME_ROOT` set runs the real-data check
  (`KALAMAR_STAGE_DUMP=<folder outside the repository>` keeps the stage pictures to look at them).

## profile.json and how it reaches the game

`profile.json` (launcher data dir), schema 1; every number is a game id, `null` = not chosen:

```json
{ "schema": 1, "name": "Kay",
  "style": 0, "skin": 2, "eyes": 5, "hair": 12, "eyebrows": 1, "legwear": 3, "legwear_variation": 0,
  "head": 1000, "clothes": 8031, "shoes": 4024, "weapon": 10,
  "banner": 11001, "title": { "adjective": 142, "subject": 0 }, "badges": [null, null, null] }
```

At launch (`profile::prepare_launch`) the launcher converts it to the runtime's `save_profile` schema
([save-profile.md](../../engine/docs/save-profile.md)), writes it atomically to `<root>\userdata\launcher-save-profile.json` and passes
`KLM_SAVE_PROFILE=<that file>`, `KLM_PLAYER_NAME` and `KLM_NICKNAME` (the name). The game itself applies it to its
encrypted save while starting (previous save copied to `profile\save-profile-backup\<time>\`). Afterwards the Play screen
reads the `save profile:` lines of the run log and says whether it was applied, already up to date, refused (with the
runtime's reason) or not passed.

- Styles: launcher style id = save `ModelType` = index in `inkling_f, inkling_m, octoling_f, octoling_m`
  (SquidF 0, SquidM 1, OctopusF 2, OctopusM 3). The creation screen's four kind buttons sit left to right, the first two
  under the Inkling heading and the last two under the Octoling one (pane positions in `Tuto_MakePlayer_00`).
- `null` fields are left out of the file: unchanged in the save. **A save exists** (a `save\<uid>\save.dat` in the
  user data folder): the profile may be partial, every field including the appearance is optional, and applying it
  twice changes nothing (the runtime logs `0 changes`, writes nothing and makes no backup). **No save yet**: the game
  creates one and the profile must hold the whole appearance (style, hair, eyebrows, legwear, skin, eyes); the launcher
  says exactly what is missing before launching and refuses to launch (the Play screen offers "launch without the
  profile": the game then makes its own character).
- A partial profile never sends `skip_first_launch: true` (the runtime would count it as ending the first launch of a
  save that is still in it); a complete one does.
- Options (Locker): apply the profile, skip the intro and tutorial (default on), unlock everything (default on).
- The runtime's refusals (`hair 33 belongs to Octolings, not to the style inkling_f ...`, the six dropped gear ids...)
  are read from the log, mapped to the offending field (`profile::classify_refusal`) and shown translated next to that
  field in the Locker and in the Play report, with the runtime's own sentence unchanged below.
- Backups: the Locker and the Play report show the folder (`<user data>\profile\save-profile-backup`) and the number
  of copies; how many the runtime keeps is the `save_profile_backups` setting (Settings, "Save" tab; default 10, 0 = all).
- Unavailable, shown with the reason: the six gear ids the game drops (head 3007, clothes 1030 and 6003, shoes 2011, 3009,
  3012) and every badge (how the save records an acquired badge is unknown).
- Hair and eyebrows must match the style's species; the legwear variation must exist.
