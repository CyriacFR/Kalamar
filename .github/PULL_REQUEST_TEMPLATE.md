<!--
Thanks! Rules: CONTRIBUTING.md. One topic per pull request; describe it in your own words. Delete what does not apply.
Never attach game files, keys, saves, or logs and screenshots that contain them.
-->

## What and why

<!-- The problem, the change, and the issue it closes (Closes #123). -->

## How it was tested

<!-- The commands you ran and their results; for engine changes, the rows of engine/CONTRIBUTING.md
"Checks by what you changed". Say what you could not run (game files, hardware you do not have). Written with an AI
assistant? The same applies: CONTRIBUTING.md, "AI-assisted contributions". -->

## Checklist

- [ ] Tests added or updated, and they pass (`npm run test:ui`, `cargo test`, the engine's checks for what changed)
- [ ] Every check reported above was actually run on this change, by me; the ones I could not run are named as not run
- [ ] Nothing derived from the game or a console is added, not even for reference: no files, extracts, decompiled code,
      images, data tables or keys
- [ ] No new network access, or it is listed in `launcher/docs/network-accesses.md` and `docs/servers.md`
- [ ] Documentation updated where behavior changed
- [ ] New interface text is in every language of `launcher/src/locales/` (and the engine's menu tables, if any)
- [ ] Commits are signed off (`git commit -s`)
