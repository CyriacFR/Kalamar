# Security policy

## Reporting a vulnerability

Please report security problems **privately**, never in a public issue, pull request or discussion.

Use GitHub's private vulnerability reporting: the repository's **Security** tab, then **Report a vulnerability**
([how it works](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)).
Only the maintainers see the report, and the discussion stays private until an advisory is published.

A useful report says:

- what is affected (launcher or engine, the version or commit, Windows version);
- what an attacker can do, and from where (a community server, a file the player opens, the local network...);
- the steps to reproduce, or a proof of concept that does not need game files.

Never attach game files, keys, saves or logs that hold personal data (paths with your user name, IP addresses, other
players' names): describe them instead, or redact them.

This is the only way to report a vulnerability: there is no e-mail address and no other private contact, and a
vulnerability is never reported in a public pull request either.

## What to expect

Kalamar is maintained by volunteers in their free time: no response time is guaranteed. A confirmed vulnerability
stays private until a fixed version is available; it is then published as a GitHub security advisory, crediting the
reporter if they wish.

Only the latest release, and the `main` branch, receive security fixes.

## Scope

In scope, for example:

- the launcher's handling of what a **community server** sends (discovery documents, server cards, protocol data,
  activity views, links), its OAuth login and the session it stores, and anything a server could use to make the
  launcher run code, open a file, reach another host or reveal private data;
- the **launcher's self-update** (manifest and signature checks, downloads, the replacement of the program:
  [launcher/docs/releasing.md](launcher/docs/releasing.md)) and the **install wizard's** downloads (pinned hashes,
  archive unpacking, paths);
- the **engine's network code**: LAN play's allowlist, the relay client, anything that would let the game reach a host
  it was not given, and the parsing of files the game reads (settings, profiles, mod and cheat files);
- the **release and CI configuration** of this repository.

Out of scope:

- defects of the game itself (it is the publisher's program, run from the player's own copy);
- community servers run by other people: report to their operator; defects of the **reference server** go to the
  server repository (`kalamar-server`), which has its own policy;
- cheating and abuse in matches: they are moderation questions for each server's operator;
- attacks that need administrator access to the player's PC, or a modified launcher or game on the attacker's own
  machine (results are reported by the players' machines by design: [docs/servers.md](docs/servers.md), "Privacy").

## What the software already does

What the launcher protects players from, what it cannot, and the risks that remain (the game's own network code, the
trust a server's operator receives, self-reported results, the plaintext relay protocol): the security model,
[launcher/docs/security-model.md](launcher/docs/security-model.md). A weakness it lists as accepted or remaining is
known; a way around a protection it claims is a vulnerability to report here. The server side has its own model, in
the server repository (`docs/security-model.md`).

The launcher's network accesses, all of them, are listed in
[launcher/docs/network-accesses.md](launcher/docs/network-accesses.md); what it sends to a server and what it never sends, in [docs/servers.md](docs/servers.md). Updates are
installed only when signed by a key compiled into the launcher ([launcher/docs/releasing.md](launcher/docs/releasing.md)).
The game opens no network socket unless LAN play is turned on ([engine/docs/lan-play.md](engine/docs/lan-play.md)).
