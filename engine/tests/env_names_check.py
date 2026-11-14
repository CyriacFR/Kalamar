"""Every environment variable the documentation names goes by its KLM_ name, and the runtime answers to that name.

    python tests/env_names_check.py           check
    python tests/env_names_check.py --write   also rewrite the table of docs/settings.md, "Legacy names"

The runtime's code names its variables by their older names, TOTK_<NAME> (the upstream's) and SPLAT3_<NAME> (the first
port's); one function reads them all, runtime/cfg.c cfg_env, which answers to KLM_<NAME> first and to the older name
after, and one writes them, cfg_setenv. Fails when:

- a page names a variable by an older name, outside the "Legacy names" appendix of docs/settings.md (the few names
  the code generator reads are kept: recomp/ is not the runtime and has no KLM_ name; so are names that are not
  variables, such as a C macro);
- a documented KLM_<NAME> has no older name the runtime reads, or no row in the appendix, or a row that differs;
- the runtime reads or sets a variable of an older name around cfg_env and cfg_setenv (getenv, plat_getenv,
  plat_setenv, setenv, unsetenv, _putenv_s), outside cfg.c and the probes, which set test values directly.

No game files, no build.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APPENDIX = ROOT / 'docs' / 'settings.md'
HEADING = '## Legacy names'
OLDER = re.compile(r'\b(TOTK|SPLAT3)_([A-Z0-9][A-Z0-9_]*)')
NEUTRAL = re.compile(r'\bKLM_([A-Z0-9][A-Z0-9_]*)')
# Older names a page may keep: read by the code generator (recomp/gen.py), or not variables at all.
KEEP = {'TOTK_LIFT_JOBS', 'TOTK_LIFT_TEST_OOM_ABOVE', 'SPLAT3_KEYS', 'SPLAT3_KEY_COUNT'}
KEEP_PREFIXES = ('TOTK_RECOMP_',)
RAW = re.compile(r'\b(getenv|plat_getenv|plat_setenv|setenv|unsetenv|_putenv_s)\s*\(\s*"((?:TOTK|SPLAT3)_[A-Z0-9_]+)"')


def pages():
    out = [ROOT / 'README.md', ROOT / 'CONTRIBUTING.md', ROOT / 'kalamar.ini.example']
    out += sorted((ROOT / 'docs').rglob('*.md'))
    return [p for p in out if p.is_file()]


def without_appendix(path, text):
    if path == APPENDIX and HEADING in text:
        return text[:text.index(HEADING)]
    return text


def runtime_sources():
    for p in sorted((ROOT / 'runtime').rglob('*')):
        if p.suffix in ('.c', '.cpp', '.h'):
            yield p, p.read_text(encoding='utf-8', errors='replace')


def known_rests(cfg):
    start = cfg.index('static const char *const known[]')
    return set(re.findall(r'"([A-Z0-9_]+)"', cfg[start:cfg.index('};', start)]))


def older_names(sources, cfg):
    """{rest: older name} of every variable the runtime reads, from its string literals, known[] and rt_knob calls."""
    found, both = {}, set()
    for _, text in sources:
        for prefix, rest in re.findall(r'"(TOTK|SPLAT3)_([A-Z0-9_]+)"', text):
            if found.setdefault(rest, f'{prefix}_{rest}') != f'{prefix}_{rest}':
                both.add(rest)
        for rest in re.findall(r'rt_knob\("([A-Z0-9_]+)"', text):
            found.setdefault(rest, f'TOTK_{rest}')
    for rest in known_rests(cfg):
        found.setdefault(rest, f'TOTK_{rest}')
    return found, both


def older_of(rest, older):
    if rest in older:
        return older[rest]
    if rest.endswith('_'):   # a family named by its prefix (KLM_BIND_<button>)
        hits = sorted({v[:len(v) - len(r) + len(rest)] for r, v in older.items() if r.startswith(rest)})
        return hits[0] if hits else None
    return None


def table(rows):
    lines = ['| Name | Older name, still read |', '|---|---|']
    lines += [f'| `{n}` | `{o}` |' for n, o in sorted(rows.items())]
    return '\n'.join(lines) + '\n'


def main():
    write = '--write' in sys.argv[1:]
    sources = list(runtime_sources())
    cfg = (ROOT / 'runtime' / 'cfg.c').read_text(encoding='utf-8')
    older, both = older_names(sources, cfg)
    # KLM_<NAME> answers for both older names: two variables sharing a <NAME> would read each other's value
    problems = [f'TOTK_{r} and SPLAT3_{r}: both read by the runtime, under one KLM_{r}' for r in sorted(both)]
    documented = set()
    for page in pages():
        text = without_appendix(page, page.read_text(encoding='utf-8'))
        rel = page.relative_to(ROOT).as_posix()
        for m in OLDER.finditer(text):
            name = m.group(0)
            if name in KEEP or name.startswith(KEEP_PREFIXES):
                continue
            line = text.count('\n', 0, m.start()) + 1
            problems.append(f'{rel}:{line}: {name}: an older name; the documentation names it KLM_{m.group(2)}')
        documented |= {m.group(1) for m in NEUTRAL.finditer(text)}
    rows = {}
    for rest in sorted(documented):
        o = older_of(rest, older)
        if not o:
            problems.append(f'KLM_{rest}: documented, but the runtime reads no TOTK_{rest} or SPLAT3_{rest}')
            continue
        rows[f'KLM_{rest}'] = o
    for path, text in sources:
        if path.name == 'cfg.c' or path.stem.endswith('_probe'):
            continue
        for m in RAW.finditer(text):
            line = text.count('\n', 0, m.start()) + 1
            problems.append(f'{path.relative_to(ROOT).as_posix()}:{line}: {m.group(1)}("{m.group(2)}"): read or set it '
                            f'with cfg_env / cfg_setenv, which answer to KLM_{m.group(2).split("_", 1)[1]}')
    doc = APPENDIX.read_text(encoding='utf-8')
    want = table(rows)
    if HEADING not in doc:
        problems.append(f'docs/settings.md: no "{HEADING}" appendix')
    else:
        tail = doc[doc.index(HEADING):]
        start = tail.find('| Name |')
        have = tail[start:] if start >= 0 else ''
        have = have[:have.index('\n\n') + 1] if '\n\n' in have else have
        if have != want:
            if write:
                doc = doc[:doc.index(HEADING)] + tail[:start] + want + tail[start + len(have):]
                APPENDIX.write_bytes(doc.encode('utf-8'))
                print('env_names_check: the table of docs/settings.md was rewritten')
            else:
                problems.append('docs/settings.md: the "Legacy names" table differs from the documented names '
                                '(python tests/env_names_check.py --write)')
    for p in problems:
        print(p)
    print(f'env_names_check: {len(rows)} documented variables, {len(older)} older names read by the runtime, '
          f'{len(problems)} problems')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
