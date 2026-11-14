// LAN play's virtual network (lan.h): the guest's UDP sockets, the nifm request and the relay client.
//
// lan mode: each guest socket is a host UDP socket bound to the interface address (SPLAT3_LAN_BIND=any: 0.0.0.0) and
// the port the guest asked for, so the game is reachable at the address nifm reported and interoperates with any
// other copy on the LAN. relay mode: guest sockets are virtual; every datagram travels inside a relay DATA message on
// one host socket whose only peer is the relay (docs/lan-relay-protocol.md), and the guest sees the virtual address
// the relay assigned in 10.13.0.0/16. Either way lan_dst_check decides every destination before anything is sent, and
// a received datagram reaches the guest only from an address lan_src_ok accepts.
//
// UDP only: Splatoon 3's session library (its LAN module) creates (AF_INET, SOCK_DGRAM, IPPROTO_UDP) sockets alone (docs/lan-play.md),
// so a stream socket fails with ENETDOWN, as every socket does offline.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "host.h"
#include "lan.h"
#include "platform/platform.h"

#define MAX_SOCKS 64
#define FD_BASE 3            // guest descriptors 3..66
#define QUEUE_SLOTS 128      // relay mode: datagrams waiting per socket, the oldest kept when full (UDP drops)
#define WAIT_SLICE_MS 50     // blocking calls wake this often to notice a close

enum { SOCK_DGRAM_ = 2, SOCK_TYPE_MASK = 0xFFFF, SOCK_NONBLOCK_ = 0x20000000, IPPROTO_UDP_ = 17, AF_INET_ = 2 };
enum { MSG_PEEK_ = 2, MSG_DONTWAIT_ = 0x80 };
enum { SOL_SOCKET_ = 0xFFFF, IPPROTO_IP_ = 0 };
enum { SO_REUSEADDR_ = 4, SO_KEEPALIVE_ = 8, SO_DONTROUTE_ = 0x10, SO_BROADCAST_ = 0x20, SO_LINGER_ = 0x80,
       SO_OOBINLINE_ = 0x100, SO_REUSEPORT_ = 0x200, SO_NOSIGPIPE_ = 0x800, SO_SNDBUF_ = 0x1001, SO_RCVBUF_ = 0x1002,
       SO_SNDLOWAT_ = 0x1003, SO_RCVLOWAT_ = 0x1004, SO_SNDTIMEO_ = 0x1005, SO_RCVTIMEO_ = 0x1006, SO_ERROR_ = 0x1007,
       SO_TYPE_ = 0x1008 };
enum { IP_TOS_ = 3, IP_TTL_ = 4, IP_MULTICAST_TTL_ = 10, IP_MULTICAST_LOOP_ = 11 };
enum { F_GETFD_ = 1, F_SETFD_ = 2, F_GETFL_ = 3, F_SETFL_ = 4, O_RDWR_ = 2, O_NONBLOCK_ = 4 };

typedef struct { uint32_t ip; uint16_t port, len; uint8_t data[LR_MAX_PAYLOAD]; } Dgram;
typedef struct {
    int used, users, closing;
    int nonblock, broadcast, reuseaddr, reuseport, ttl;
    int rcvbuf, sndbuf;
    int64_t rcvtimeo_ms, sndtimeo_ms;   // 0: none
    int bound;
    uint32_t bound_ip;                  // as the guest bound it (0: any)
    uint16_t port;
    int connected;
    uint32_t peer_ip;
    uint16_t peer_port;
    int shut_rd, shut_wr;
    LanHost *h;                         // lan mode, once bound
    Dgram *q;                           // relay mode, once bound
    int qhead, qcount;
    uint64_t dropped;
} VSock;

static VSock socks[MAX_SOCKS];
static PlatMutex lock = PLAT_MUTEX_INIT;   // socks, cfg.ip/mask, the relay state, the nifm state
static PlatCond changed = PLAT_COND_INIT;  // a datagram was queued, the relay state or a socket changed
static LanCfg cfg;
static int started;
_Thread_local uint64_t lan_caller;         // ov_lan.c: the guest return address of the call in progress

const LanCfg *lan_cfg(void) { return &cfg; }
LanMode lan_mode(void) { return started ? cfg.mode : LAN_OFF; }

// ---- refusals
static struct { uint32_t ip; uint16_t port; } refused_seen[64];
static int nrefused_seen;
static volatile int64_t refused_total;

void lan_refused(const char *what, uint32_t ip, uint16_t port, const char *why) {
    plat_inc64(&refused_total);
    plat_mutex_lock(&lock);
    int seen = 0;
    for (int i = 0; i < nrefused_seen && !seen; i++) seen = refused_seen[i].ip == ip && refused_seen[i].port == port;
    int log = !seen && nrefused_seen < 64;
    if (log) refused_seen[nrefused_seen++] = (typeof(refused_seen[0])){ip, port};
    plat_mutex_unlock(&lock);
    if (!log) return;
    char a[16], lr[96] = "host";
    if (lan_caller) ldr_describe(lan_caller - 4, lr, sizeof lr);
    rt_log("lan: REFUSED %s %s:%u (%s) from %s", what, lan_ip_str(ip, a), port, why, lr);
}

uint64_t lan_refusals(void) { return (uint64_t)refused_total; }

// ---- sockets
static VSock *acquire(int fd, int *err) {
    int i = fd - FD_BASE;
    plat_mutex_lock(&lock);
    if (i < 0 || i >= MAX_SOCKS || !socks[i].used || socks[i].closing) {
        plat_mutex_unlock(&lock);
        *err = LE_BADF;
        return NULL;
    }
    socks[i].users++;
    plat_mutex_unlock(&lock);
    return &socks[i];
}

// Holding lock: frees what a closed socket held once no call uses it.
static void reap(VSock *s) {
    if (!s->closing || s->users) return;
    lan_host_close(s->h);
    free(s->q);
    memset(s, 0, sizeof *s);
}

static void release(VSock *s) {
    plat_mutex_lock(&lock);
    s->users--;
    reap(s);
    plat_mutex_unlock(&lock);
}

int lan_is_socket(int fd) {
    int i = fd - FD_BASE;
    return i >= 0 && i < MAX_SOCKS && socks[i].used && !socks[i].closing;
}

// A socket kind LAN play does not open: logged for the first 16 calls.
static void refuse_kind(int family, int type, int proto, const char *why) {
    static volatile int32_t told;
    if (plat_inc32(&told) > 16) return;
    char lr[96] = "host";
    if (lan_caller) ldr_describe(lan_caller - 4, lr, sizeof lr);
    rt_log("lan: REFUSED a socket (family %d, type %#x, protocol %d): %s, from %s", family, type, proto, why, lr);
}

int lan_socket(int family, int type, int proto, int *err) {
    if (family != AF_INET_) {
        refuse_kind(family, type, proto, "IPv4 only");
        return *err = LE_AFNOSUPPORT, -1;
    }
    if ((type & SOCK_TYPE_MASK) != SOCK_DGRAM_) {
        refuse_kind(family, type, proto, "UDP only, LAN play opens no TCP or raw socket");
        return *err = LE_NETDOWN, -1;
    }
    if (proto != 0 && proto != IPPROTO_UDP_) return *err = LE_PROTONOSUPPORT, -1;
    plat_mutex_lock(&lock);
    int i = 0;
    while (i < MAX_SOCKS && socks[i].used) i++;
    if (i == MAX_SOCKS) {
        plat_mutex_unlock(&lock);
        return *err = LE_MFILE, -1;
    }
    socks[i] = (VSock){.used = 1, .nonblock = (type & SOCK_NONBLOCK_) != 0, .ttl = 64};
    plat_mutex_unlock(&lock);
    return FD_BASE + i;
}

// Holding lock, relay mode: a socket already bound to port.
static int port_taken(uint16_t port) {
    for (int i = 0; i < MAX_SOCKS; i++)
        if (socks[i].used && !socks[i].closing && socks[i].bound && socks[i].port == port) return 1;
    return 0;
}

// A closed datagram socket bound to port still waits for a call to return before its host socket closes (reap)
static int closing_holds(uint16_t port) {
    plat_mutex_lock(&lock);
    int held = 0;
    for (int i = 0; i < MAX_SOCKS && !held; i++)
        held = socks[i].used && socks[i].closing && socks[i].bound && socks[i].port == port;
    plat_mutex_unlock(&lock);
    return held;
}

// Binds s (users held). ip, port: the guest's; port 0 picks one.
static int do_bind(VSock *s, uint32_t ip, uint16_t port, int *err) {
    if (s->bound) return *err = LE_INVAL, -1;
    if (ip && ip != cfg.ip) return *err = LE_ADDRNOTAVAIL, -1;
    if (cfg.mode == LAN_LAN) {
        LanHost *h = lan_host_udp(cfg.bind_any ? 0 : cfg.ip, port, err);
        // A socket the guest closed while another of its threads was still inside a call on it keeps its host socket
        // until that call returns (reap), where BSD frees the port at the close. The session library closes its two sockets and binds
        // the same ports again at once when a session is created or joined, while its receive thread polls them: wait
        // for that call to return (it does not block: MSG_DONTWAIT) and bind again.
        for (int tries = 0; !h && *err == LE_ADDRINUSE && port && tries < 50 && closing_holds(port); tries++) {
            int away = kern_core_away();
            plat_sleep_ms(10);
            kern_core_back(&away);
            h = lan_host_udp(cfg.bind_any ? 0 : cfg.ip, port, err);
        }
        if (!h) return -1;
        if (s->rcvbuf) lan_host_setbuf(h, 1, s->rcvbuf);
        if (s->sndbuf) lan_host_setbuf(h, 0, s->sndbuf);
        plat_mutex_lock(&lock);
        s->h = h;
        s->port = lan_host_port(h);
        s->bound_ip = ip;
        s->bound = 1;
        plat_mutex_unlock(&lock);
        return 0;
    }
    Dgram *q = malloc(sizeof(Dgram) * QUEUE_SLOTS);
    if (!q) return *err = LE_NOBUFS, -1;
    plat_mutex_lock(&lock);
    if (!port) {   // the ephemeral range of FreeBSD and Windows
        static uint16_t next = 49152;
        for (int tries = 0; tries < 16384 && !port; tries++) {
            uint16_t p = next;
            next = next == 65535 ? 49152 : next + 1;
            if (!port_taken(p)) port = p;
        }
    }
    if (!port || port_taken(port)) {
        plat_mutex_unlock(&lock);
        free(q);
        return *err = port ? LE_ADDRINUSE : LE_ADDRNOTAVAIL, -1;
    }
    s->q = q;
    s->qhead = s->qcount = 0;
    s->port = port;
    s->bound_ip = ip;
    s->bound = 1;
    plat_mutex_unlock(&lock);
    return 0;
}

static LanObserver observer;

void lan_set_observer(LanObserver f) { __atomic_store_n(&observer, f, __ATOMIC_RELEASE); }

static int observe(int ev, uint16_t lport, uint32_t ip, const void *p, long n) {
    LanObserver f = __atomic_load_n(&observer, __ATOMIC_ACQUIRE);
    return !f || f(ev, lport, ip, p, n);
}

int lan_bind(int fd, uint32_t ip, uint16_t port, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    int r = do_bind(s, ip, port, err);
    uint16_t bound = s->port;
    release(s);
    if (!r) observe(LAN_OBS_BIND, bound, 0, NULL, 0);
    return r;
}

int lan_connect(int fd, uint32_t ip, uint16_t port, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    uint32_t wire;
    const char *why;
    plat_mutex_lock(&lock);
    LanCfg c = cfg;
    plat_mutex_unlock(&lock);
    int r = 0;
    if (!lan_dst_check(&c, ip, port, &wire, &why)) {
        lan_refused("connect to", ip, port, why);
        *err = LE_NETUNREACH, r = -1;
    } else {
        s->connected = 1, s->peer_ip = ip, s->peer_port = port;
    }
    release(s);
    return r;
}

// Relay mode, holding lock: queues a datagram for the socket bound to port (the guest's own address or any).
static void deliver(uint16_t port, uint32_t src, uint16_t sport, const void *p, size_t n) {
    for (int i = 0; i < MAX_SOCKS; i++) {
        VSock *s = &socks[i];
        if (!s->used || s->closing || !s->bound || s->port != port || !s->q || s->shut_rd) continue;
        if (s->qcount == QUEUE_SLOTS) {
            s->dropped++;
            return;
        }
        Dgram *d = &s->q[(s->qhead + s->qcount++) % QUEUE_SLOTS];
        d->ip = src, d->port = sport, d->len = (uint16_t)n;
        memcpy(d->data, p, n);
        plat_cond_broadcast(&changed);
        return;
    }
}

static void relay_send_data(uint16_t sport, uint32_t dst, uint16_t dport, const void *p, size_t n);

long lan_sendto(int fd, const void *p, size_t n, int flags, int has_dst, uint32_t ip, uint16_t port, int *err) {
    (void)flags;
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    long r = -1;
    if (s->shut_wr) {
        *err = LE_INVAL;   // EPIPE on a stream socket; a datagram socket reports it the same way FreeBSD does not
        goto out;
    }
    if (!has_dst) {
        if (!s->connected) {
            *err = LE_DESTADDRREQ;
            goto out;
        }
        ip = s->peer_ip, port = s->peer_port;
    }
    plat_mutex_lock(&lock);
    LanCfg c = cfg;
    plat_mutex_unlock(&lock);
    uint32_t wire;
    const char *why;
    int kind = lan_dst_check(&c, ip, port, &wire, &why);
    if (!kind) {
        lan_refused("send to", ip, port, why);
        *err = LE_NETUNREACH;
        goto out;
    }
    if (kind == LAN_DST_BROADCAST && !s->broadcast) {
        *err = LE_ACCES;   // BSD: a broadcast needs SO_BROADCAST
        goto out;
    }
    if (n > (c.mode == LAN_RELAY ? (size_t)LR_MAX_PAYLOAD : (size_t)65507)) {
        static volatile int32_t told;
        if (!plat_xchg32(&told, 1)) rt_log("lan: a %zu-byte datagram is over the %s limit (EMSGSIZE)", n,
                                          c.mode == LAN_RELAY ? "relay's 1440-byte" : "UDP");
        *err = LE_MSGSIZE;
        goto out;
    }
    if (!s->bound && do_bind(s, 0, 0, err) < 0) goto out;
    if (c.mode == LAN_LAN) {
        mem_commit_guest((uintptr_t)p, n);   // the kernel's read fails on a heap page not committed yet
        r = lan_host_sendto(s->h, p, n, wire, port, err);
        goto out;
    }
    if (kind != LAN_DST_SELF) relay_send_data(s->port, wire, port, p, n);
    if (kind != LAN_DST_UNICAST) {   // the stack hands the host its own broadcasts too, as FreeBSD does
        plat_mutex_lock(&lock);
        deliver(port, c.ip, s->port, p, n);
        plat_mutex_unlock(&lock);
    }
    r = (long)n;
out:
    release(s);
    return r;
}

// Waits up to ms for s to have a datagram (lan: host poll; relay: the queue). The virtual core is given up meanwhile.
static void wait_readable(VSock *s, int ms) {
    int away = kern_core_away();
    if (cfg.mode == LAN_LAN) {
        int ready = 0;
        if (s->h) lan_host_wait((LanHost *const *)&s->h, 1, &ready, ms);
        else plat_sleep_ms((uint32_t)ms);
    } else {
        plat_mutex_lock(&lock);
        if (!s->qcount && !s->closing) plat_cond_wait_ms(&changed, &lock, (uint32_t)ms);
        plat_mutex_unlock(&lock);
    }
    kern_core_back(&away);
}

// A datagram is received into a buffer of this call's (lan: the host socket's next one; relay: the queue's head),
// checked (lan: its source; both: the receive observer, auto_match.c), and only then copied into the guest's buffer:
// a refused datagram never touches it. One call drops LAN_DROPS_PER_CALL datagrams at most before it answers EAGAIN
// (non-blocking) or waits as for an empty socket (blocking): a flood of refused datagrams cannot keep one call busy.
long lan_recvfrom(int fd, void *p, size_t n, int flags, uint32_t *ip, uint16_t *port, int *trunc, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    long r = -1;
    *trunc = 0;
    if (flags & MSG_PEEK_) {
        static volatile int32_t told;
        if (!plat_xchg32(&told, 1)) rt_log("lan: recv with MSG_PEEK is not supported (EOPNOTSUPP)");
        *err = LE_OPNOTSUPP;
        goto out;
    }
    int dontwait = s->nonblock || (flags & MSG_DONTWAIT_), drops = 0;
    uint8_t b[65536];   // a UDP datagram's largest payload (65507) fits: a guest buffer as large never sees a cut
    size_t cap = n < sizeof b ? n : sizeof b;
    uint64_t t0 = plat_now_ms();
    for (;;) {
        if (s->closing) {
            *err = LE_BADF;
            goto out;
        }
        if (s->shut_rd) {
            r = 0;
            goto out;
        }
        if (drops >= LAN_DROPS_PER_CALL) {
            drops = 0;
        } else if (cfg.mode == LAN_LAN) {
            if (s->h) {
                r = lan_host_recvfrom(s->h, b, cap, ip, port, trunc, err);
                if (r >= 0) {
                    plat_mutex_lock(&lock);
                    int ok = lan_src_ok(&cfg, *ip);
                    plat_mutex_unlock(&lock);
                    if (!ok) lan_refused("a datagram from", *ip, *port, "outside the LAN subnet");
                    if (ok && observe(LAN_OBS_RECV, s->port, *ip, b, r)) {
                        mem_commit_guest((uintptr_t)p, (size_t)r);
                        memcpy(p, b, (size_t)r);
                        goto out;
                    }
                    drops++;
                    continue;
                }
                if (*err != LE_AGAIN) goto out;
            }
        } else {
            plat_mutex_lock(&lock);
            if (s->qcount) {
                Dgram *d = &s->q[s->qhead];
                size_t k = d->len < cap ? d->len : cap;
                memcpy(b, d->data, k);
                *ip = d->ip, *port = d->port, *trunc = d->len > n;
                s->qhead = (s->qhead + 1) % QUEUE_SLOTS;
                s->qcount--;
                plat_mutex_unlock(&lock);
                r = (long)k;
                if (observe(LAN_OBS_RECV, s->port, *ip, b, r)) {
                    memcpy(p, b, k);
                    goto out;
                }
                drops++;
                continue;
            }
            plat_mutex_unlock(&lock);
        }
        r = -1;
        int64_t left = s->rcvtimeo_ms ? s->rcvtimeo_ms - (int64_t)(plat_now_ms() - t0) : WAIT_SLICE_MS;
        if (dontwait || left <= 0) {
            *err = LE_AGAIN;
            goto out;
        }
        wait_readable(s, left < WAIT_SLICE_MS ? (int)left : WAIT_SLICE_MS);
    }
out:
    release(s);
    return r;
}

int lan_close(int fd, int *err) {
    int i = fd - FD_BASE;
    plat_mutex_lock(&lock);
    if (i < 0 || i >= MAX_SOCKS || !socks[i].used || socks[i].closing) {
        plat_mutex_unlock(&lock);
        return *err = LE_BADF, -1;
    }
    socks[i].closing = 1;
    uint16_t port = socks[i].bound ? socks[i].port : 0;
    reap(&socks[i]);
    plat_cond_broadcast(&changed);
    plat_mutex_unlock(&lock);
    if (port) observe(LAN_OBS_CLOSE, port, 0, NULL, 0);
    return 0;
}

int lan_shutdown(int fd, int how, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    int r = 0;
    if (how < 0 || how > 2) *err = LE_INVAL, r = -1;
    else if (!s->connected) *err = LE_NOTCONN, r = -1;
    else {
        if (how != 1) s->shut_rd = 1;
        if (how != 0) s->shut_wr = 1;
    }
    release(s);
    return r;
}

int lan_getsockname(int fd, uint32_t *ip, uint16_t *port, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    *ip = s->bound ? s->bound_ip : 0;
    *port = s->bound ? s->port : 0;
    release(s);
    return 0;
}

int lan_getpeername(int fd, uint32_t *ip, uint16_t *port, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    int r = s->connected ? 0 : -1;
    if (r) *err = LE_NOTCONN;
    else *ip = s->peer_ip, *port = s->peer_port;
    release(s);
    return r;
}

static int64_t timeval_ms(const void *v, uint32_t len) {
    int64_t tv[2] = {0, 0};   // FreeBSD aarch64 struct timeval: tv_sec, tv_usec as 64-bit
    memcpy(tv, v, len < sizeof tv ? len : sizeof tv);
    return tv[0] * 1000 + tv[1] / 1000;
}

int lan_setsockopt(int fd, int level, int opt, const void *v, uint32_t len, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    int r = 0, val = 0;
    if (v && len >= 4) memcpy(&val, v, 4);
    else if (v && len == 1) val = *(const uint8_t *)v;
    if (!v && len) *err = LE_FAULT, r = -1;
    else if (level == SOL_SOCKET_) {
        switch (opt) {
        case SO_REUSEADDR_: s->reuseaddr = val != 0; break;   // recorded; host sockets never share a port (lan_host_win.c)
        case SO_REUSEPORT_: s->reuseport = val != 0; break;
        case SO_BROADCAST_: s->broadcast = val != 0; break;
        case SO_SNDBUF_:
        case SO_RCVBUF_:
            if (val <= 0) { *err = LE_INVAL, r = -1; break; }
            *(opt == SO_RCVBUF_ ? &s->rcvbuf : &s->sndbuf) = val;
            if (s->h) lan_host_setbuf(s->h, opt == SO_RCVBUF_, val);
            break;
        case SO_RCVTIMEO_: s->rcvtimeo_ms = timeval_ms(v, len); break;
        case SO_SNDTIMEO_: s->sndtimeo_ms = timeval_ms(v, len); break;
        case SO_KEEPALIVE_: case SO_DONTROUTE_: case SO_LINGER_: case SO_OOBINLINE_: case SO_NOSIGPIPE_:
        case SO_SNDLOWAT_: case SO_RCVLOWAT_:
            break;   // nothing a datagram socket here acts on
        default: *err = LE_NOPROTOOPT, r = -1; break;
        }
    } else if (level == IPPROTO_IP_) {
        switch (opt) {
        case IP_TTL_: s->ttl = val; break;   // recorded: one hop is all a LAN or the relay needs
        case IP_TOS_: case IP_MULTICAST_TTL_: case IP_MULTICAST_LOOP_: break;
        default: *err = LE_NOPROTOOPT, r = -1; break;   // multicast membership among them: LAN play uses broadcast
        }
    } else {
        *err = LE_NOPROTOOPT, r = -1;
    }
    if (r && *err == LE_NOPROTOOPT) {
        static volatile int32_t told;
        if (plat_inc32(&told) <= 16) rt_log("lan: setsockopt level %#x option %#x is not supported (ENOPROTOOPT)", level, opt);
    }
    release(s);
    return r;
}

int lan_getsockopt(int fd, int level, int opt, void *v, uint32_t *len, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    int r = 0, val = 0;
    if (!v || !len || *len < 4) *err = LE_INVAL, r = -1;
    else if (level == SOL_SOCKET_) {
        switch (opt) {
        case SO_ERROR_: val = 0; break;
        case SO_TYPE_: val = SOCK_DGRAM_; break;
        case SO_REUSEADDR_: val = s->reuseaddr ? SO_REUSEADDR_ : 0; break;
        case SO_REUSEPORT_: val = s->reuseport ? SO_REUSEPORT_ : 0; break;
        case SO_BROADCAST_: val = s->broadcast ? SO_BROADCAST_ : 0; break;
        case SO_RCVBUF_: val = s->rcvbuf ? s->rcvbuf : 0x10000; break;
        case SO_SNDBUF_: val = s->sndbuf ? s->sndbuf : 0x10000; break;
        default: *err = LE_NOPROTOOPT, r = -1; break;
        }
    } else if (level == IPPROTO_IP_ && opt == IP_TTL_) {
        val = s->ttl;
    } else {
        *err = LE_NOPROTOOPT, r = -1;
    }
    if (!r) {
        memcpy(v, &val, 4);
        *len = 4;
    }
    release(s);
    return r;
}

int lan_fcntl(int fd, int cmd, int arg, int *err) {
    VSock *s = acquire(fd, err);
    if (!s) return -1;
    int r = 0;
    switch (cmd) {
    case F_GETFL_: r = O_RDWR_ | (s->nonblock ? O_NONBLOCK_ : 0); break;
    case F_SETFL_: s->nonblock = (arg & O_NONBLOCK_) != 0; break;
    case F_GETFD_: case F_SETFD_: break;
    default: *err = LE_INVAL, r = -1; break;
    }
    release(s);
    return r;
}

int lan_poll(LanPollFd *fds, uint64_t n, int timeout_ms, int *err) {
    if (n > 64) return *err = LE_INVAL, -1;
    uint64_t t0 = plat_now_ms();
    for (;;) {
        VSock *held[64];
        LanHost *hs[64];
        int which[64], nh = 0, nheld = 0, ready[64], count = 0;
        plat_mutex_lock(&lock);
        for (uint64_t i = 0; i < n; i++) {
            LanPollFd *f = &fds[i];
            f->revents = 0;
            if (f->fd < 0) continue;
            int k = f->fd - FD_BASE;
            VSock *s = k >= 0 && k < MAX_SOCKS && socks[k].used && !socks[k].closing ? &socks[k] : NULL;
            if (!s) {
                f->revents = LAN_POLLNVAL;
                continue;
            }
            s->users++;   // the host socket stays open while the wait below lists it
            held[nheld++] = s;
            if (f->events & (LAN_POLLOUT | LAN_POLLWRNORM)) f->revents |= f->events & (LAN_POLLOUT | LAN_POLLWRNORM);
            if (f->events & (LAN_POLLIN | LAN_POLLRDNORM)) {
                if (cfg.mode == LAN_RELAY && s->qcount) f->revents |= f->events & (LAN_POLLIN | LAN_POLLRDNORM);
                if (cfg.mode == LAN_LAN && s->h) hs[nh] = s->h, which[nh++] = (int)i;
            }
        }
        plat_mutex_unlock(&lock);
        if (nh) {
            lan_host_wait(hs, nh, ready, 0);
            for (int j = 0; j < nh; j++)
                if (ready[j]) fds[which[j]].revents |= fds[which[j]].events & (LAN_POLLIN | LAN_POLLRDNORM);
        }
        for (uint64_t i = 0; i < n; i++) count += fds[i].revents != 0;
        int64_t left = timeout_ms < 0 ? WAIT_SLICE_MS : timeout_ms - (int64_t)(plat_now_ms() - t0);
        if (!count && left > 0) {
            int slice = left < WAIT_SLICE_MS ? (int)left : WAIT_SLICE_MS;
            int away = kern_core_away();
            if (nh) {
                lan_host_wait(hs, nh, ready, slice);
            } else {
                plat_mutex_lock(&lock);
                plat_cond_wait_ms(&changed, &lock, (uint32_t)slice);
                plat_mutex_unlock(&lock);
            }
            kern_core_back(&away);
        }
        for (int j = 0; j < nheld; j++) release(held[j]);
        if (count || left <= 0) return count;
    }
}

// ---- relay client (docs/lan-relay-protocol.md, "Client")
enum { R_HELLO, R_JOIN, R_JOINED, R_STOPPED };
static struct {
    LanHost *up;
    int state, tries;
    uint64_t nonce;
    uint8_t cookie[16];
    uint32_t session, want_vip;
    uint8_t token[8];
    uint16_t max_payload;
    uint32_t keepalive_ms, silent_ms, seq;
    uint64_t last_tx, last_rx, next_try;
    uint64_t foreign;   // datagrams from another address than the relay's, dropped
    int refusal, refusal_detail;   // the last JOIN_NAK since the last JOIN_ACK (lan_relay_refusal)
} R;

static const char *nak_reason(int r) {
    static const char *const s[] = {"?", "protocol version", "expired cookie", "invalid room code", "room full",
                                    "relay full", "invalid name", "rate limited", "malformed join"};
    return r > 0 && r < (int)(sizeof s / sizeof *s) ? s[r] : "unknown reason";
}

static void relay_tx(const uint8_t *b, size_t n) {
    int err;
    if (lan_host_sendto(R.up, b, n, cfg.relay_ip, cfg.relay_port, &err) < 0) {
        static volatile int32_t told;
        if (plat_inc32(&told) <= 8) rt_log("lan: relay send failed (errno %d)", err);
    }
}

static void relay_send_data(uint16_t sport, uint32_t dst, uint16_t dport, const void *p, size_t n) {
    uint8_t b[LR_MAX_DATAGRAM];
    plat_mutex_lock(&lock);
    int joined = R.state == R_JOINED;
    uint32_t session = R.session, src = cfg.ip;
    uint8_t token[8];
    memcpy(token, R.token, 8);
    plat_mutex_unlock(&lock);
    if (!joined) return;
    size_t k = lr_data(b, session, token, src, sport, dst, dport, p, n);
    if (k) relay_tx(b, k);
}

// Holding lock: a new handshake (new nonce), its first HELLO after delay ms. backoff: HELLOs every 5 s from the start.
static void relay_restart(uint64_t now, uint64_t delay, int backoff) {
    R.state = R_HELLO;
    R.tries = backoff ? 5 : 0;
    if (!plat_random(&R.nonce, sizeof R.nonce)) R.nonce = now * 0x9E3779B97F4A7C15ull;
    R.next_try = now + delay;
}

static void relay_timers(uint64_t now) {
    uint8_t b[LR_MAX_DATAGRAM];
    plat_mutex_lock(&lock);
    switch (R.state) {
    case R_HELLO:   // every 1 s, after 5 unanswered every 5 s
        if (now >= R.next_try) {
            if (R.tries == 5) rt_log("lan: the relay does not answer; still trying every 5 s");
            relay_tx(b, lr_hello(b, R.nonce));
            R.last_tx = now;
            R.tries++;
            R.next_try = now + (R.tries < 5 ? 1000 : 5000);
        }
        break;
    case R_JOIN:   // every 1 s, after 5 unanswered a new handshake
        if (now >= R.next_try) {
            if (R.tries >= 5) {
                relay_restart(now, 0, 0);
                break;
            }
            relay_tx(b, lr_join(b, R.nonce, R.cookie, R.want_vip, cfg.room, cfg.name));
            R.last_tx = now, R.tries++, R.next_try = now + 1000;
        }
        break;
    case R_JOINED:
        if (now - R.last_rx > R.silent_ms) {
            rt_log("lan: nothing from the relay for %u s; joining again", R.silent_ms / 1000);
            relay_restart(now, 0, 0);
        } else if (now - R.last_tx >= R.keepalive_ms) {
            relay_tx(b, lr_keepalive(b, R.session, R.token, ++R.seq));
            R.last_tx = now;
        }
        break;
    }
    plat_mutex_unlock(&lock);
}

static void roster_note(int joined, uint32_t vip, const char *name);

static void relay_receive(const uint8_t *b, size_t n, uint64_t now) {
    LrMsg m;
    uint8_t out[LR_MAX_DATAGRAM];
    char a[16];
    if (!lr_parse(b, n, &m)) {
        // A JOIN_NAK of another version answers our HELLO: the relay speaks another protocol version.
        if (n >= 24 && !memcmp(b, "S3LR", 4) && b[4] != LR_VERSION && b[5] == LR_JOIN_NAK) {
            uint64_t nonce = 0;
            for (int i = 8; i < 16; i++) nonce = nonce << 8 | b[i];
            plat_mutex_lock(&lock);
            if (R.state == R_HELLO && nonce == R.nonce) {
                rt_log("lan: the relay speaks protocol version %u, this build %u; trying again in 60 s", b[4], LR_VERSION);
                relay_restart(now, 60000, 1);
            }
            plat_mutex_unlock(&lock);
        }
        return;
    }
    plat_mutex_lock(&lock);
    int ours = m.session && m.session == R.session && !memcmp(m.token, R.token, 8);
    switch (m.type) {
    case LR_CHALLENGE:
        if (R.state != R_HELLO || m.nonce != R.nonce) break;
        memcpy(R.cookie, m.cookie, 16);
        R.state = R_JOIN, R.tries = 1, R.next_try = now + 1000, R.last_tx = now;
        relay_tx(out, lr_join(out, R.nonce, R.cookie, R.want_vip, cfg.room, cfg.name));
        break;
    case LR_JOIN_ACK:
        if (R.state != R_JOIN || m.nonce != R.nonce) break;
        if ((m.vip & LAN_VNET_MASK) != LAN_VNET || m.prefix != 16 || (m.vip & 0xFFFF) == 0 || (m.vip & 0xFFFF) == 0xFFFF) {
            rt_log("lan: the relay assigned %s/%u, outside 10.13.0.0/16; not joining", lan_ip_str(m.vip, a), m.prefix);
            relay_restart(now, 30000, 1);
            break;
        }
        if (cfg.ip && cfg.ip != m.vip)
            rt_log("lan: rejoined as %s instead of %s: a game session in progress loses this player", lan_ip_str(m.vip, a),
                   lan_ip_str(cfg.ip, (char[16]){0}));
        R.session = m.session;
        memcpy(R.token, m.token, 8);
        R.want_vip = cfg.ip = m.vip;
        cfg.mask = LAN_VNET_MASK;
        R.max_payload = m.max_payload;
        R.keepalive_ms = (m.keepalive_s ? m.keepalive_s : 5) * 1000u;
        R.silent_ms = 3 * R.keepalive_ms;
        R.state = R_JOINED, R.last_rx = now;
        R.refusal = R.refusal_detail = 0;
        rt_log("lan: joined relay room %s as %s (session %08x, keepalive %u s, %u members at most)", cfg.room,
               lan_ip_str(m.vip, a), m.session, R.keepalive_ms / 1000, m.max_members);
        plat_cond_broadcast(&changed);
        break;
    case LR_JOIN_NAK:
        if (R.state == R_JOINED || m.nonce != R.nonce) break;
        rt_log("lan: the relay refused the join: %s%s", nak_reason(m.reason),
               m.detail == LR_NAK_DETAIL_SOURCE ? " (too many members from this player's Internet address)" : "");
        R.refusal = m.reason, R.refusal_detail = m.detail;
        if (m.reason == LR_NAK_COOKIE) relay_restart(now, 0, 0);
        else relay_restart(now, m.reason == LR_NAK_VERSION ? 60000 : 10000, 1);
        break;
    case LR_KEEPALIVE_ACK:
        if (ours) R.last_rx = now;
        break;
    case LR_CLOSED:
        if (!ours) break;
        rt_log("lan: the relay closed the session (reason %u); joining again", m.reason);
        R.session = 0;
        relay_restart(now, 0, 0);
        break;
    case LR_MEMBER:
        if (!ours) break;
        R.last_rx = now;
        rt_log("lan: relay room %s: %s %s%s%s", cfg.room, lan_ip_str(m.vip, a), m.reason == 1 ? "joined" : "left",
               m.name[0] ? " as " : "", m.name);
        roster_note(m.reason == 1, m.vip, m.name);
        break;
    case LR_DATA:
        if (!ours || R.state != R_JOINED) break;
        R.last_rx = now;
        if (m.len > LR_MAX_PAYLOAD) break;
        if (!lan_src_ok(&cfg, m.src_ip) || m.src_ip == cfg.ip) {
            plat_mutex_unlock(&lock);
            lan_refused("relayed datagram from", m.src_ip, m.src_port, "outside the virtual subnet");
            return;
        }
        if (m.dst_ip != cfg.ip && m.dst_ip != (cfg.ip | ~cfg.mask) && m.dst_ip != 0xFFFFFFFFu) break;
        deliver(m.dst_port, m.src_ip, m.src_port, m.payload, m.len);
        break;
    }
    plat_mutex_unlock(&lock);
}

// The relay room's other members, by the MEMBER messages (lock held): their virtual address and the name they joined
// with (SPLAT3_LAN_NAME); lan_relay_member_name reads it (auto_match.c's teams by name)
static struct { uint32_t vip; char name[33]; } roster[32];

static void roster_note(int joined, uint32_t vip, const char *name) {
    int free_slot = -1;
    for (int i = 0; i < 32; i++) {
        if (roster[i].vip == vip) roster[i].vip = 0;
        if (!roster[i].vip && free_slot < 0) free_slot = i;
    }
    if (joined && free_slot >= 0) {
        roster[free_slot].vip = vip;
        snprintf(roster[free_slot].name, sizeof roster[0].name, "%s", name);
    }
}

// The relay vouches for no name (a member joins with any): a name that two members of the room hold, or this player's
// own, names nobody, so teams by name seat neither member by it (the relay or the server
// must keep names unique for the first member with a name to be the right one).
int lan_relay_member_name(uint32_t vip, char *out, size_t cap) {
    int found = -1, same = 0;
    plat_mutex_lock(&lock);
    for (int i = 0; i < 32 && found < 0; i++)
        if (vip && roster[i].vip == vip) found = i;
    if (found >= 0) {
        same = roster[found].name[0] && !strcmp(roster[found].name, cfg.name);
        for (int i = 0; i < 32; i++)
            same += i != found && roster[i].vip && !strcmp(roster[i].name, roster[found].name);
        if (!same) snprintf(out, cap, "%s", roster[found].name);
    }
    plat_mutex_unlock(&lock);
    if (same) {
        static volatile int32_t told;
        if (plat_inc32(&told) <= 4) rt_log("lan: relay room %s: a member's name is held twice (or is this player's): it names nobody", cfg.room);
    }
    return found >= 0 && !same;
}

int lan_relay_refusal(int *detail) {
    plat_mutex_lock(&lock);
    int r = R.refusal;
    if (detail) *detail = R.refusal_detail;
    plat_mutex_unlock(&lock);
    return r;
}

// --probe-lan: the relay client's handling of JOIN_NAK and JOIN_ACK, fed to relay_receive as if from the relay (no
// relay, no socket: nothing is sent on these paths). 0 failures expected.
int lan_relay_nak_selftest(void) {
    int fails = 0, detail = -1;
    uint8_t b[LR_MAX_DATAGRAM], token[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    plat_mutex_lock(&lock);
    int state0 = R.state;
    uint64_t nonce0 = R.nonce;
    uint32_t ip0 = cfg.ip, mask0 = cfg.mask, want0 = R.want_vip;
    R.state = R_JOIN, R.nonce = 77;
    plat_mutex_unlock(&lock);
    fails += lan_relay_refusal(&detail) != 0;
    relay_receive(b, lr_join_nak(b, 77, LR_NAK_ROOM_FULL, LR_NAK_DETAIL_SOURCE), 1000);
    fails += !(lan_relay_refusal(&detail) == LR_NAK_ROOM_FULL && detail == LR_NAK_DETAIL_SOURCE);
    plat_mutex_lock(&lock);
    R.state = R_JOIN, R.nonce = 78;   // the next attempt (relay_restart drew a new nonce)
    plat_mutex_unlock(&lock);
    relay_receive(b, lr_join_nak(b, 77, LR_NAK_RELAY_FULL, 0), 1000);   // an old attempt's answer: ignored
    fails += lan_relay_refusal(&detail) != LR_NAK_ROOM_FULL;
    relay_receive(b, lr_join_nak(b, 78, LR_NAK_RELAY_FULL, 0), 1000);   // a relay up to 1.1.12: detail 0
    fails += !(lan_relay_refusal(&detail) == LR_NAK_RELAY_FULL && detail == LR_NAK_DETAIL_NONE);
    plat_mutex_lock(&lock);
    R.state = R_JOIN, R.nonce = 79;
    plat_mutex_unlock(&lock);
    relay_receive(b, lr_join_ack(b, 79, 0x1234, token, 0x0A0D0002u, 16, 12, 5, 30, LR_MAX_PAYLOAD), 1000);
    fails += lan_relay_refusal(&detail) != 0;   // joined: forgotten
    plat_mutex_lock(&lock);
    R.state = state0, R.nonce = nonce0, R.session = 0, cfg.ip = ip0, cfg.mask = mask0, R.want_vip = want0;
    memset(R.token, 0, sizeof R.token);
    plat_mutex_unlock(&lock);
    return fails;
}

// --probe-lan: the roster's names as lan_relay_member_name gives them (no relay needed). 0 failures expected.
int lan_roster_selftest(void) {
    int fails = 0;
    char name[33], self0[sizeof cfg.name];
    plat_mutex_lock(&lock);
    memcpy(self0, cfg.name, sizeof self0);
    snprintf(cfg.name, sizeof cfg.name, "Charlie");
    roster_note(1, 0x0A0D0002u, "Alpha");
    roster_note(1, 0x0A0D0003u, "Bravo");
    roster_note(1, 0x0A0D0004u, "Alpha");     // a second member joined with Alpha's name
    roster_note(1, 0x0A0D0005u, "Charlie");   // a member joined with this player's own name
    plat_mutex_unlock(&lock);
    fails += !(lan_relay_member_name(0x0A0D0003u, name, sizeof name) && !strcmp(name, "Bravo"));
    fails += lan_relay_member_name(0x0A0D0002u, name, sizeof name) || lan_relay_member_name(0x0A0D0004u, name, sizeof name);
    fails += lan_relay_member_name(0x0A0D0005u, name, sizeof name);
    plat_mutex_lock(&lock);
    roster_note(0, 0x0A0D0004u, "");          // the second Alpha left: the first one is Alpha again
    plat_mutex_unlock(&lock);
    fails += !(lan_relay_member_name(0x0A0D0002u, name, sizeof name) && !strcmp(name, "Alpha"));
    fails += lan_relay_member_name(0x0A0D0009u, name, sizeof name);   // not in the room
    plat_mutex_lock(&lock);
    memset(roster, 0, sizeof roster);
    memcpy(cfg.name, self0, sizeof self0);
    plat_mutex_unlock(&lock);
    return fails;
}

static void relay_main(void *arg) {
    (void)arg;
    uint8_t b[2048];
    for (;;) {
        relay_timers(plat_now_ms());
        int ready = 0;
        lan_host_wait(&R.up, 1, &ready, 50);
        for (int k = 0; k < 64; k++) {
            uint32_t ip;
            uint16_t port;
            int trunc, err;
            long n = lan_host_recvfrom(R.up, b, sizeof b, &ip, &port, &trunc, &err);
            if (n < 0) break;
            if (ip != cfg.relay_ip || port != cfg.relay_port) {   // the relay is the host socket's only peer
                if (R.foreign++ < 4) lan_refused("a datagram from", ip, port, "not the relay");
                continue;
            }
            if (!trunc) relay_receive(b, (size_t)n, plat_now_ms());
        }
    }
}

static int relay_start(char *err, size_t cap) {
    int e;
    // The uplink stays on the loopback interface when the relay is local (tests, a relay on this PC).
    R.up = lan_host_udp((cfg.relay_ip >> 24) == 127 ? 0x7F000001u : 0, 0, &e);
    if (!R.up) return snprintf(err, cap, "cannot open the relay socket (errno %d)", e), 0;
    // The relay is one unicast peer: without SO_BROADCAST a relay address that is a subnet's broadcast (one a match code
    // could name) fails to send instead of reaching every machine of that subnet
    if (!lan_host_broadcast(R.up, 0)) rt_log("lan: the relay socket keeps SO_BROADCAST");
    R.keepalive_ms = 5000;
    R.silent_ms = 15000;
    plat_mutex_lock(&lock);
    relay_restart(plat_now_ms(), 0, 0);   // the relay thread sends the first HELLO
    plat_mutex_unlock(&lock);
    if (!plat_thread_new(relay_main, NULL, "lan relay")) return snprintf(err, cap, "cannot start the relay thread"), 0;
    return 1;
}

void lan_leave(void) {
    uint8_t b[64];
    plat_mutex_lock(&lock);
    if (started && cfg.mode == LAN_RELAY && R.state == R_JOINED) {
        size_t n = lr_leave(b, R.session, R.token);
        relay_tx(b, n);
        relay_tx(b, n);   // twice: a lost LEAVE costs the other members the 30 s idle timeout
        R.state = R_STOPPED;
    }
    plat_mutex_unlock(&lock);
}

// ---- nifm
static int nifm_local, nifm_state;   // nifm_state: 0 no request, 1 accepted, 2 refused

void lan_nifm_local(int local) {
    plat_mutex_lock(&lock);
    nifm_local = local != 0;
    plat_mutex_unlock(&lock);
}

static int joined_locked(void) { return cfg.mode == LAN_LAN ? cfg.ip != 0 : R.state == R_JOINED; }

int lan_nifm_submit(void) {
    int away = kern_core_away();
    plat_mutex_lock(&lock);
    if (!nifm_local) {
        nifm_state = 2;
        plat_mutex_unlock(&lock);
        kern_core_back(&away);
        rt_log("lan: nifm internet request refused (LAN play provides a local network only)");
        return 0;
    }
    uint64_t t0 = plat_now_ms();
    while (!joined_locked() && plat_now_ms() - t0 < 10000) plat_cond_wait_ms(&changed, &lock, 100);
    nifm_state = joined_locked() ? 1 : 2;
    int ok = nifm_state == 1;
    plat_mutex_unlock(&lock);
    kern_core_back(&away);
    rt_log("lan: nifm local network request %s", ok ? "accepted" : "failed: not joined to the relay");
    observe(ok ? LAN_OBS_NIFM : LAN_OBS_NIFM_FAILED, 0, 0, NULL, 0);
    return ok;
}

int lan_nifm_available(void) {
    plat_mutex_lock(&lock);
    int ok = nifm_state == 1 && joined_locked();
    plat_mutex_unlock(&lock);
    return ok;
}

void lan_nifm_cancel(void) {
    plat_mutex_lock(&lock);
    nifm_state = 0;
    plat_mutex_unlock(&lock);
}

int lan_nifm_ipconfig(uint32_t *ip, uint32_t *mask) {
    plat_mutex_lock(&lock);
    int ok = nifm_state == 1 && joined_locked();
    if (ok) *ip = cfg.ip, *mask = cfg.mask;
    plat_mutex_unlock(&lock);
    return ok;
}

// ---- start
int lan_net_start(const LanCfg *c, char *err, size_t cap) {
    char a[16], m[16], g[16], desc[96];
    if (started) return 1;
    cfg = *c;
    if (cfg.mode == LAN_OFF) return 1;
    if (!lan_host_init(err, cap)) return 0;
    if (cfg.mode == LAN_LAN) {
        if (cfg.detect) {
            if (!lan_host_detect(&cfg.ip, &cfg.mask, &cfg.gateway, desc, sizeof desc))
                return snprintf(err, cap, "no network adapter with a private IPv4 address is up (give one with "
                                          "SPLAT3_LAN=lan:<address>/<prefix>)"), 0;
        } else if ((cfg.ip >> 24) != 127) {
            uint32_t mask;
            if (!lan_host_find(cfg.ip, &mask, &cfg.gateway))
                return snprintf(err, cap, "no up adapter has the address %s", lan_ip_str(cfg.ip, a)), 0;
            snprintf(desc, sizeof desc, "given");
        } else {
            snprintf(desc, sizeof desc, "loopback");
        }
        if ((cfg.gateway & cfg.mask) != (cfg.ip & cfg.mask)) cfg.gateway = 0;
        rt_log("lan: lan mode on %s/%d (%s), broadcast %s, gateway %s refused, sockets bound to %s", lan_ip_str(cfg.ip, a),
               lan_mask_prefix(cfg.mask), desc, lan_ip_str(cfg.ip | ~cfg.mask, m),
               cfg.gateway ? lan_ip_str(cfg.gateway, g) : "none", cfg.bind_any ? "0.0.0.0" : a);
    } else {
        rt_log("lan: relay mode, relay %s:%u, room %s%s%s", lan_ip_str(cfg.relay_ip, a), cfg.relay_port, cfg.room,
               cfg.name[0] ? ", name " : "", cfg.name);
        if (!relay_start(err, cap)) return 0;
    }
    started = 1;
    return 1;
}

#ifndef _WIN32
// The host side exists for Windows only so far (lan_host_win.c).
int lan_host_init(char *err, size_t cap) { return snprintf(err, cap, "LAN play is implemented on Windows only"), 0; }
int lan_host_detect(uint32_t *ip, uint32_t *mask, uint32_t *gateway, char *desc, size_t cap) {
    (void)ip, (void)mask, (void)gateway, (void)desc, (void)cap;
    return 0;
}
int lan_host_find(uint32_t ip, uint32_t *mask, uint32_t *gateway) { (void)ip, (void)mask, (void)gateway; return 0; }
LanHost *lan_host_udp(uint32_t ip, uint16_t port, int *err) { (void)ip, (void)port; *err = LE_NETDOWN; return NULL; }
uint16_t lan_host_port(LanHost *h) { (void)h; return 0; }
long lan_host_sendto(LanHost *h, const void *p, size_t n, uint32_t ip, uint16_t port, int *err) {
    (void)h, (void)p, (void)n, (void)ip, (void)port;
    *err = LE_NETDOWN;
    return -1;
}
long lan_host_recvfrom(LanHost *h, void *p, size_t n, uint32_t *ip, uint16_t *port, int *trunc, int *err) {
    (void)h, (void)p, (void)n, (void)ip, (void)port, (void)trunc;
    *err = LE_NETDOWN;
    return -1;
}
int lan_host_wait(LanHost *const *h, int n, int *ready, int timeout_ms) { (void)h, (void)n, (void)ready, (void)timeout_ms; return 0; }
int lan_host_setbuf(LanHost *h, int rcv, int bytes) { (void)h, (void)rcv, (void)bytes; return 0; }
int lan_host_broadcast(LanHost *h, int on) { (void)h, (void)on; return 0; }
void lan_host_close(LanHost *h) { (void)h; }
#endif
