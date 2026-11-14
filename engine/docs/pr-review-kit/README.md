# PR review kit

> **From the upstream project, adapted.** The kit comes from totk-pc-recomp. Here, review follows this repository's public GitHub
> flow: pull requests against `main`, CI green (the workflows of the repository root), every commit signed off
> (Developer Certificate of Origin), merged by a maintainer ([CONTRIBUTING.md](../../CONTRIBUTING.md)). This page,
> [01-rules.md](01-rules.md) and [05-checklist.md](05-checklist.md) were adapted to that flow.
> [02-performance.md](02-performance.md), [03-correctness.md](03-correctness.md) and [04-knowledge.md](04-knowledge.md)
> keep the upstream's *Tears of the Kingdom* measurements and pull request numbers as background; each says at its top
> what was checked for Kalamar.

Rules and background for a reviewer of engine pull requests. One goal: no pull request breaks the game's behavior or
costs frames on weak hardware.

## The short version

1. A PR description and "works for me" prove nothing. Read the diff yourself and check it against `refs/`, the
   Vulkan spec and upstream issues.
2. Almost every change needs a test: a repro before and after, a probe, a differential test, or whole runs with the
   switch off and on.
3. The developer's machine is fast; players run weak machines. "Neutral here" says nothing about them, in either
   direction.
4. A player without the problem sees no change: the new path sits behind a switch and the old path stays as it was.
5. A pull request goes into `main` only with CI green, every commit signed off, the checks its author reports really
   run, and a maintainer's approval. Nothing derived from the game goes in, whatever it fixes.

## Files

| File | Covers |
|---|---|
| [01-rules.md](01-rules.md) | triage order, what a PR needs, reasons to reject, merging and commits |
| [02-performance.md](02-performance.md) | target hardware, gain ceiling, how to measure, what the hot path must not do |
| [03-correctness.md](03-correctness.md) | lifter, tests and probes, GPU portability, Maxwell behavior, timers, threads, game content |
| [04-knowledge.md](04-knowledge.md) | invariants of recent changes, fixed bug classes, open risks, mistakes |
| [05-checklist.md](05-checklist.md) | the per-PR checklist |

The public docs this kit builds on: [docs/architecture.md](../architecture.md),
[docs/building.md](../building.md) ("Checks"), [docs/maxwell.md](../maxwell.md),
[docs/settings.md](../settings.md), [docs/tester-guide.md](../tester-guide.md), [AGENTS.md](../../AGENTS.md).
Reviewing with a coding agent: give it this folder, the engine's [AGENTS.md](../../AGENTS.md) and the repository's
top-level `AGENTS.md` before the first review.

The code and `docs/` win over this text where they differ.
