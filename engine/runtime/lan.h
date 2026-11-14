// LAN play (docs/lan-play.md): the virtual network that answers nn::socket and nn::nifm when SPLAT3_LAN is set. Off by
// default: without SPLAT3_LAN nothing here runs, no networking library is loaded and ov_offline.c answers every call.
//   lan_policy.c    SPLAT3_LAN parsing, the destination allowlist, the relay protocol codec (no OS calls)
//   lan_net.c       guest sockets over host UDP sockets (lan mode) or over the relay (relay mode); the nifm state
//   lan_host_win.c  Winsock and the adapter list, loaded at run time from System32 (the executable imports neither)
//   ov_lan.c        the guest ABI of nn::socket, nnsocket*, nn::nifm; the call log (SPLAT3_NET_LOG); installation
//   lan_probe.c     --probe-lan (unit tests) and --probe-lan-peer (two processes exchanging datagrams)
#pragma once
#include <stddef.h>
#include <stdint.h>

// ---- errno values of the guest. nn::socket's errno (GetLastError, nn::socket::Errno) uses Linux's numbers, while its
// structures and flags are FreeBSD's: the session library's errno-to-result table (main+0x92c600, jump table at main+0x5076090) treats
// 4, 11, 12 and 16 as "try again", 9, 22, 88, 90, 95 and 97 as bad arguments, 100, 102, 107 and 110 as a lost network
// and 104 as a reset, and fails the whole session on any other value: FreeBSD's EAGAIN (35) after a receive that found
// nothing made every LAN session end at once.
enum {
    LE_INTR = 4, LE_BADF = 9, LE_AGAIN = 11, LE_NOMEM = 12, LE_ACCES = 13, LE_FAULT = 14, LE_INVAL = 22, LE_MFILE = 24,
    LE_NOTSOCK = 88, LE_DESTADDRREQ = 89, LE_MSGSIZE = 90, LE_PROTOTYPE = 91, LE_NOPROTOOPT = 92,
    LE_PROTONOSUPPORT = 93, LE_OPNOTSUPP = 95, LE_AFNOSUPPORT = 97, LE_ADDRINUSE = 98, LE_ADDRNOTAVAIL = 99,
    LE_NETDOWN = 100, LE_NETUNREACH = 101, LE_CONNRESET = 104, LE_NOBUFS = 105, LE_ISCONN = 106, LE_NOTCONN = 107,
    LE_TIMEDOUT = 110, LE_CONNREFUSED = 111, LE_HOSTUNREACH = 113
};

// ---- configuration (SPLAT3_LAN, SPLAT3_LAN_ROOM, SPLAT3_LAN_NAME, SPLAT3_LAN_BIND)
typedef enum { LAN_OFF = 0, LAN_LAN = 1, LAN_RELAY = 2 } LanMode;
#define LAN_VNET 0x0A0D0000u          // relay mode's virtual subnet 10.13.0.0/16
#define LAN_VNET_MASK 0xFFFF0000u
#define LAN_RELAY_DEFAULT_PORT 47313
typedef struct {
    LanMode mode;
    uint32_t ip, mask;     // host byte order. lan: the interface (0 until lan_net_start detects one); relay: the
                           // virtual address the relay assigned (0 until joined)
    uint32_t gateway;      // lan: the interface's default gateway, refused as a destination; 0 when there is none
    int detect;            // lan: SPLAT3_LAN=lan, the interface is chosen from the adapter list
    int bind_any;          // lan: SPLAT3_LAN_BIND=any binds host sockets to 0.0.0.0 instead of the interface address
    uint32_t relay_ip;     // relay: the relay's IPv4 address (a literal: nothing is ever resolved), host byte order
    uint16_t relay_port;
    char room[33];         // relay: the room code, upper case
    char name[33];         // relay: a display name for the relay's logs (may be empty)
} LanCfg;

// spec: SPLAT3_LAN ("lan", "lan:<a.b.c.d>/<prefix>", "relay:<a.b.c.d>[:<port>]"; NULL, "", "0" and "off" are off).
// room, name, bind: the other variables (NULL when unset). 1 with *out filled, 0 with the reason in err.
int lan_parse(const char *spec, const char *room, const char *name, const char *bind, LanCfg *out, char *err,
              size_t cap);
// A lan-mode interface address: inside a private block (10/8, 172.16/12, 192.168/16, 169.254/16, 127/8), a prefix
// no shorter than the block's and no longer than /30, a host address. 1 ok, 0 with the reason in err.
int lan_net_ok(uint32_t ip, uint32_t mask, char *err, size_t cap);
int lan_private(uint32_t ip);                       // in one of the blocks above
uint32_t lan_prefix_mask(int prefix);
int lan_mask_prefix(uint32_t mask);                 // -1 when the mask is not contiguous
const char *lan_ip_str(uint32_t ip, char out[16]);  // host byte order -> dotted quad
int lan_ip_parse(const char *s, uint32_t *ip);      // dotted quad -> host byte order; 1 ok

// ---- the allowlist: every datagram the guest sends goes through lan_dst_check, every one it receives through
// lan_src_ok. Destinations: lan mode the interface's subnet and its broadcast (255.255.255.255 becomes the subnet's
// directed broadcast), never the network address, the default gateway, multicast or a service port; relay mode the
// virtual subnet 10.13.0.0/16 only (the host itself only ever sends to the relay). Off: nothing.
enum { LAN_DST_UNICAST = 1, LAN_DST_BROADCAST = 2, LAN_DST_SELF = 3 };
int lan_dst_check(const LanCfg *c, uint32_t dst, uint16_t port, uint32_t *wire, const char **why);
int lan_src_ok(const LanCfg *c, uint32_t src);
int lan_port_blocked(uint16_t port);   // DNS, DHCP, NTP, NetBIOS, SSDP, NAT-PMP/PCP, mDNS, LLMNR: refused in every mode

// ---- relay protocol, version 1 (docs/lan-relay-protocol.md). Big-endian fields; every datagram starts with the
// 8-byte header "S3LR", version, type, reserved u16.
#define LR_VERSION 1
#define LR_MAX_DATAGRAM 1472
#define LR_DATA_HEADER 32
#define LR_MAX_PAYLOAD (LR_MAX_DATAGRAM - LR_DATA_HEADER)   // 1440
#define LR_HELLO_SIZE 64
#define LR_JOIN_MIN 64
enum {
    LR_HELLO = 0x01, LR_CHALLENGE = 0x02, LR_JOIN = 0x03, LR_JOIN_ACK = 0x04, LR_JOIN_NAK = 0x05, LR_KEEPALIVE = 0x06,
    LR_KEEPALIVE_ACK = 0x07, LR_LEAVE = 0x08, LR_CLOSED = 0x09, LR_DATA = 0x10, LR_MEMBER = 0x11
};
enum {   // JOIN_NAK reasons
    LR_NAK_VERSION = 1, LR_NAK_COOKIE = 2, LR_NAK_ROOM = 3, LR_NAK_ROOM_FULL = 4, LR_NAK_RELAY_FULL = 5,
    LR_NAK_NAME = 6, LR_NAK_RATE = 7, LR_NAK_MALFORMED = 8
};
enum {   // JOIN_NAK byte 17, the detail of a ROOM_FULL or RELAY_FULL (0 from a relay that does not say)
    LR_NAK_DETAIL_NONE = 0, LR_NAK_DETAIL_SOURCE = 1   // SOURCE: the limit counts this player's Internet address
};
enum {   // CLOSED reasons
    LR_CLOSED_IDLE = 1, LR_CLOSED_UNKNOWN = 2, LR_CLOSED_KICKED = 3, LR_CLOSED_SHUTDOWN = 4, LR_CLOSED_VIOLATION = 5
};
typedef struct {
    uint8_t version, type;
    uint64_t nonce;                 // HELLO, CHALLENGE, JOIN, JOIN_ACK, JOIN_NAK
    uint8_t cookie[16];             // CHALLENGE, JOIN
    uint32_t vip;                   // JOIN (requested), JOIN_ACK (assigned), MEMBER
    uint8_t prefix, max_members, keepalive_s, idle_s;   // JOIN_ACK
    uint16_t max_payload;           // JOIN_ACK
    uint8_t reason;                 // JOIN_NAK, CLOSED; MEMBER: 1 joined, 2 left
    uint8_t detail;                 // JOIN_NAK: LR_NAK_DETAIL_*
    uint32_t session;               // session messages (JOIN_ACK and every type from KEEPALIVE on)
    uint8_t token[8];
    uint32_t seq;                   // KEEPALIVE, KEEPALIVE_ACK
    uint8_t members;                // KEEPALIVE_ACK
    char room[33], name[33];        // JOIN; MEMBER (name)
    uint32_t src_ip, dst_ip;        // DATA
    uint16_t src_port, dst_port;
    const uint8_t *payload;         // DATA: points into the parsed datagram
    size_t len;
} LrMsg;
// Encoders write one datagram into out (at least LR_MAX_DATAGRAM bytes) and return its size.
size_t lr_hello(uint8_t *out, uint64_t nonce);
size_t lr_join(uint8_t *out, uint64_t nonce, const uint8_t cookie[16], uint32_t want_vip, const char *room,
               const char *name);
size_t lr_keepalive(uint8_t *out, uint32_t session, const uint8_t token[8], uint32_t seq);
size_t lr_leave(uint8_t *out, uint32_t session, const uint8_t token[8]);
size_t lr_data(uint8_t *out, uint32_t session, const uint8_t token[8], uint32_t src_ip, uint16_t src_port,
               uint32_t dst_ip, uint16_t dst_port, const void *payload, size_t len);   // 0 when len > LR_MAX_PAYLOAD
// The relay side (lan_probe.c's tests and the reference relay's format, kept here so both ends share one codec).
size_t lr_challenge(uint8_t *out, uint64_t nonce, const uint8_t cookie[16]);
size_t lr_join_ack(uint8_t *out, uint64_t nonce, uint32_t session, const uint8_t token[8], uint32_t vip,
                   uint8_t prefix, uint8_t max_members, uint8_t keepalive_s, uint8_t idle_s, uint16_t max_payload);
size_t lr_join_nak(uint8_t *out, uint64_t nonce, uint8_t reason, uint8_t detail);
size_t lr_keepalive_ack(uint8_t *out, uint32_t session, const uint8_t token[8], uint32_t seq, uint8_t members);
size_t lr_closed(uint8_t *out, uint32_t session, const uint8_t token[8], uint8_t reason);
size_t lr_member(uint8_t *out, uint32_t session, const uint8_t token[8], uint8_t event, uint32_t vip, const char *name);
// 1 when b holds a well-formed version-1 message of a known type (m filled), 0 otherwise. A header with another version
// fills m->version and m->type and returns 0.
int lr_parse(const uint8_t *b, size_t n, LrMsg *m);

// ---- lan_host_win.c: host UDP sockets. Errors come back as guest errno values.
typedef struct LanHost LanHost;
int lan_host_init(char *err, size_t cap);   // loads ws2_32.dll and iphlpapi.dll, WSAStartup; 1 ok
// The adapter a lan-mode game uses: up, not loopback, an IPv4 address that passes lan_net_ok; one with a default
// gateway first. desc: the adapter's name for the log. 1 found.
int lan_host_detect(uint32_t *ip, uint32_t *mask, uint32_t *gateway, char *desc, size_t cap);
int lan_host_find(uint32_t ip, uint32_t *mask, uint32_t *gateway);   // ip is an address of an up adapter: 1, mask set
LanHost *lan_host_udp(uint32_t ip, uint16_t port, int *err);         // bound, non-blocking, broadcast allowed
uint16_t lan_host_port(LanHost *h);
long lan_host_sendto(LanHost *h, const void *p, size_t n, uint32_t ip, uint16_t port, int *err);
// One datagram: its size, or -1 with LE_AGAIN when none is queued. *trunc 1 when n was too small (n bytes kept).
long lan_host_recvfrom(LanHost *h, void *p, size_t n, uint32_t *ip, uint16_t *port, int *trunc, int *err);
// Waits until one of h[0..n) has a datagram or timeout_ms passes (-1 forever); ready[i] set per socket. The count.
int lan_host_wait(LanHost *const *h, int n, int *ready, int timeout_ms);
int lan_host_setbuf(LanHost *h, int rcv, int bytes);
int lan_host_broadcast(LanHost *h, int on);   // SO_BROADCAST (on by lan_host_udp); 1 ok
void lan_host_close(LanHost *h);

// ---- lan_net.c: the virtual network
int lan_net_start(const LanCfg *cfg, char *err, size_t cap);   // boot (ov_lan.c lan_install); 1 ok
void lan_leave(void);                                          // relay mode: LEAVE the room (the session ends)
const LanCfg *lan_cfg(void);                                   // the running configuration (ip, mask once known)
LanMode lan_mode(void);
// nn::nifm: the request the game submits. Only a local-network request (SetLocalNetworkMode(true)) is accepted: an
// internet request stays unavailable in every mode.
void lan_nifm_local(int local);
int lan_nifm_submit(void);                                     // 1 accepted (relay: waits up to 10 s for the join)
int lan_nifm_available(void);
void lan_nifm_cancel(void);
int lan_nifm_ipconfig(uint32_t *ip, uint32_t *mask);           // host byte order; 1 while a request is accepted
// nn::socket, guest semantics; -1 with *err (guest errno) on failure
typedef struct { int32_t fd; int16_t events, revents; } LanPollFd;   // the guest's struct pollfd
enum { LAN_POLLIN = 1, LAN_POLLPRI = 2, LAN_POLLOUT = 4, LAN_POLLERR = 8, LAN_POLLHUP = 0x10, LAN_POLLNVAL = 0x20,
       LAN_POLLRDNORM = 0x40, LAN_POLLWRNORM = 4 };
int lan_socket(int family, int type, int proto, int *err);
int lan_bind(int fd, uint32_t ip, uint16_t port, int *err);
int lan_connect(int fd, uint32_t ip, uint16_t port, int *err);
long lan_sendto(int fd, const void *p, size_t n, int flags, int has_dst, uint32_t ip, uint16_t port, int *err);
// A datagram refused (its source) or dropped (the observer below) never reaches p; one call drops LAN_DROPS_PER_CALL
// at most before it answers EAGAIN (non-blocking) or waits again (blocking).
#define LAN_DROPS_PER_CALL 64
long lan_recvfrom(int fd, void *p, size_t n, int flags, uint32_t *ip, uint16_t *port, int *trunc, int *err);
int lan_close(int fd, int *err);
int lan_shutdown(int fd, int how, int *err);
int lan_getsockname(int fd, uint32_t *ip, uint16_t *port, int *err);
int lan_getpeername(int fd, uint32_t *ip, uint16_t *port, int *err);
int lan_setsockopt(int fd, int level, int opt, const void *v, uint32_t len, int *err);
int lan_getsockopt(int fd, int level, int opt, void *v, uint32_t *len, int *err);
int lan_fcntl(int fd, int cmd, int arg, int *err);
int lan_poll(LanPollFd *fds, uint64_t n, int timeout_ms, int *err);
int lan_is_socket(int fd);
// Refusals: one log line per destination (the first 64 destinations, then a count), with the caller.
// An observer of the guest's datagram sockets (auto_match.c): a bind and a close with the local port, a received
// datagram with the local port, its source and its bytes, a nifm local-network request accepted or failed (relay mode:
// not joined to the relay within 10 s). A receive observer that returns 0 drops the datagram: the guest never sees it.
// Called outside the table's lock, on the guest's thread.
enum { LAN_OBS_BIND = 1, LAN_OBS_CLOSE = 2, LAN_OBS_RECV = 3, LAN_OBS_NIFM = 4, LAN_OBS_NIFM_FAILED = 5 };
// Relay mode: the name another member of the room joined with (its SPLAT3_LAN_NAME), by its virtual address; 1 found
int lan_relay_member_name(uint32_t vip, char *out, size_t cap);
int lan_roster_selftest(void);   // --probe-lan: lan_relay_member_name on a roster of its own; the failures
// Relay mode: the reason (LR_NAK_*) of the last JOIN_NAK since the last JOIN_ACK, 0 when none; *detail its
// LR_NAK_DETAIL_*. auto_match.c says it in the error it stops with.
int lan_relay_refusal(int *detail);
int lan_relay_nak_selftest(void);   // --probe-lan: JOIN_NAKs kept, and forgotten by a JOIN_ACK; the failures
typedef int (*LanObserver)(int ev, uint16_t lport, uint32_t ip, const void *p, long n);
void lan_set_observer(LanObserver f);

void lan_refused(const char *what, uint32_t ip, uint16_t port, const char *why);
uint64_t lan_refusals(void);
extern _Thread_local uint64_t lan_caller;   // ov_lan.c sets the guest return address of the call it serves

// ---- ov_lan.c (lan_install is in host.h)
struct Cpu;
typedef void (*LanGuestFn)(struct Cpu *);
LanGuestFn lan_guest_fn(const char *name);   // LAN play's implementation of an imported name, NULL when it has none
int lan_guest_errno(void);                   // the calling thread's errno as the guest reads it
