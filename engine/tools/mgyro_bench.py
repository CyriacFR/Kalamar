"""Tables from a TOTK_MGYRO_BENCH run (runtime/mgyro_bench.c): the mouse gyro's gain, linearity, repeatability, step
response, latency, polling-rate independence and limits, measured on the game's aim, as docs/controls.md reports them.

    python tools/mgyro_bench.py <bench.csv> [--aim N]

No game files. Angles are degrees; a positive count is the mouse moved right (x) or down (y), and a positive turn is
the one those counts ask for (right, down). "Gain" is degrees of aim per count; the setting asks for mouse_gyro_sens
(0.04 by default) times the game's own motion sensitivity (1 at the game's default, measured).
"""
import argparse
import math
import re
import struct
import sys
import zlib
from pathlib import Path

import numpy as np


def load(path):
    H, E, R, S, T, P, F = {}, [], [], [], [], [], []
    with open(path, encoding='utf-8') as f:
        for line in f:
            p = line.rstrip('\n').split(',')
            k = p[0]
            if k == 'R':
                R.append(p[1:])
            elif k == 'S':
                S.append(p[1:])
            elif k == 'T':
                T.append(p[1:])
            elif k == 'P':
                P.append(p[1:])
            elif k == 'F':
                F.append(p[1:])
            elif k == 'E':
                E.append((int(p[1]), int(p[2]), ','.join(p[3:])))
            elif k == 'H':
                H.setdefault(p[1], []).append(','.join(p[2:]))
    return H, E, R, S, T, P, F


class Run:
    def __init__(self, path, aim):
        H, E, R, S, T, P, F = load(path)
        self.path = Path(path)
        self.H, self.E = H, E
        self.k = float(H['deg_per_count'][0])
        na = max(len(r) for r in R) - 9
        self.naims = na // 3
        n = len(R)
        self.read = np.array([int(r[0]) for r in R])
        self.t = np.array([int(r[1]) for r in R], dtype=np.int64)
        self.smp = np.array([int(r[2]) for r in R])
        self.new = np.array([int(r[3]) for r in R])
        self.pres = np.array([int(r[4]) for r in R])
        self.cyaw = np.array([float(r[5]) for r in R])
        self.cpitch = np.array([float(r[6]) for r in R])
        self.injx = np.array([int(r[7]) for r in R])
        self.injy = np.array([int(r[8]) for r in R])
        v = np.full((n, max(self.naims, 1), 3), np.nan)
        for i, r in enumerate(R):
            for a in range((len(r) - 9) // 3):
                v[i, a] = [float(x) for x in r[9 + 3 * a:12 + 3 * a]]
        self.vec = v
        self.row = {int(r): i for i, r in enumerate(self.read)}
        # the controller: + is the turn the counts ask for (right, down)
        self.cx = -self.cyaw
        self.cy = -self.cpitch
        # The aim: the view's forward vector (y up). Vertical: its elevation. Horizontal: its azimuth (yaw about the
        # world's up); self.arc is the turn along the screen's horizontal instead, the path integral of
        # cos(elevation) d(azimuth). Signs set so a positive count is a positive turn.
        kinds = {int(k[3:]): v[0] for k, v in H.items() if re.fullmatch(r'aim\d+', k)}
        if aim < 0:   # the camera's forward vector: nearly level, turned most by the yaw, then moved most by the pitch
            cand = {}
            for i, text in kinds.items():
                m = re.search(r'c1=(\S+) .*cp=(\S+) kind=view v0=(\S+) (\S+) (\S+)', text)
                if m and abs(float(m.group(4))) < 0.1 and 10 < float(m.group(2)) < 40:
                    cand[i] = (float(m.group(1)), float(m.group(2)))
            top = max((c[0] for c in cand.values()), default=0)
            near = [i for i, c in cand.items() if c[0] > 0.98 * top]
            aim = max(near, key=lambda i: cand[i][1]) if near else \
                next((i for i in sorted(kinds) if 'kind=view ' in kinds[i]), 0)
        self.aim = aim
        self.aim_info = kinds.get(aim, '')
        self.ax = self.ay = np.full(n, np.nan)
        if aim < self.naims:
            vv = v[:, aim]
            ok = ~np.isnan(vv[:, 0])
            elev = np.degrees(np.arcsin(np.clip(vv[:, 1], -1, 1)))
            az = np.full(n, np.nan)
            az[ok] = np.degrees(np.unwrap(np.arctan2(vv[ok, 0], vv[ok, 2])))
            dh = np.zeros(n)
            idx = np.flatnonzero(ok)
            if len(idx) > 1:
                mid = np.radians((elev[idx[1:]] + elev[idx[:-1]]) / 2)
                dh[idx[1:]] = np.diff(az[idx]) * np.cos(mid)
            h = np.cumsum(dh)
            h[~ok] = np.nan
            # the signs from the finder's own turns (+500 counts right after its scan, 250 up before "find pitch"), else
            # from how the aim followed the controller over the whole run
            ev = {text: r for t, r, text in E}
            def signed(arr, ctl, e0, e1):
                a, b = self.row.get(ev.get(e0, -1)), self.row.get(ev.get(e1, -1))
                if a is None or b is None or np.isnan(arr[a]) or np.isnan(arr[b]) or abs(ctl[b] - ctl[a]) < 1e-9:
                    okd = ok[1:] & ok[:-1]
                    return 1 if np.nansum(np.diff(arr)[okd] * np.diff(ctl)[okd]) >= 0 else -1
                return 1 if (arr[b] - arr[a]) * (ctl[b] - ctl[a]) >= 0 else -1
            scan = next((t for t in ev if t.startswith('find scan')), '')
            y1 = next((t for t in ev if t.startswith('find yaw1')), '')
            y3 = next((t for t in ev if t.startswith('find yaw3')), '')
            sx = signed(az, self.cx, scan, y1)
            sy = signed(elev, self.cy, y3, 'find pitch')
            self.ax = sx * az
            self.arc = sx * h
            self.ay = sy * elev
            self.elev = elev
        self.S = np.array([[int(s[0]), int(s[1]), float(s[2]), float(s[3]), int(s[4])] for s in S]) if S else np.zeros((0, 5))
        self.T = np.array([[int(x[0]), int(x[1]), int(x[2])] for x in T], dtype=np.int64) if T else np.zeros((0, 3), np.int64)
        self.P = np.array([[int(x[0]), int(x[1]), int(x[2]), int(x[3])] for x in P], dtype=np.int64) if P else np.zeros((0, 4), np.int64)
        self.F = {int(x[0]): int(x[1]) for x in F}

    def at(self, read):
        return self.row.get(read)

    def trials(self, prefix):
        """(event text, start row, end row) for each "<prefix> ..." event followed by its "end"."""
        out = []
        for i, (t, r, text) in enumerate(self.E):
            if text.split(' ')[0] != prefix:
                continue
            for t2, r2, text2 in self.E[i + 1:]:
                if text2 == 'end':
                    if self.at(r) is not None and self.at(r2) is not None:
                        out.append((text, self.at(r), self.at(r2)))
                    break
        return out


def kv(text):
    return dict(x.split('=', 1) for x in text.split(' ')[1:] if '=' in x)


def table(head, rows):
    print('| ' + ' | '.join(head) + ' |')
    print('|' + '---|' * len(head))
    for r in rows:
        print('| ' + ' | '.join(str(x) for x in r) + ' |')
    print()


def f(x, d=4):
    return '-' if x is None or (isinstance(x, float) and math.isnan(x)) else f'{x:.{d}f}'


# ---- each test
def timing(run):
    dt = np.diff(run.t) / 1e6
    dt = dt[(dt > 0) & (dt < 200)]
    new = run.new[run.new > 0]
    ticks = np.unique(run.S[:, 1]) if len(run.S) else np.array([])
    tick_dt = np.diff(ticks) / 1e6 if len(ticks) > 1 else np.array([0.0])
    # read to the game's present of that frame, and that present to the host present (display)
    rp, pf = [], []
    for p in run.P:
        i = run.at(int(p[3]))
        if i is not None:
            rp.append((p[1] - run.t[i]) / 1e6)
        if int(p[2]) in run.F:
            pf.append((run.F[int(p[2])] - p[1]) / 1e6)
    rp, pf = np.array(rp), np.array(pf)
    print('## Timing')
    table(['quantity', 'mean', 'p50', 'p95', 'max', 'n'], [
        ['six-axis read interval (ms)', f(dt.mean(), 2), f(np.median(dt), 2), f(np.percentile(dt, 95), 2), f(dt.max(), 2), len(dt)],
        ['new samples per read', f(new.mean(), 2), f(np.median(new), 0), f(np.percentile(new, 95), 0), f(new.max(), 0), len(new)],
        ['motion ticks with samples, interval (ms)', f(tick_dt.mean(), 2), f(np.median(tick_dt), 2), f(np.percentile(tick_dt, 95), 2), f(tick_dt.max(), 2), len(tick_dt)],
        ['read to the game\'s present (ms)', f(rp.mean(), 2) if len(rp) else '-', f(np.median(rp), 2) if len(rp) else '-', f(np.percentile(rp, 95), 2) if len(rp) else '-', f(rp.max(), 2) if len(rp) else '-', len(rp)],
        ['present to host present (ms)', f(pf.mean(), 2) if len(pf) else '-', f(np.median(pf), 2) if len(pf) else '-', f(np.percentile(pf, 95), 2) if len(pf) else '-', f(pf.max(), 2) if len(pf) else '-', len(pf)],
    ])


def gain_rows(run, prefix, keyf):
    res = {}
    for text, a, b in run.trials(prefix):
        k = kv(text)
        n = int(k['n'])
        axis = k.get('axis', 'x')
        aim = (run.ax if axis == 'x' else run.ay)
        ctl = (run.cx if axis == 'x' else run.cy)
        res.setdefault(keyf(k), []).append((n, (aim[b] - aim[a]) / n, (ctl[b] - ctl[a]) / n))
    return res


def ref_gain(run, axis='x'):
    """Aim degrees per count of the run's reference move: 1200 counts/s at 1000 Hz (lin, else rate), a 100-count packet
    from rest (cap), else the setting."""
    for prefix, key in (('lin', lambda k: k.get('axis') == axis and float(k['v']) == 1200),
                        ('rate', lambda k: axis == 'x' and float(k['v']) == 1200 and float(k['hz']) == 1000),
                        ('cap', lambda k: axis == 'x' and abs(int(k['n'])) == 100)):
        g = []
        for text, a, b in run.trials(prefix):
            k = kv(text)
            if key(k):
                aim = run.ax if axis == 'x' else run.ay
                g.append((aim[b] - aim[a]) / int(k['n']))
        if g:
            return float(np.mean(g))
    return run.k


def lin(run):
    print('## Linearity (aim degrees per count, 1000 Hz mouse)')
    res = gain_rows(run, 'lin', lambda k: (k['axis'], float(k['v'])))
    ref = {}
    for (axis, v), g in res.items():
        if v == 1200:
            ref[axis] = np.mean([x[1] for x in g])
    rows = []
    for (axis, v), g in sorted(res.items()):
        gp = [x[1] for x in g if x[0] > 0]
        gm = [x[1] for x in g if x[0] < 0]
        mean = np.mean([x[1] for x in g])
        ctl = np.mean([x[2] for x in g])
        rows.append([axis, f'{v:g}', f'{v * run.k:g}', abs(g[0][0]), f(gp[0] if gp else None, 5), f(gm[0] if gm else None, 5),
                     f(mean / run.k, 4), f((mean / ref[axis] - 1) * 100 if axis in ref else None, 2),
                     f(abs(gp[0] - gm[0]) / mean * 100 if gp and gm else None, 2),
                     f((ctl / run.k - 1) * 100, 3)])
    table(['axis', 'counts/s', 'controller deg/s', 'counts', 'aim deg/count +', 'aim deg/count -',
           'aim / controller', 'vs 1200/s %', '+/- asymmetry %', 'controller error %'], rows)
    return res


def roundtrip(run):
    print('## Round trips and drift')
    gx, gy = ref_gain(run, 'x'), ref_gain(run, 'y')
    rows = []
    for text, a, b in run.trials('rt') + run.trials('idle'):
        rows.append([text, f(run.ax[b] - run.ax[a], 4), f(run.ay[b] - run.ay[a], 4),
                     f((run.ax[b] - run.ax[a]) / gx, 2), f((run.ay[b] - run.ay[a]) / gy, 2),
                     f(run.cx[b] - run.cx[a], 5), f(run.cy[b] - run.cy[a], 5)])
    table(['test', 'aim x residual (deg)', 'aim y residual (deg)', 'x (counts)', 'y (counts)', 'controller x (deg)',
           'controller y (deg)'], rows)


def first_change(arr, a, b, tol):
    for i in range(a + 1, b + 1):
        if abs(arr[i] - arr[a]) > tol:
            return i
    return None


def step(run):
    print('## Step response (one packet; frames counted from the read that hands the counts to the game)')
    by, lost = {}, {}
    for text, a, b in run.trials('step'):
        k = kv(text)
        n = int(k['n'])
        axis = k['axis']
        aim = run.ax if axis == 'x' else run.ay
        ctl = run.cx if axis == 'x' else run.cy
        total = aim[b] - aim[a]
        r1 = first_change(ctl, a, b, 1e-6)
        lost.setdefault((axis, abs(n)), []).append(abs(total) < 0.05 * abs(n) * run.k)
        if r1 is None or abs(total) < 1e-9:
            continue
        fr = [(aim[i] - aim[a]) / total for i in range(r1, min(b, r1 + 12) + 1)]
        split = abs((ctl[r1] - ctl[a]) / (ctl[b] - ctl[a]) - 1) > 1e-3
        def reach(th):
            for j, x in enumerate(fr):
                if x >= th:
                    return j
            return None
        by.setdefault((axis, abs(n)), []).append((reach(0.5), reach(0.9), reach(0.999), max(fr) - 1, total / n, split, fr[:4]))
    rows = []
    for key in sorted(lost):
        axis, n = key
        lo = lost[key]
        v = [y for y in by.get(key, []) if abs(y[4]) > 0.05 * run.k]   # the trials the aim moved in
        if not v:
            rows.append([axis, n, len(lo), sum(lo)] + ['-'] * 8)
            continue
        def stat(i):
            x = [y[i] for y in v if y[i] is not None]
            return f'{min(x)}-{max(x)}' if x else '-'
        g = [y[4] for y in v]
        rows.append([axis, n, len(lo), sum(lo), stat(0), stat(1), stat(2),
                     f(max(y[3] for y in v) * 100, 3), f(np.mean(g), 5), f(min(g), 5), f(max(g), 5),
                     ' '.join(f'{x:.3f}' for x in v[0][6])])
    table(['axis', 'counts', 'trials', 'lost (aim did not move)', 'frames to 50%', 'to 90%', 'to 100%', 'overshoot %',
           'aim deg/count mean', 'min', 'max', 'fraction at frames 0..3 (first trial)'], rows)


def phase(run):
    print('## Latency: a packet sent a given time before the game\'s next read')
    rows = []
    recs = []
    for text, a, b in run.trials('phase'):
        k = kv(text)
        t0 = int(k['t0'])
        # the read that handed the counts to the game, the first read after they were sent, the aim's first move
        rd = first_change(run.cx, a, b, 1e-6)
        if rd is None:
            continue
        nxt = next((i for i in range(a, b + 1) if run.t[i] > t0), None)
        ra = first_change(run.ax, a, b, 1e-6)
        took = run.T[(run.T[:, 0] >= t0)]
        t_take = int(took[0, 0]) if len(took) else None
        lead = (run.t[nxt] - t0) / 1e6 if nxt is not None else None
        pres = run.P[run.P[:, 3] == run.read[rd]]
        flip = run.F.get(int(pres[0, 2])) if len(pres) else None
        recs.append(dict(lead=lead, frames=rd - nxt if nxt is not None else None, take=(t_take - t0) / 1e6 if t_take else None,
                         to_read=(run.t[rd] - t0) / 1e6, aim_frames=(ra - rd) if ra is not None else None,
                         lost=ra is None or abs(run.ax[b] - run.ax[a]) < 0.5 * abs(int(k['n'])) * run.k,
                         present=(pres[0, 1] - t0) / 1e6 if len(pres) else None,
                         display=(flip - t0) / 1e6 if flip else None))
    if not recs:
        print('(no phase trials)\n')
        return
    bins = {}
    for r in recs:
        if r['lead'] is None:
            continue
        bins.setdefault(int(r['lead'] // 2) * 2, []).append(r)
    for lo, v in sorted(bins.items()):
        def m(key):
            x = [y[key] for y in v if y[key] is not None]
            return f(np.mean(x), 2) if x else '-'
        rows.append([f'{lo}-{lo + 2}', len(v), f(np.mean([y['frames'] == 0 for y in v]) * 100, 0),
                     f(np.mean([y['lost'] for y in v]) * 100, 0), m('take'), m('to_read'),
                     m('aim_frames'), m('present'), m('display')])
    allr = recs
    def mm(key):
        x = [y[key] for y in allr if y[key] is not None]
        return f(np.mean(x), 2) if x else '-', f(np.max(x), 2) if x else '-'
    rows.append(['all', len(allr), f(np.mean([y['frames'] == 0 for y in allr if y['frames'] is not None]) * 100, 0),
                 f(np.mean([y['lost'] for y in allr]) * 100, 0),
                 '/'.join(mm('take')), '/'.join(mm('to_read')), '/'.join(mm('aim_frames')), '/'.join(mm('present')),
                 '/'.join(mm('display'))])
    table(['sent before the next read (ms)', 'trials', 'in that read %', 'aim did not take it %', 'to the motion thread (ms)',
           'to the read that has it (ms)', 'frames to the aim (mean/max)', 'to that frame\'s present (ms)',
           'to its display (ms)'], rows)


def rate(run):
    print('## Mouse polling rate')
    g0 = ref_gain(run)
    rows = []
    for text, a, b in run.trials('rate'):
        k = kv(text)
        n = int(k['n'])
        v = float(k['v'])
        gain = (run.ax[b] - run.ax[a]) / n
        r1 = first_change(run.cx, a, b, 1e-6)
        # per-frame turn while the mouse moves at a constant speed: the aim's speed per read interval
        last = max(i for i in range(a, b + 1) if abs(run.cx[i] - run.cx[b]) > 1e-6) if r1 else None
        sp = []
        if r1 and last and last - r1 > 6:
            for i in range(r1 + 2, last):
                dtr = (run.t[i] - run.t[i - 1]) / 1e9
                sp.append((run.ax[i] - run.ax[i - 1]) / dtr / (v * g0) * math.copysign(1, n))
        sp = np.array(sp)
        rows.append([f'{v:g}', k['hz'], n, f(gain, 5), f((gain / g0 - 1) * 100, 3),
                     f(sp.mean(), 3) if len(sp) else '-', f(sp.std() * 100, 1) if len(sp) else '-',
                     f(sp.min(), 2) if len(sp) else '-', f(sp.max(), 2) if len(sp) else '-'])
    table(['counts/s', 'Hz', 'counts', 'aim deg/count', 'vs reference %', 'aim speed per frame / steady (mean)',
           'its spread (sd %)', 'min', 'max'], rows)


def fields(run):
    print('## What the game reads of the sensor (one packet of 200 counts with a field changed)')
    names = {0: 'as made', 1: 'rate only (attitude held)', 2: 'attitude only (rate 0)', 3: 'delta_ns doubled'}
    rows = []
    for text, a, b in run.trials('field'):
        k = kv(text)
        n = int(k['n'])
        axis = k.get('axis', 'x')
        aim, other = (run.ax, run.ay) if axis == 'x' else (run.ay, run.ax)
        rows.append([axis, names[int(k['mode'])], n, f((aim[b] - aim[a]) / n, 5), f(other[b] - other[a], 3)])
    table(['axis', 'samples', 'counts', 'aim deg/count on that axis', 'the other axis moved (deg)'], rows)


def pitchyaw(run):
    print('## Yaw gain with the aim pitched')
    g0 = ref_gain(run)
    rows = []
    for text, a, b in run.trials('pitchyaw'):
        k = kv(text)
        n = int(k['n'])
        rows.append([k['pitch_counts'], f(run.cy[a], 2), f(run.ay[a], 2), n, f((run.ax[b] - run.ax[a]) / n, 5),
                     f(((run.ax[b] - run.ax[a]) / n / g0 - 1) * 100, 2), f((run.arc[b] - run.arc[a]) / n, 5),
                     f(run.ay[b] - run.ay[a], 3)])
    table(['controller pitch (counts)', 'controller pitch (deg, + down)', 'aim pitch (deg, + down)', 'counts',
           'aim yaw deg/count', 'vs level %', 'screen-horizontal deg/count', 'aim pitch moved (deg)'], rows)


def yawaxis(run):
    print('## Yaw with the controller pitched: rate about the world\'s up (body=0) or the controller\'s own axis (body=1)')
    g0 = ref_gain(run)
    rows = []
    for text, a, b in run.trials('yawaxis'):
        k = kv(text)
        n = int(k['n'])
        g = (run.ax[b] - run.ax[a]) / n
        rows.append([k['body'], k['pitch_counts'], f(run.cy[a], 2), f(run.ay[a], 2), n, f(g, 5), f((g / g0 - 1) * 100, 2),
                     f(run.ay[b] - run.ay[a], 3)])
    table(['body', 'controller pitch (counts)', 'controller pitch (deg)', 'aim pitch (deg)', 'counts', 'aim yaw deg/count',
           'vs level %', 'aim pitch moved (deg)'], rows)


def reverse(run):
    print('## Reversals: back and forth at 1200 counts/s, 240 counts a leg, with reads without counts between legs')
    g0 = ref_gain(run)
    yaw_of = {int(s[0]): s[2] for s in run.S}
    rows = []
    for text, a, b in run.trials('reverse'):
        k = kv(text)
        pause = k['pause']
        hold = k.get('hold', '')
        # per read: counts handed (the controller's change in the newest state, so either scheme counts right) and the
        # aim's change one read later; the frames after each change of sign
        legs = [(t, r, tx) for t, r, tx in run.E if tx.startswith('leg ') and f'pause={pause} ' in tx
                and run.read[a] <= r <= run.read[b]]
        ratios = {}
        lost = []
        for j in range(1, len(legs)):
            i0 = run.at(legs[j][1])
            i1 = run.at(legs[j + 1][1]) if j + 1 < len(legs) else b
            if i0 is None or i1 is None:
                continue
            exp_sum = got_sum = 0
            m = 0
            for i in range(i0 + 1, i1):
                c = (run.cx[i] - run.cx[i - 1]) / run.k   # counts handed at read i
                d = run.ax[i + 1] - run.ax[i] if i + 1 <= b else 0
                exp_sum += c * g0
                got_sum += d
                if abs(c) > 1e-9:
                    ratios.setdefault(m, []).append(d / (c * g0))
                    m += 1
            lost.append((abs(exp_sum) - abs(got_sum)) / g0)
        first = ' '.join(f'{np.mean(ratios[m]):.2f}' for m in range(0, 10) if m in ratios)
        rows.append([{'0': 'none', '-1': 'input.c (2 reads)', '': '-'}.get(hold, hold), pause, f(run.ax[b] - run.ax[a], 3),
                     f((run.ax[b] - run.ax[a]) / g0, 1), f(np.mean(lost), 1) if lost else '-',
                     f(np.max(lost), 1) if lost else '-', first])
    table(['hold at a reversal', 'reads without counts between legs', 'aim residual after 5 cycles (deg)', 'in counts',
           'turn missing in a leg, mean (counts of 240)', 'max', 'aim / counts at frames 0..9 of a leg'], rows)


def cap(run):
    print('## One packet of n counts (one frame\'s turn): the fastest turn the game takes')
    g0 = ref_gain(run)
    rows = []
    for text, a, b in run.trials('cap'):
        n = int(kv(text)['n'])
        turned = run.ax[b] - run.ax[a]
        frames = sum(1 for i in range(a + 1, b + 1) if abs(run.ax[i] - run.ax[i - 1]) > 1e-4)
        rows.append([n, f(n * g0, 2), f(turned, 2), f((turned / (n * g0) - 1) * 100, 2), frames,
                     f(abs(n) * run.k * 60, 0)])
    table(['counts', 'aim turn expected (deg)', 'aim turn (deg)', 'error %', 'frames the aim moved in',
           'controller deg/s in that frame'], rows)


def recenter(run):
    print('## Recentre (Y) with the controller pitched 10 degrees down')
    rows = []
    for text, a, b in run.trials('recenter'):
        rows.append([text, f(run.cy[a], 2), f(run.cy[b], 2), f(run.ay[a], 2), f(run.ay[b], 2), f(run.ax[b] - run.ax[a], 2)])
    table(['step', 'controller pitch before', 'after', 'aim pitch before', 'after', 'aim yaw moved'], rows)


def limits(run):
    print('## Pitch limits (controller and aim pitch after each 125-count step; + is down)')
    rows = []
    for t, r, text in run.E:
        if text.startswith('lim'):
            i = run.at(r)
            if i is not None:
                rows.append([text, f(run.cy[i], 2), f(run.ay[i], 2)])
    table(['step', 'controller pitch', 'aim pitch'], rows)


def hitch(run):
    print('## A stalled frame (the game thread held in one read) during a 1200 counts/s move')
    g0 = ref_gain(run)
    rows = []
    for text, a, b in run.trials('hitch'):
        k = kv(text)
        n = int(k['n'])
        rows.append([text, f((run.ax[b] - run.ax[a]) / n, 5), f(((run.ax[b] - run.ax[a]) / n / g0 - 1) * 100, 3),
                     int(run.new[a:b + 1].max()), f(((run.ax[b] - run.ax[a]) - n * g0) / g0, 1)])
    table(['test', 'aim deg/count', 'vs reference %', 'most new samples in one read', 'counts lost'], rows)


def fps(run):
    print('## Frame limit')
    g0 = ref_gain(run)
    rows = []
    for text, a, b in run.trials('fps'):
        k = kv(text)
        n = int(k['n'])
        dt = np.diff(run.t[a:b + 1]) / 1e6
        rows.append([text, f((run.ax[b] - run.ax[a]) / n, 5), f(((run.ax[b] - run.ax[a]) / n / g0 - 1) * 100, 3),
                     f(np.median(dt), 2)])
    table(['test', 'aim deg/count', 'vs reference (60 fps) %', 'read interval p50 (ms)'], rows)


# ---- pictures: a minimal PNG reader (8-bit RGB or RGBA, the five filters), so no imaging library is needed
def read_png(path):
    data = Path(path).read_bytes()
    pos, idat, w = 8, b'', 0
    while pos < len(data):
        n, typ = struct.unpack('>I4s', data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if typ == b'IHDR':
            w, h, depth, ctype = struct.unpack('>IIBB', body[:10])
            bpp = 3 if ctype == 2 else 4
        elif typ == b'IDAT':
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = w * bpp
    out = np.zeros((h, stride), np.uint8)
    prev = np.zeros(stride, np.int32)
    for y in range(h):
        ft = raw[y * (stride + 1)]
        row = np.frombuffer(raw, np.uint8, stride, y * (stride + 1) + 1).astype(np.int32)
        if ft == 0:
            cur = row
        elif ft == 2:
            cur = (row + prev) & 255
        else:
            cur = np.zeros(stride, np.int32)
            r, p = row.tolist(), prev.tolist()
            c = [0] * stride
            for i in range(stride):
                a = c[i - bpp] if i >= bpp else 0
                if ft == 1:
                    pr = a
                elif ft == 3:
                    pr = (a + p[i]) >> 1
                else:
                    b_, cc = p[i], p[i - bpp] if i >= bpp else 0
                    pa, pb, pc = abs(b_ - cc), abs(a - cc), abs(a + b_ - 2 * cc)
                    pr = a if pa <= pb and pa <= pc else b_ if pb <= pc else cc
                c[i] = (r[i] + pr) & 255
            cur = np.array(c, np.int32)
        out[y] = cur
        prev = cur
    return out.reshape(h, w, bpp)[:, :, :3].astype(np.float64).mean(axis=2)


def shift_x(a, b):
    """Horizontal shift of b against a (pixels), by phase correlation of the middle band."""
    h = a.shape[0]
    A = a[h // 3:2 * h // 3]
    B = b[h // 3:2 * h // 3]
    A = A - A.mean()
    B = B - B.mean()
    X = np.fft.fft2(A) * np.conj(np.fft.fft2(B))
    X /= np.abs(X) + 1e-9
    c = np.real(np.fft.ifft2(X)).sum(axis=0)
    k = int(np.argmax(c))
    return k if k <= len(c) // 2 else k - len(c)


def image(run):
    d = Path(str(run.path) + '-shots')
    if not d.is_dir():
        return
    print('## The picture: horizontal shift of each presented frame after a 400-count packet')
    shots = sorted(d.glob('p*-r*.png'))
    if not shots:
        return
    info = [(int(re.search(r'p(\d+)', s.name).group(1)), int(re.search(r'r(\d+)', s.name).group(1)), s) for s in shots]
    rows = []
    for text, a, b in run.trials('image'):
        rd = first_change(run.cx, a, b, 1e-6)
        mine = [x for x in info if run.read[a] <= x[1] <= run.read[b]]
        if rd is None or len(mine) < 3:
            continue
        imgs = [read_png(x[2]) for x in mine]
        shifts = [0] + [shift_x(imgs[i - 1], imgs[i]) for i in range(1, len(imgs))]
        moved = [x for x, s in zip(mine, shifts) if abs(s) > 2]
        first = moved[0] if moved else None
        rows.append([text, run.read[rd], ' '.join(f'p{x[0]}/r{x[1]}:{s}' for x, s in zip(mine, shifts)),
                     f'p{first[0]} (after read {first[1]}): {first[1] - run.read[rd]} frame(s) after the read' if first else '-'])
    table(['test', 'read with the counts', 'present/read: shift (px)', 'first frame that moved'], rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('csv')
    ap.add_argument('--aim', type=int, default=-1, help='aim vector index (default: the first of the view)')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    run = Run(a.csv, a.aim)
    print(f'# {Path(a.csv).name}\n')
    print(f'deg_per_count {run.k}; aim {run.aim}: {run.aim_info}; {len(run.read)} reads\n')
    tests = [timing, lin, step, phase, rate, roundtrip, reverse, fields, pitchyaw, yawaxis, cap, recenter, limits, hitch,
             fps, image]
    for t in tests:
        if a.only and t.__name__ not in a.only.split(','):
            continue
        t(run)


if __name__ == '__main__':
    sys.exit(main())
