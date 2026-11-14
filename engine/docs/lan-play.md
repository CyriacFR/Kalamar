# LAN play (Splatoon 3)

Splatoon 3 has an official LAN mode: in the local lobby (scene `LobbyLocal`), hold ZL+ZR and the left stick for three
seconds. Private battles take up to 8
players and 2 spectators, Salmon Run 2 to 4. Consoles find each other with UDP broadcasts on the local network through
the game's session library (its LAN module) and contact no server. This runtime lets copies of the recompiled game
play that mode together:

- **lan mode**: on one local network, as consoles do (real UDP on the PC's LAN adapter);
- **relay mode**: over the internet, through a small UDP relay ([lan-relay-protocol.md](lan-relay-protocol.md)) that
  gives each copy an address on a virtual LAN, 10.13.0.0/16.

Status: two copies on one PC played a whole private battle (Turf War, room, teams, match, results, back in the room)
in lan mode on 127.0.0.2/8 and 127.0.0.3/8, and through the relay on 127.0.0.1 (room, teams and the match itself;
section 7). The quickest way in is `KLM_START=lobby-lan` ([save-profile.md](save-profile.md), Start scene): the
game starts in the local lobby in LAN mode, and `L` opens the Private Battle / Salmon Run menu.

## 1. Safety: off by default, and an allowlist when on

The game must never reach the game publisher's or anyone else's server. So:

- **Off by default.** Without `KLM_LAN` the runtime adds nothing: `runtime/ov_offline.c` answers every network call
  as a console without a connection, exactly as before. The executable imports no networking library; `ws2_32.dll`
  and `iphlpapi.dll` are loaded from System32 only when LAN play starts (`runtime/lan_host_win.c`). A run without it
  opens no TCP or UDP endpoint and loads neither DLL (checked with `Get-NetUDPEndpoint`, `Get-NetTCPConnection` and the
  process's module list).
- **UDP only.** A stream (TCP) or raw socket, or any family but IPv4, fails with ENETDOWN as offline: The session library's LAN module
  opens UDP sockets only (section 3), so nothing in LAN play needs TCP.
- **No name is ever resolved.** `GetAddrInfo`, `GetHostByName` and the resolver fail in every mode, numeric names too;
  nifm reports no DNS server and no gateway.
- **An allowlist decides every datagram before it is sent** (`lan_dst_check`, `runtime/lan_policy.c`):
  - lan mode: only the adapter's subnet (unicast) and its broadcast; 255.255.255.255 becomes the subnet's directed
    broadcast. The subnet must lie inside a private block (10/8, 172.16/12, 192.168/16, 169.254/16, or 127/8 for two
    copies on one PC), else LAN play does not start. Refused even inside the subnet: the network address, the default
    gateway, multicast, port 0 and service ports (DNS 53, DHCP 67/68, NTP 123, NetBIOS 137-139, SSDP 1900,
    NAT-PMP/PCP 5350/5351, mDNS 5353, LLMNR 5355).
  - relay mode: only the virtual subnet 10.13.0.0/16; the PC itself sends to the relay's address and nothing else, and
    takes datagrams from the relay's address only. The relay's address must be an IPv4 unicast literal, its port none
    of the service ports above, and the relay socket has no `SO_BROADCAST`.
  - received datagrams reach the game only from a source in the same subnet (`lan_src_ok`); a refused one never touches
    the game's buffer, and one receive call drops 64 at most before it answers.
- **Every refusal is logged** once per destination with the guest caller (`lan: REFUSED send to 8.8.8.8:443 (outside
  the LAN subnet) from main+0x...`); `lan_refusals()` counts them all.
- **Still offline in every mode**: nn::ssl and nnssl*, libcurl, NPLN/gRPC (they reach the network only through the
  socket calls, which refuse their TCP sockets and names), the network service account and its tokens, friends, bcat,
  prepo, nsd, nn::ldn (local wireless), the proxy setting. nifm accepts only a local-network request
  (`SetLocalNetworkMode(true)`): the internet request the game submits at boot stays unavailable.
- `--probe-lan` proves that a public address is refused in every mode: 16 addresses outside the private blocks
  (resolvers, CDNs, cloud and documentation ranges, neighbors of the private blocks) on 13 ports, in 7 configurations (off, three lan subnets, relay joined, relay not joined,
  relay with a hostile out-of-range assignment), and again end to end through the guest's `SendTo` and `Connect`.

## 2. Settings

Environment variables (not totk.ini keys: LAN play is chosen per launch).

| Variable | Values |
|---|---|
| `KLM_LAN` | unset, `off` or `0`: offline (default). `lan`: the up adapter with a private IPv4 address (one with a default gateway first). `lan:<a.b.c.d>/<prefix>`: that address of this PC; `lan:127.0.0.2/8` and `lan:127.0.0.3/8` run two copies on one PC. `relay:<a.b.c.d>[:<port>]`: relay mode, default port 47313 |
| `KLM_LAN_ROOM` | relay mode: the room code, 1-32 of `A-Z a-z 0-9 - _` (sent upper case); default `DEFAULT`. Anyone with the code can join: use a long random one on a public relay |
| `KLM_LAN_NAME` | relay mode: the name the relay gives the room's other members (32 bytes at most); automatic matches seat teams by it ([auto-match.md](auto-match.md)) |
| `KLM_LAN_BIND` | lan mode: `any` binds host sockets to 0.0.0.0 instead of the adapter's address (datagrams from outside the subnet are still dropped); try it if discovery fails on a real LAN |
| `KLM_NET_LOG` | `1`: log every nifm, socket and ldn call with its arguments, result, errno and caller, counting repetitive ones (receive with nothing queued, errno reads, unchanged status polls) in a line every 5 s; `2`: log every call. Works with LAN play off too |
| `KLM_NICKNAME` | the account's nickname, which Splatoon 3 shows as the player's name; it wins over `player_name` and the `save_profile` name ([save-profile.md](save-profile.md)); default `Local` |
| `KLM_WINDOW_TITLE` | the window's title, to tell two instances apart |

A malformed `KLM_LAN` stops the game at start with the reason (`FATAL: KLM_LAN=...`).

## 3. What the game needs

Measured from Splatoon 3 11.2.0's `main` (call sites of the imports, disassembly), and from runs with
`KLM_NET_LOG=1`. Offsets are into `main`.

- **The network-interface start** (`main+0x44f3e00`, a virtual method of the game's network system) takes a mode:
  1 internet (nifm with `SetLocalNetworkMode(false)`, `socket::Initialize`, libcurl's global init), 2 local wireless
  (`ldn::Initialize`, `ldn::SetProtocol`), **3 LAN** (`nifm::Initialize`, `SetLocalNetworkMode(true)`,
  `SubmitNetworkRequestAndWait`, `IsNetworkAvailable` must be true, then `socket::Initialize(pool, size, alloc size, 14)`).
  `SetLocalNetworkMode(mode == 3)` is the only call that tells a LAN request from an internet one. At boot the game
  starts mode 1 once: `SetLocalNetworkMode(false)`, `SubmitNetworkRequestAndWait`, `IsNetworkAvailable` = false.
- **Its own address**: `main+0x44f42b8` calls `nifm::GetCurrentIpConfigInfo(ip, mask, gateway, dns1, dns2)` and keeps the
  address and the mask. The session library's `LanService` start (`main+0x95bdb0`) needs a valid address and a contiguous mask and
  computes the broadcast address as `ip | ~mask`.
- **The session library's LAN sockets** (`main+0x95cdfc`, binding helper `main+0x95d010`): three `Socket(AF_INET, SOCK_DGRAM,
  IPPROTO_UDP)` bound to 0.0.0.0, `SO_REUSEADDR` set before `bind` and `SO_BROADCAST` after:
  - UDP **49152**: the session transport (the range 49152-49152 by default);
  - UDP **35000**: discovery broadcasts;
  - UDP **40000**: only when a flag of the LAN service is set (`+0x31`), purpose unknown.
  Another start mode (an argument the caller passes as 0) binds ranges 49152..49151+n and 35000..34999+n without
  `SO_REUSEADDR`; Splatoon 3 does not use it.
- **The session library's socket calls** (wrapper at `main+0x92bba8`-`0x92cab0`, the typed `nn::socket` API): `Socket`, `Bind`,
  `SetSockOpt` (SOL_SOCKET SO_REUSEADDR, SO_BROADCAST, SO_REUSEPORT; IPPROTO_IP IP_TTL), `SendTo` with no flags,
  `RecvFrom` with `MSG_DONTWAIT` (polled, no `Poll`, no `fcntl`), `Close`, `GetLastError`; `GetAddrInfo` for the
  internet modes. Guest `sockaddr_in` is FreeBSD's: `{u8 len = 16, u8 family = 2, be16 port, be32 address, 8 zero}`.
- **Not LAN play's**: `nn::ldn` is the session library's local-wireless module (`main+0x950d94`-`0x957694`, mode 2); the C shims
  `nnsocket*` serve gRPC (eventfd, sendmsg, accept, `main+0x1ad490`-`0x1bcac0`) and libcurl's socket layer
  (`main+0x6ab188`-`0x6aba10`, name resolution, connect); another UDP user at `main+0x5b5a40` resolves names first.
  In LAN play these get UDP sockets limited by the allowlist, and no TCP socket or name at all.

## 4. How it works

`runtime/ov_lan.c` adds overrides for the 30 `nn::socket` functions, the 29 `nnsocket*` shims and the 9 `nn::nifm`
functions main imports (`hostfn_add_override`, before the modules load); the 26 `nn::ldn` ones stay offline.
`runtime/lan_net.c` keeps the guest's sockets (descriptors 3-66):

- lan mode: each guest socket is a non-blocking host UDP socket bound to the adapter's address and the port the game
  asked for, so the game is reachable at the address nifm reported, by another copy or (untested) a console.
  `SO_REUSEADDR` is recorded but not passed to Windows, whose `SO_REUSEADDR` lets a second socket take a bound unicast
  port (FreeBSD's does not). ICMP port-unreachable reports are switched off (`SIO_UDP_CONNRESET`).
- relay mode: guest sockets are virtual; a host thread keeps the relay session and queues arriving datagrams (128 per
  socket). The game's broadcasts are also handed to its own sockets, as a BSD stack does.
- Blocking calls (a receive without `MSG_DONTWAIT`, `Poll` with a timeout, `SubmitNetworkRequestAndWait`, which waits
  up to 10 s for the relay join) give the virtual core up while they wait.
- errno values are Linux's (EAGAIN 11, ENETUNREACH 101, ...), as nn::socket's `Errno` numbers them, while its
  structures and flags are FreeBSD's: the session library ends the session on any errno its table does not know
  (`main+0x92c600`), and FreeBSD's EAGAIN (35) after an empty receive did exactly that. A refused destination fails
  with ENETUNREACH, a
  broadcast without `SO_BROADCAST` with EACCES, a relay datagram over 1440 bytes with EMSGSIZE.

## 5. Two instances

Each instance needs its own profile folder (its own account Uid in `account.uid`, its own save under
`save/<uid>/`), log and shader caches. The simplest save is one `save_profile` makes ([save-profile.md](save-profile.md)):
give each instance its own profile.json (the name there is the player's name in the room) and `KLM_START=lobby-lan`.
With a save that reached the lobby in `<A>\profile`:

```powershell
# instance 1
$env:KLM_LAN = 'lan:127.0.0.2/8'; $env:KLM_NICKNAME = 'Player1'; $env:KLM_NET_LOG = '1'
$env:KLM_WINDOW_TITLE = 'Splatoon 3 - 1'; $env:KLM_SHADER_CACHE = "<A>\shader-cache"
$env:KLM_PIPELINE_CACHE = "<A>\pipeline-cache.bin"
splat3.exe --profile <A>\profile 2> <A>\run.log
# instance 2, another shell: the same with 127.0.0.3, Player2 and <B>
```

The second profile gets its own Uid on its first start; copy the save folder `<A>\profile\save\<uid A>` to
`<B>\profile\save\<uid B>` (the Uid in hex, high half first; `account.uid` holds the low then the high 8 bytes,
little-endian). Across the internet use `KLM_LAN=relay:<relay address>` and the same `KLM_LAN_ROOM` on every
PC; across a real LAN, `KLM_LAN=lan` on each PC (Windows Firewall may ask once to let `splat3.exe` receive on
private networks: allow it there).

## 6. Tests

- `splat3.exe --probe-lan` (no game files, loopback only): `KLM_LAN` parsing; the allowlist of section 1; the relay
  codec against the test vectors of the protocol; the guest ABI end to end in lan mode on 127.0.0.2/8: socket kinds,
  bind conflicts, `sockaddr_in` layout, unicast and broadcast, `SO_BROADCAST`, `Poll`, `SO_RCVTIMEO`, `O_NONBLOCK`,
  `SendMsg`/`RecvMsg`, truncation, errno values, nifm's local and internet requests, refused name lookups.
- `splat3.exe --probe-lan-peer` in two or more processes with `KLM_LAN` set: each acts as the session library does (sockets on
  49152 and 35000, broadcast discovery, unicast round trips) through the guest-facing functions and passes after
  hearing another's broadcast and three round trips. `KLM_PEER_SECONDS` bounds it (default 20);
  `KLM_PEER_HOLD=1` runs the whole time and needs round trips in the last 4 s (to test a relay restarted
  meanwhile: the clients join again at the same addresses).

## 7. What the game did

Measured with two copies on one PC (`KLM_NET_LOG=1`, Splatoon 3 11.2.0):

- The local lobby starts the network in mode 3 (job `InitLan`, `main+0x14e9ae8`); nothing else is opened until the
  Private Battle menu, where the session library binds 49152 and 35000 (`SO_REUSEADDR`, then `SO_BROADCAST`) and asks `csrng` for
  random bytes. The 40000 socket was never opened.
- Discovery: one 325-byte broadcast to the subnet's broadcast address on 35000; the session list shows the host's
  name and "join: OK". Then unicast between the 49152 sockets only.
- Datagram sizes: 45 to 620 bytes in the room, the match and the results (most 125 and 141), so far below relay mode's
  1440. About 15 datagrams a second each way in the room.
- Each copy had its own account Uid; the room and the results show each player's nickname. Whether two copies with
  the same Uid could play together was not tried.
- The session library needs the console's own errno values (it ends the session on FreeBSD's EAGAIN, section 4) and
  the `csrng` service.

Not tried: a relay across the internet (latency), more than two players in a match (sessions of three and four
members ran with spectators: [auto-match.md](auto-match.md)), Salmon Run, and a console on the same LAN.

Automatic room entry and the match result for a launcher (`KLM_MATCH`): [auto-match.md](auto-match.md).
