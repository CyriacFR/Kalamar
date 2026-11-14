//! Progress of the steps: what each tool prints (`[n/733]` from Ninja, `emitted n/m chunks` from the lifter), and the
//! time estimates. Reference durations come from one measured install (prerequisites present,
//! 16 compile jobs); during an install they are scaled by how fast this PC went through the steps already finished.

use super::StepId;

/// Seconds each step took in the reference run (prerequisites already installed).
pub fn reference_secs(step: StepId, jobs: u32) -> f64 {
    match step {
        StepId::Checks => 1.0,
        StepId::Prereqs => 4.0,
        StepId::Sources => 4.0,
        StepId::Python => 26.0,
        StepId::Exefs => 2.0,
        StepId::Hactool => 26.0,
        StepId::Romfs => 111.0,
        StepId::Lift => 178.0,
        // 891 s with 16 jobs on 16 logical CPUs; fewer jobs is slower, though not linearly.
        StepId::Compile => 891.0 * (16.0 / jobs.clamp(1, 16) as f64).powf(0.8),
        StepId::Xlate => 51.0,
        StepId::Keys => 7.0,
        StepId::Layout => 1.0,
        StepId::Selftest => 5.0,
        StepId::Cleanup => 5.0,
    }
}

/// The same with the tools kept in the install folder (`portable`): Python and the toolchain are downloaded, hactool and
/// the translator are copied, the RomFS merge reads the NCAs in place, and llvm-mingw's clang compiles faster than
/// MSYS2's. Measured on one install (an 8-core, 16-thread CPU, 32 GB, the install and the
/// player's files on a SATA SSD, about 40 MB/s download).
pub fn reference_secs_portable(step: StepId, jobs: u32) -> f64 {
    match step {
        // 202 MB downloaded at about 40 MB/s and unpacked (28 s measured)
        StepId::Prereqs => 30.0,
        StepId::Python => 6.0,
        StepId::Hactool => 1.0,
        // hactool reading the NSPs in place on a SATA SSD, beside the lift and the start of the compile (197-201 s
        // measured; 156 s beside the lift alone)
        StepId::Romfs => 200.0,
        // 16 workers (133-135 s measured beside the merge; 202 s with the generator's own 7)
        StepId::Lift => 135.0,
        // 489 s measured with 16 jobs on 16 logical CPUs, the first 64 s beside the merge (532 s before the largest
        // functions got their smaller splitting budget; 1508 s with MSYS2's clang and 7 jobs on the same PC); fewer jobs
        // is slower, though not linearly
        StepId::Compile => 490.0 * (16.0 / jobs.clamp(1, 16) as f64).powf(0.8),
        StepId::Xlate => 1.0,
        _ => reference_secs(step, jobs),
    }
}

/// Seconds left when some steps run beside the foreground chain. `fg` holds the foreground steps still to run, in order,
/// with their remaining seconds (the current one first); `bg` the steps running beside them with their remaining
/// seconds and the index in `fg` of the step that waits for them (`fg.len()`: the end).
pub fn lanes_eta(fg: &[f64], bg: &[(f64, usize)]) -> f64 {
    let mut t = 0.0f64;
    for i in 0..=fg.len() {
        for (rem, join) in bg {
            if *join == i {
                t = t.max(*rem);
            }
        }
        if let Some(s) = fg.get(i) {
            t += s;
        }
    }
    t
}

/// Extra time when the prerequisites have to be installed (downloads and installers; the Build Tools are the bulk).
pub fn prereq_install_secs(winget_ids: &[&str], packages: usize) -> f64 {
    let mut s = 0.0;
    for id in winget_ids {
        s += if *id == super::prereq::WINGET_VS { 900.0 } else { 90.0 };
    }
    if packages > 0 {
        s += 240.0;
    }
    s
}

/// `[n/N]` at the start of a Ninja line.
pub fn ninja_fraction(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix('[')?;
    let (a, b) = rest.split_once(']')?.0.split_once('/')?;
    let (n, t) = (a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?);
    (t > 0 && n <= t).then_some((n, t))
}

/// `emitted n/m chunks, ...s` from `recomp/gen.py`.
pub fn lift_fraction(line: &str) -> Option<(u64, u64)> {
    let rest = line.trim().strip_prefix("emitted ")?;
    let frac = rest.split_whitespace().next()?;
    let (a, b) = frac.split_once('/')?;
    let (n, t) = (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?);
    (t > 0 && n <= t).then_some((n, t))
}

/// The compile step has two Ninja runs: the compile (`[n/733]`) then the link (`[1/1] LINK`). The link is quick; the
/// compile is mapped to 0..0.97.
pub fn compile_progress(line: &str, current: Option<f64>) -> Option<f64> {
    let (n, t) = ninja_fraction(line)?;
    if line.contains("LINK") {
        return Some(if n == t { 0.99 } else { 0.97 });
    }
    let f = 0.97 * n as f64 / t as f64;
    Some(current.map_or(f, |c| c.max(f)))
}

/// Remaining seconds of a step from its measured fraction, else from its reference duration.
pub fn step_eta(elapsed: f64, fraction: Option<f64>, reference: f64, speed: f64) -> f64 {
    match fraction {
        Some(f) if f >= 0.03 && elapsed >= 5.0 => (elapsed * (1.0 - f) / f).max(0.0),
        Some(f) => (reference * speed * (1.0 - f)).max(0.0),
        None => (reference * speed - elapsed).max(reference * speed * 0.05),
    }
}

/// How fast this PC goes compared with the reference: actual / reference over the steps finished in this run (only
/// steps long enough to say something), clamped.
pub fn speed_factor(done: &[(f64, f64)]) -> f64 {
    let (a, r) = done.iter().filter(|(_, r)| *r >= 10.0).fold((0.0, 0.0), |(a, r), (x, y)| (a + x, r + y));
    if r < 10.0 { 1.0 } else { (a / r).clamp(0.4, 3.0) }
}

/// Overall fraction: finished weight plus the current step's share, over the weight of the steps this run executes.
pub fn overall(weights_done: f64, current_weight: f64, current_fraction: f64, total: f64) -> f64 {
    if total <= 0.0 {
        return 1.0;
    }
    ((weights_done + current_weight * current_fraction.clamp(0.0, 1.0)) / total).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_output() {
        assert_eq!(ninja_fraction("[32/733] CC C:/x/lift/main/main_0030.c"), Some((32, 733)));
        assert_eq!(ninja_fraction("[1/1] LINK C:/x/build/splat3.exe"), Some((1, 1)));
        assert_eq!(ninja_fraction("905 warnings generated."), None);
        assert_eq!(ninja_fraction("[x/3]"), None);
        assert_eq!(lift_fraction("emitted 200/595 chunks, 73s"), Some((200, 595)));
        assert_eq!(lift_fraction("emitting with 8 workers"), None);
        let a = compile_progress("[366/733] CC x.c", None).unwrap();
        assert!((a - 0.4846).abs() < 0.01);
        // an out-of-order lower count never moves the bar back
        assert_eq!(compile_progress("[10/733] CC y.c", Some(a)), Some(a));
        assert_eq!(compile_progress("[1/1] LINK splat3.exe", Some(a)), Some(0.99));
    }

    #[test]
    fn estimates() {
        assert!((reference_secs(StepId::Compile, 16) - 891.0).abs() < 1e-6);
        assert!(reference_secs(StepId::Compile, 8) > 1400.0);
        // measured: half done after 100 s -> 100 s left
        assert!((step_eta(100.0, Some(0.5), 500.0, 1.0) - 100.0).abs() < 1e-6);
        // nothing measurable: reference minus elapsed, never zero while running
        assert!((step_eta(10.0, None, 50.0, 1.0) - 40.0).abs() < 1e-6);
        assert!(step_eta(80.0, None, 50.0, 1.0) > 0.0);
        assert_eq!(speed_factor(&[]), 1.0);
        assert!((speed_factor(&[(200.0, 100.0), (1.0, 2.0)]) - 2.0).abs() < 1e-6);
        assert_eq!(speed_factor(&[(1000.0, 100.0)]), 3.0);
        assert!((overall(50.0, 50.0, 0.5, 100.0) - 0.75).abs() < 1e-6);
        assert_eq!(overall(0.0, 0.0, 0.0, 0.0), 1.0);
        assert!(prereq_install_secs(&["Git.Git", super::super::prereq::WINGET_VS], 3) > 1000.0);
    }

    #[test]
    fn parallel_steps_count_once() {
        // nothing beside: the plain sum
        assert_eq!(lanes_eta(&[10.0, 20.0, 30.0], &[]), 60.0);
        // a merge of 100 s waited for before the third step, beside 10 + 200: hidden
        assert_eq!(lanes_eta(&[10.0, 200.0, 30.0], &[(100.0, 2)]), 240.0);
        // longer than what runs beside it: the third step waits for it
        assert_eq!(lanes_eta(&[10.0, 20.0, 30.0], &[(100.0, 2)]), 130.0);
        // a download waited for at the end
        assert_eq!(lanes_eta(&[5.0], &[(50.0, 1)]), 50.0);
        assert_eq!(lanes_eta(&[], &[]), 0.0);
        assert!(reference_secs_portable(StepId::Compile, 16) < reference_secs(StepId::Compile, 7));
        assert_eq!(reference_secs_portable(StepId::Keys, 8), reference_secs(StepId::Keys, 8));
    }
}
