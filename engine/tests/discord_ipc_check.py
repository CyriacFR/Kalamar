"""The Discord presence of the game (runtime/discord_rpc.c) against a stand-in Discord: never the real one.

    python tests/discord_ipc_check.py <splat3.exe or totk.exe>

Windows only. Serves the pipe \\\\.\\pipe\\kalamar-test-<pid>-0 (a name of its own: a Discord running on the machine is
never reached) and runs `<exe> --probe-discord` with SPLAT3_DISCORD_IPC naming it:

1. with an application id: the stand-in gets the handshake with that id, then one SET_ACTIVITY with the launcher's
   minimal words (Splatoon 3 target: "In game", the picture kalamar_logo, the brand's name as hover text, a start time,
   nothing else), then the clear;
2. with SPLAT3_DISCORD_PRESENCE=launcher (what the launcher passes to every game it starts): the probe says it skips and
   the stand-in sees no connection at all;
3. without an application id on a target that has no default one: no connection;
4. with a SPLAT3_DISCORD_IPC that is not a plain name: refused, no connection.

No game files: the probe needs none. The stand-in answers the handshake with READY and every command with an answer
carrying its nonce, as the Discord client does.
"""
import json
import os
import struct
import subprocess
import sys
import threading
import time

if sys.platform != 'win32':
    print('discord_ipc_check: Windows only (named pipes); skipped')
    sys.exit(0)

import _winapi  # noqa: E402  (CPython's own Windows API module, the one multiprocessing uses for its pipes)

TEST_APP = '100000000000000001'   # not a real application: it only ever reaches the stand-in
WAIT_TIMEOUT = 0x102
ERROR_IO_PENDING = 997
PIPE_TYPE_BYTE = PIPE_READMODE_BYTE = PIPE_WAIT = 0   # winbase.h; _winapi names only the message-mode ones


class StandIn:
    """One instance of a byte-mode pipe, overlapped, served by a thread until stop()."""

    def __init__(self, name):
        assert not name.startswith('discord-ipc'), 'the stand-in never takes Discord\'s own pipe names'
        self.path = '\\\\.\\pipe\\' + name
        self.frames = []        # (opcode, payload bytes) as received
        self.connections = 0
        self.done = threading.Event()
        self.handle = self._create()   # the pipe exists from here on
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def _create(self):
        return _winapi.CreateNamedPipe(
            self.path, _winapi.PIPE_ACCESS_DUPLEX | _winapi.FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 65536, 65536,
            _winapi.NMPWAIT_WAIT_FOREVER, _winapi.NULL)

    def _wait(self, ov, ms):
        while not self.done.is_set():
            if _winapi.WaitForMultipleObjects([ov.event], False, min(ms, 100)) != WAIT_TIMEOUT:
                return True
            ms -= 100
            if ms <= 0:
                break
        ov.cancel()
        return False

    def _write(self, op, payload):
        data = struct.pack('<II', op, len(payload)) + payload
        ov, err = _winapi.WriteFile(self.handle, data, overlapped=True)
        ov.GetOverlappedResult(True)

    def serve(self):
        while not self.done.is_set():
            ov = _winapi.ConnectNamedPipe(self.handle, overlapped=True)
            if not self._wait(ov, 60000):
                return
            _, err = ov.GetOverlappedResult(True)
            if err:
                return
            self.connections += 1
            buf = b''
            while not self.done.is_set():
                try:
                    ov, err = _winapi.ReadFile(self.handle, 65536, overlapped=True)
                except OSError:
                    break
                if err == ERROR_IO_PENDING and not self._wait(ov, 60000):
                    return
                try:
                    _, err = ov.GetOverlappedResult(True)
                except OSError:
                    break
                if err:
                    break
                buf += ov.getbuffer()
                while len(buf) >= 8:
                    op, n = struct.unpack('<II', buf[:8])
                    if len(buf) < 8 + n:
                        break
                    payload, buf = bytes(buf[8:8 + n]), buf[8 + n:]
                    self.frames.append((op, payload))
                    body = json.loads(payload) if payload else None
                    if op == 0:
                        self._write(1, json.dumps({'cmd': 'DISPATCH', 'evt': 'READY', 'data': {'v': 1, 'config': {},
                                                   'user': {'id': '1', 'username': 'test'}}}).encode())
                    elif op == 1 and body and body.get('cmd') == 'SET_ACTIVITY':
                        self._write(1, json.dumps({'cmd': 'SET_ACTIVITY', 'evt': None, 'nonce': body.get('nonce'),
                                                   'data': body['args'].get('activity')}).encode())
            # the client went: a new instance for the next one (the old one closed first, so the name stays one pipe)
            _winapi.CloseHandle(self.handle)
            self.handle = self._create()

    def stop(self):
        self.done.set()
        self.thread.join(5)
        _winapi.CloseHandle(self.handle)


def probe(exe, base, extra):
    env = {k: v for k, v in os.environ.items()
           if not k.upper().startswith(('TOTK_DISCORD', 'KLM_DISCORD', 'SPLAT3_DISCORD'))}
    env.update({'SPLAT3_DISCORD_IPC': base, 'TOTK_MENU_LANG': 'en'})
    env.update(extra)
    r = subprocess.run([exe, '--probe-discord'], env=env, capture_output=True, text=True, timeout=60,
                       cwd=os.path.dirname(os.path.abspath(exe)))
    return r.returncode, r.stderr


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    exe = sys.argv[1]
    fails = []

    def check(ok, what):
        print(('ok   ' if ok else 'FAIL ') + what)
        if not ok:
            fails.append(what)

    base = 'kalamar-test-%d' % os.getpid()

    # 1. an application named: the exchange, and exactly the launcher's minimal words
    s = StandIn(base + '-0')
    code, err = probe(exe, base, {'TOTK_DISCORD_APP_ID': TEST_APP})
    time.sleep(0.3)
    s.stop()
    check(code == 0 and 'probe passed' in err, 'the probe passes against the stand-in: ' + err.strip().replace('\n', ' | '))
    hs = [json.loads(p) for op, p in s.frames if op == 0]
    check(hs == [{'v': 1, 'client_id': TEST_APP}], 'one handshake with the application id: %r' % hs)
    acts = [json.loads(p) for op, p in s.frames if op == 1]
    check(len(acts) == 2, 'one activity then one clear: %d commands' % len(acts))
    upstream = False
    if len(acts) == 2:
        a = acts[0]['args'].get('activity') or {}
        check(set(acts[0]['args']) == {'pid', 'activity'}, 'the command carries the process id and the activity')
        upstream = ((a.get('assets') or {}).get('large_text') or '').startswith('TotK')
        if upstream:
            check(a['assets']['large_image'], 'the upstream target keeps its own art')
        else:
            check(a.get('details') == 'In game', 'details "In game": %r' % a.get('details'))
            check(a.get('assets') == {'large_image': 'kalamar_logo', 'large_text': 'Kalamar'}, 'the logo and the brand: %r' % a.get('assets'))
            check(set(a) == {'details', 'timestamps', 'assets', 'instance'}, 'nothing else in the activity: %r' % sorted(a))
            start = (a.get('timestamps') or {}).get('start', 0)
            check(abs(start - time.time()) < 120, 'the start is the process\'s start: %r' % start)
            raw = b''.join(p for _, p in s.frames).decode('utf-8', 'replace')
            for bad in ('\\\\', ':\\', '/', 'Recomp', '+'):
                check(bad not in raw.replace('\\u', ''), 'no path, version or commit on the wire (%r)' % bad)
        check(set(acts[1]['args']) == {'pid'}, 'the clear has no activity: %r' % acts[1]['args'])

    # 2. started by the launcher: the probe skips, the pipe is never opened
    s = StandIn(base + '-0')
    code, err = probe(exe, base, {'TOTK_DISCORD_APP_ID': TEST_APP, 'SPLAT3_DISCORD_PRESENCE': 'launcher'})
    time.sleep(0.3)
    s.stop()
    check(code == 0 and 'started by the launcher' in err, 'started by the launcher: skipped (%s)' % err.strip())
    check(s.connections == 0 and not s.frames, 'started by the launcher: no connection (%d)' % s.connections)

    # 3. no application named: nothing opened (the upstream target has a default one and connects)
    s = StandIn(base + '-0')
    code, err = probe(exe, base, {})
    time.sleep(0.3)
    s.stop()
    if upstream:
        check(s.connections == 1, 'the upstream target uses its own application (%s)' % err.strip())
    else:
        check('no Discord application named' in err and s.connections == 0, 'no application: no connection (%d)' % s.connections)

    # 4. a test pipe name that is not a plain name: refused, not Discord's pipes instead
    code, err = probe(exe, '..\\discord-ipc', {'TOTK_DISCORD_APP_ID': TEST_APP})
    check(code == 1 and 'not a plain pipe name' in err, 'a pipe name with a path is refused (%s)' % err.strip())

    # 5. a launcher shows the presence (it holds the lock both take, Local\Kalamar.DiscordPresence): the game stays quiet
    import ctypes
    k32 = ctypes.WinDLL('kernel32', use_last_error=True)
    k32.CreateMutexW.restype = ctypes.c_void_p
    mutex = k32.CreateMutexW(None, True, 'Local\\Kalamar.DiscordPresence')
    owned = bool(mutex) and ctypes.get_last_error() != 183   # ERROR_ALREADY_EXISTS: someone else has it
    s = StandIn(base + '-0')
    code, err = probe(exe, base, {'TOTK_DISCORD_APP_ID': TEST_APP})
    time.sleep(0.3)
    s.stop()
    if owned and upstream:
        check(s.connections == 1, 'the upstream target takes no lock (%s)' % err.strip())
    elif owned:
        check('another Kalamar' in err and s.connections == 0, 'the lock is held: no connection (%s)' % err.strip())
    else:
        print('skip the lock is already held by another program of this session')
    if mutex:
        k32.ReleaseMutex(ctypes.c_void_p(mutex))
        k32.CloseHandle(ctypes.c_void_p(mutex))

    print('discord_ipc_check: %s' % ('FAILED: %d' % len(fails) if fails else 'passed'))
    return 1 if fails else 0


if __name__ == '__main__':
    sys.exit(main())
