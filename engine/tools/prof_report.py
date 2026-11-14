"""Report for a TOTK_PROFILE sampling profile (runtime/prof_win.c): top functions and the guest/runtime/outside split.

usage: prof_report.py <profile> <totk.map> [--top 40] [--chains 30]

The map is the lld one from the link (build/<version>/totk.map); its addresses are RVAs, the profile header carries the
runtime image base, so ASLR does not matter. Symbols in the map are labels without sizes: a Rip belongs to the nearest
symbol at or below it.

Samples outside the exe are named by the nearest export of their module, read from the module's file. The profile's
per-thread counts (trip lines) give each busy thread's own top functions, under the thread's description.

A profile taken with TOTK_PROFILE_RENDER=1 or TOTK_PROFILE_CHAINS=1 also has call chains: the report then adds the inclusive time per
function (a function counted once per chain), and for each module outside the exe the exe functions that call into it
and the longest common chains down to it.
"""
import argparse
import bisect
import collections
import re
import struct
import sys
from pathlib import Path

SYM = re.compile(r'^([0-9a-f]{8}) +[0-9a-f]{8} +\d+ {2,}(\S+)$')   # In column empty: object lines have a path:(section) there


def load_map(path):
    syms = {}
    with open(path, encoding='utf-8', errors='replace') as f:
        for line in f:
            m = SYM.match(line.rstrip('\n'))
            if m and '(' not in m.group(2):
                syms.setdefault(int(m.group(1), 16), m.group(2))
    addrs = sorted(syms)
    return addrs, [syms[a] for a in addrs]


def load_profile(path):
    prof = {'rips': [], 'modules': [], 'threads': [], 'stacks': [], 'samples': 0, 'base': 0, 'trips': [], 'tnames': {}}
    for line in Path(path).read_text(encoding='utf-8', errors='replace').splitlines():
        p = line.split(None, 3)
        if not p:
            continue
        if p[0] == 'trip':
            prof['trips'].append((int(p[1]), int(p[2], 16), int(p[3])))
        elif p[0] == 'tname':
            prof['tnames'][int(p[1])] = line.split(None, 2)[2] if len(p) > 2 else '-'
        elif p[0] == 'base':
            prof['base'] = int(p[1], 16)
        elif p[0] == 'samples':
            prof['samples'] = int(p[1])
        elif p[0] == 'thread':
            prof['threads'].append((int(p[1]), int(p[2]), int(p[3])))
        elif p[0] == 'module':
            prof['modules'].append((int(p[1], 16), int(p[2], 16), p[3]))
        elif p[0] == 'rip':
            prof['rips'].append((int(p[1], 16), int(p[2])))
        elif p[0] == 'stack':
            f = line.split()
            prof['stacks'].append((int(f[1]), [int(x, 16) for x in f[2:]]))
    return prof


def report_chains(prof, name_of, in_exe, top):
    stacks = prof['stacks']
    total = sum(n for n, _ in stacks) or 1
    print(f'\ncall chains: {total} samples in {len(stacks)} chains (render thread, or every thread with TOTK_PROFILE_CHAINS)')
    incl = collections.Counter()
    for n, chain in stacks:
        for f in set(name_of(r) for r in chain):
            incl[f] += n
    print(f'\ninclusive, top {top} (% of chain samples)')
    for f, n in incl.most_common(top):
        print(f'  {100 * n / total:6.2f}%  {n:8d}  {f}')
    # Per outside module: the innermost exe frame of each chain whose leaf is in that module
    by_mod = collections.defaultdict(collections.Counter)
    paths = collections.defaultdict(collections.Counter)
    for n, chain in stacks:
        if in_exe(chain[0]):
            continue
        mod = name_of(chain[0]).split('!')[0]
        i = next((k for k, r in enumerate(chain) if in_exe(r)), None)
        if i is None:
            by_mod[mod]['(no exe frame in the chain)'] += n
            continue
        by_mod[mod][name_of(chain[i])] += n
        paths[mod][' < '.join(name_of(r) for r in chain[i:i + 4])] += n
    for mod, c in sorted(by_mod.items(), key=lambda kv: -sum(kv[1].values())):
        m = sum(c.values())
        print(f'\n{mod}: {100 * m / total:.2f}% of chain samples; exe callers')
        for f, n in c.most_common(12):
            print(f'  {100 * n / total:6.2f}%  {n:8d}  {f}')
        for pth, n in paths[mod].most_common(6):
            print(f'      {100 * n / total:6.2f}%  {pth}')


def load_exports(path):
    """Exported function RVAs and names of a PE file -> (sorted rvas, names); empty when it cannot be read. Names the
    system calls and waits that samples outside the exe stop in (ntdll!NtWaitForSingleObject, NtGetWriteWatch, ...)."""
    try:
        d = Path(path).read_bytes()
        pe = struct.unpack_from('<I', d, 0x3c)[0]
        nsec, opt_size = struct.unpack_from('<H', d, pe + 6)[0], struct.unpack_from('<H', d, pe + 20)[0]
        opt = pe + 24
        exp_rva = struct.unpack_from('<I', d, opt + (112 if struct.unpack_from('<H', d, opt)[0] == 0x20b else 96))[0]
        secs = [struct.unpack_from('<IIII', d, opt + opt_size + 40 * i + 8) for i in range(nsec)]

        def off(rva):
            for vsize, va, rsize, raw in secs:
                if va <= rva < va + max(vsize, rsize):
                    return rva - va + raw
            raise ValueError(rva)
        e = off(exp_rva)
        nfun, nnames, afun, aname, aord = struct.unpack_from('<IIIII', d, e + 20)
        funcs = struct.unpack_from(f'<{nfun}I', d, off(afun))
        out = {}
        for i in range(nnames):
            nrva = struct.unpack_from('<I', d, off(aname) + 4 * i)[0]
            o = off(nrva)
            name = d[o:d.index(b'\0', o)].decode('ascii', 'replace')
            out.setdefault(funcs[struct.unpack_from('<H', d, off(aord) + 2 * i)[0]], name)
        rvas = sorted(out)
        return rvas, [out[r] for r in rvas]
    except (OSError, ValueError, struct.error, IndexError):
        return [], []


def report_threads(prof, name_of, blocked_mod, top, per):
    """Per thread (trip lines): samples, the share not stopped in ntdll/win32u/KERNELBASE, and its top functions."""
    by = collections.defaultdict(collections.Counter)
    for tid, rip, n in prof['trips']:
        by[tid][name_of(rip)] += n
    rows = []
    for tid, c in by.items():
        tot = sum(c.values())
        run = sum(n for f, n in c.items() if not blocked_mod(f))
        rows.append((run, tot, tid, c))
    rows.sort(reverse=True)
    print(f'\nthreads by samples not stopped in ntdll/win32u/KERNELBASE (top {top}; 1 sample = 1 ms of the thread)')
    for run, tot, tid, c in rows[:top]:
        print(f'\n  tid {tid} "{prof["tnames"].get(tid, "?")}": {tot} samples, {100 * run / max(tot, 1):.1f}% outside the '
              f'blocking modules')
        for f, n in c.most_common(per):
            print(f'    {100 * n / max(tot, 1):6.2f}%  {n:7d}  {f}')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('profile')
    ap.add_argument('map')
    ap.add_argument('--top', type=int, default=40)
    ap.add_argument('--chains', type=int, default=30, help='inclusive functions listed from the call chains')
    ap.add_argument('--threads', type=int, default=16, help='threads listed with their own top functions')
    ap.add_argument('--per-thread', type=int, default=10, help='functions listed per thread')
    a = ap.parse_args()
    prof = load_profile(a.profile)
    addrs, names = load_map(a.map)
    base = prof['base']
    exe = next((m for m in prof['modules'] if m[0] == base), None)
    exe_end = base + exe[1] if exe else base
    exports = {}

    def in_exe(rip):
        return base <= rip < exe_end

    def name_of(rip):
        if in_exe(rip):
            i = bisect.bisect_right(addrs, rip - base) - 1
            return names[i] if i >= 0 else '?'
        m = next((m for m in prof['modules'] if m[0] <= rip < m[0] + m[1]), None)
        if not m:
            return 'unknown'
        if m[2] not in exports:
            exports[m[2]] = load_exports(m[2])
        rvas, enames = exports[m[2]]
        i = bisect.bisect_right(rvas, rip - m[0]) - 1
        if i >= 0 and rip - m[0] - rvas[i] < 0x10000:
            return f'{Path(m[2]).name}!{enames[i]}+{rip - m[0] - rvas[i]:#x}'
        return f'{Path(m[2]).name}!+{rip - m[0]:#x}'
    total = sum(n for _, n in prof['rips']) or 1
    funcs = collections.Counter()
    cats = collections.Counter()
    for rip, n in prof['rips']:
        if base <= rip < exe_end:
            i = bisect.bisect_right(addrs, rip - base) - 1
            name = names[i] if i >= 0 else '?'
            funcs[name] += n
            cats['guest (f_*)' if name.startswith('f_') else 'runtime host (exe)'] += n
            continue
        mod = next((Path(m[2]).name for m in prof['modules'] if m[0] <= rip < m[0] + m[1]), None)
        cats['outside: ' + (mod or 'unknown')] += n
        funcs[name_of(rip)] += n
    blocked = ('outside: ntdll', 'outside: win32u', 'outside: KERNELBASE')   # waits and message loops: threads not running
    idle = sum(n for k, n in cats.items() if k.startswith(blocked))
    act = (total - idle) or 1
    print(f'samples {prof["samples"]} (Rips kept {total}), 1 ms nominal period; ntdll/win32u/KERNELBASE (blocked threads) {100 * idle / total:.1f}%')
    print('\ncategory (% of all samples, % of running samples = all minus ntdll/win32u/KERNELBASE)')
    for k, n in cats.most_common():
        print(f'  {100 * n / total:6.2f}%  {"" if k.startswith(blocked) else format(100 * n / act, "6.2f") + "%":>7}  {n:8d}  {k}')
    print(f'\ntop {a.top} functions')
    for k, n in funcs.most_common(a.top):
        print(f'  {100 * n / total:6.2f}%  {n:8d}  {k}')
    print('\nthreads (samples, % of them inside the exe)')
    for tid, n, ie in sorted(prof['threads'], key=lambda t: -t[1])[:12]:
        print(f'  tid {tid:6d} {n:8d}  {100 * ie / max(n, 1):5.1f}% in exe')
    if prof['trips']:
        report_threads(prof, name_of, lambda f: f.lower().startswith(('ntdll', 'win32u', 'kernelbase')), a.threads,
                       a.per_thread)
    if prof['stacks']:
        report_chains(prof, name_of, in_exe, a.chains)


if __name__ == '__main__':
    sys.exit(main())
