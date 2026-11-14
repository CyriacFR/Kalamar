// LAN play's guest side (lan.h): nn::socket, its nnsocket* C shims and nn::nifm when SPLAT3_LAN is set, and the call
// log SPLAT3_NET_LOG in any mode. lan_install (boot.c, before the modules load) adds these overrides with
// hostfn_add_override, which wins over the linker table; with neither variable set it adds nothing and ov_offline.c
// answers every network call exactly as before.
//
// What stays offline in every mode: nn::ssl and nnssl*, libcurl and NPLN/gRPC (they reach the network only through
// the socket calls here, which open UDP sockets only and resolve no name), nn::ldn (local wireless), friends, bcat,
// prepo, the network service account, nsd, the proxy setting.
//
// Guest structures (FreeBSD's, aarch64): sockaddr_in {u8 len, u8 family, be16 port, be32 addr, u8 zero[8]};
// pollfd {i32 fd, i16 events, i16 revents}; msghdr {ptr name, u32 namelen, ptr iov, i32 iovlen, ptr control,
// u32 controllen, i32 flags} with natural alignment (48 bytes); iovec {ptr base, u64 len}.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cfg.h"
#include "host.h"
#include "lan.h"
#include "platform/platform.h"

#define R_NIFM_UNAVAILABLE RT_RESULT(110, 300)   // ov_offline.c's: the request found no network
enum { AF_INET_ = 2, EAI_AGAIN_ = 2 };

static _Thread_local int32_t lan_errno;

#define ENTER(c) (lan_caller = (c)->x[30])
static void fail(Cpu *c, int e) {
    lan_errno = e;
    ret_x(c, (uint64_t)(int64_t)-1);
}
static void ret_int(Cpu *c, long r, int err) {
    if (r < 0) lan_errno = err;
    ret_x(c, (uint64_t)(int64_t)r);
}

// Guest sockaddr_in -> host-order address. 0 with *err set when it is not one.
static int sa_in(const uint8_t *sa, uint32_t len, uint32_t *ip, uint16_t *port, int *err) {
    if (!sa) return *err = LE_FAULT, 0;
    if (len < 8) return *err = LE_INVAL, 0;
    if (sa[1] != AF_INET_) return *err = LE_AFNOSUPPORT, 0;
    *port = (uint16_t)(sa[2] << 8 | sa[3]);
    *ip = (uint32_t)sa[4] << 24 | (uint32_t)sa[5] << 16 | (uint32_t)sa[6] << 8 | sa[7];
    return 1;
}

// Host-order address -> guest sockaddr_in at sa, *len bytes at most; *len becomes 16 (BSD).
static void sa_out(uint8_t *sa, uint32_t *len, uint32_t ip, uint16_t port) {
    if (!sa || !len) return;
    uint8_t b[16] = {16, AF_INET_, (uint8_t)(port >> 8), (uint8_t)port, (uint8_t)(ip >> 24), (uint8_t)(ip >> 16),
                     (uint8_t)(ip >> 8), (uint8_t)ip};
    memcpy(sa, b, *len < 16 ? *len : 16);
    *len = 16;
}

// ==== nn::socket
static void ov_initialize(Cpu *c) { ret_x(c, 0); }   // Initialize(pool, size, alloc size, concurrency), Finalize
static void ov_socket(Cpu *c) {
    ENTER(c);
    int err = 0;
    int fd = lan_socket((int)arg_x(c, 0), (int)arg_x(c, 1), (int)arg_x(c, 2), &err);
    ret_int(c, fd, err);
}
static void ov_bind(Cpu *c) {   // int Bind(int, const sockaddr *, socklen_t)
    ENTER(c);
    int err = 0;
    uint32_t ip;
    uint16_t port;
    if (!sa_in(arg_p(c, 1), (uint32_t)arg_x(c, 2), &ip, &port, &err)) return fail(c, err);
    ret_int(c, lan_bind((int)arg_x(c, 0), ip, port, &err), err);
}
static void ov_connect(Cpu *c) {
    ENTER(c);
    int err = 0;
    uint32_t ip;
    uint16_t port;
    if (!sa_in(arg_p(c, 1), (uint32_t)arg_x(c, 2), &ip, &port, &err)) return fail(c, err);
    ret_int(c, lan_connect((int)arg_x(c, 0), ip, port, &err), err);
}
// ssize_t SendTo(int, const void *, size_t, flags, const sockaddr *, socklen_t)
static void ov_sendto(Cpu *c) {
    ENTER(c);
    int err = 0;
    uint32_t ip = 0;
    uint16_t port = 0;
    const uint8_t *sa = arg_p(c, 4);
    if (sa && !sa_in(sa, (uint32_t)arg_x(c, 5), &ip, &port, &err)) return fail(c, err);
    ret_int(c, lan_sendto((int)arg_x(c, 0), arg_p(c, 1), arg_x(c, 2), (int)arg_x(c, 3), sa != NULL, ip, port, &err), err);
}
static void ov_send(Cpu *c) {   // ssize_t Send(int, const void *, size_t, int)
    ENTER(c);
    int err = 0;
    ret_int(c, lan_sendto((int)arg_x(c, 0), arg_p(c, 1), arg_x(c, 2), (int)arg_x(c, 3), 0, 0, 0, &err), err);
}
// ssize_t RecvFrom(int, void *, size_t, flags, sockaddr *, socklen_t *)
static void ov_recvfrom(Cpu *c) {
    ENTER(c);
    int err = 0, trunc;
    uint32_t ip;
    uint16_t port;
    long r = lan_recvfrom((int)arg_x(c, 0), arg_p(c, 1), arg_x(c, 2), (int)arg_x(c, 3), &ip, &port, &trunc, &err);
    if (r >= 0) sa_out(arg_p(c, 4), arg_p(c, 5), ip, port);
    ret_int(c, r, err);
}
static void ov_recv(Cpu *c) {   // ssize_t Recv(int, void *, size_t, int)
    ENTER(c);
    int err = 0, trunc;
    uint32_t ip;
    uint16_t port;
    ret_int(c, lan_recvfrom((int)arg_x(c, 0), arg_p(c, 1), arg_x(c, 2), (int)arg_x(c, 3), &ip, &port, &trunc, &err), err);
}

typedef struct { uint64_t base, len; } GuestIov;
typedef struct {
    uint64_t name;
    uint32_t namelen, pad0;
    uint64_t iov;
    int32_t iovlen, pad1;
    uint64_t control;
    uint32_t controllen;
    int32_t flags;
} GuestMsg;
_Static_assert(sizeof(GuestMsg) == 48 && sizeof(GuestIov) == 16, "FreeBSD aarch64 msghdr");

// ssize_t SendMsg(int, const msghdr *, int): the iovecs gathered into one datagram
static void ov_sendmsg(Cpu *c) {
    ENTER(c);
    const GuestMsg *m = arg_p(c, 1);
    int err = 0;
    uint32_t ip = 0;
    uint16_t port = 0;
    if (!m || m->iovlen < 0 || m->iovlen > 64) return fail(c, LE_INVAL);
    if (m->name && !sa_in((const uint8_t *)(uintptr_t)m->name, m->namelen, &ip, &port, &err)) return fail(c, err);
    uint8_t *buf = malloc(65536);
    if (!buf) return fail(c, LE_NOBUFS);
    size_t n = 0;
    const GuestIov *v = (const GuestIov *)(uintptr_t)m->iov;
    for (int i = 0; i < m->iovlen; i++) {
        if (n + v[i].len > 65536) {
            free(buf);
            return fail(c, LE_MSGSIZE);
        }
        memcpy(buf + n, (const void *)(uintptr_t)v[i].base, v[i].len);
        n += v[i].len;
    }
    long r = lan_sendto((int)arg_x(c, 0), buf, n, (int)arg_x(c, 2), m->name != 0, ip, port, &err);
    free(buf);
    ret_int(c, r, err);
}
// ssize_t RecvMsg(int, msghdr *, int): one datagram scattered over the iovecs; no control data
static void ov_recvmsg(Cpu *c) {
    ENTER(c);
    GuestMsg *m = arg_p(c, 1);
    int err = 0, trunc;
    uint32_t ip;
    uint16_t port;
    if (!m || m->iovlen < 0 || m->iovlen > 64) return fail(c, LE_INVAL);
    uint8_t *buf = malloc(65536);
    if (!buf) return fail(c, LE_NOBUFS);
    size_t cap = 0;
    const GuestIov *v = (const GuestIov *)(uintptr_t)m->iov;
    for (int i = 0; i < m->iovlen; i++) cap += v[i].len;
    long r = lan_recvfrom((int)arg_x(c, 0), buf, cap < 65536 ? cap : 65536, (int)arg_x(c, 2), &ip, &port, &trunc, &err);
    if (r >= 0) {
        size_t off = 0;
        for (int i = 0; i < m->iovlen && off < (size_t)r; i++) {
            size_t k = (size_t)r - off < v[i].len ? (size_t)r - off : v[i].len;
            memcpy((void *)(uintptr_t)v[i].base, buf + off, k);
            off += k;
        }
        if (m->name) sa_out((uint8_t *)(uintptr_t)m->name, &m->namelen, ip, port);
        m->controllen = 0;
        m->flags = trunc ? 0x10 : 0;   // MSG_TRUNC
    }
    free(buf);
    ret_int(c, r, err);
}
static void ov_close(Cpu *c) {
    ENTER(c);
    int err = 0;
    ret_int(c, lan_close((int)arg_x(c, 0), &err), err);
}
static void ov_shutdown(Cpu *c) {
    ENTER(c);
    int err = 0;
    ret_int(c, lan_shutdown((int)arg_x(c, 0), (int)arg_x(c, 1), &err), err);
}
static void ov_getsockname(Cpu *c) {   // int GetSockName(int, sockaddr *, socklen_t *)
    ENTER(c);
    int err = 0;
    uint32_t ip;
    uint16_t port;
    if (!arg_p(c, 1) || !arg_p(c, 2)) return fail(c, LE_FAULT);
    int r = lan_getsockname((int)arg_x(c, 0), &ip, &port, &err);
    if (!r) sa_out(arg_p(c, 1), arg_p(c, 2), ip, port);
    ret_int(c, r, err);
}
static void ov_getpeername(Cpu *c) {
    ENTER(c);
    int err = 0;
    uint32_t ip;
    uint16_t port;
    if (!arg_p(c, 1) || !arg_p(c, 2)) return fail(c, LE_FAULT);
    int r = lan_getpeername((int)arg_x(c, 0), &ip, &port, &err);
    if (!r) sa_out(arg_p(c, 1), arg_p(c, 2), ip, port);
    ret_int(c, r, err);
}
// int SetSockOpt(int, int level, int option, const void *, socklen_t)
static void ov_setsockopt(Cpu *c) {
    ENTER(c);
    int err = 0;
    ret_int(c, lan_setsockopt((int)arg_x(c, 0), (int)arg_x(c, 1), (int)arg_x(c, 2), arg_p(c, 3), (uint32_t)arg_x(c, 4), &err),
            err);
}
static void ov_getsockopt(Cpu *c) {   // int GetSockOpt(int, int, int, void *, socklen_t *)
    ENTER(c);
    int err = 0;
    ret_int(c, lan_getsockopt((int)arg_x(c, 0), (int)arg_x(c, 1), (int)arg_x(c, 2), arg_p(c, 3), arg_p(c, 4), &err), err);
}
static void ov_fcntl(Cpu *c) {   // int Fcntl(int, int cmd, ...): the third argument in x2 (AAPCS64 variadic)
    ENTER(c);
    int err = 0;
    ret_int(c, lan_fcntl((int)arg_x(c, 0), (int)arg_x(c, 1), (int)arg_x(c, 2), &err), err);
}
static void ov_poll(Cpu *c) {   // int Poll(pollfd *, nfds_t, int timeout_ms)
    ENTER(c);
    int err = 0;
    if (!arg_p(c, 0) && arg_x(c, 1)) return fail(c, LE_FAULT);
    ret_int(c, lan_poll(arg_p(c, 0), arg_x(c, 1), (int)arg_x(c, 2), &err), err);
}
static void ov_stream_only(Cpu *c) {   // Listen, Accept: datagram sockets have neither
    ENTER(c);
    fail(c, lan_is_socket((int)arg_x(c, 0)) ? LE_OPNOTSUPP : LE_BADF);
}
static void ov_no_eventfd(Cpu *c) { fail(c, LE_NETDOWN); }   // EventFd: as offline (gRPC only)
static void ov_bad_fd(Cpu *c) { fail(c, LE_BADF); }           // EventFdRead/Write: there is none
static void ov_errno(Cpu *c) { ret_x(c, (uint64_t)(int64_t)lan_errno); }
static void ov_set_errno(Cpu *c) { lan_errno = (int32_t)arg_x(c, 0); }
// Names are never resolved, in any mode: no DNS (GetAddrInfo, GetHostByName) and no resolver.
static void ov_getaddrinfo(Cpu *c) {
    uint64_t *out = arg_p(c, 3);
    if (out) *out = 0;
    static volatile int32_t told;
    if (plat_inc32(&told) <= 16) {
        char lr[96] = "host";
        const char *node = arg_p(c, 0);
        if (c->x[30]) ldr_describe(c->x[30] - 4, lr, sizeof lr);
        rt_log("lan: REFUSED name lookup '%.64s' (no DNS in LAN play) from %s", node ? node : "", lr);
    }
    ret_x(c, EAI_AGAIN_);
}
static void ov_gethostbyname(Cpu *c) { ov_getaddrinfo(c), ret_x(c, 0); }
static void ov_freeaddrinfo(Cpu *c) { (void)c; }
static void ov_gai_strerror(Cpu *c) {
    static char *text;
    if (!text) {
        static const char msg[] = "Temporary failure in name resolution (LAN play resolves no names)";
        char *p = mem_kernel_alloc(sizeof msg, 16);
        memcpy(p, msg, sizeof msg);
        text = p;
    }
    ret_x(c, (uint64_t)(uintptr_t)text);
}
static void ov_resolver_set(Cpu *c) { ret_x(c, 0); }
static void ov_resolver_get(Cpu *c) { fail(c, LE_INVAL); }
static void ov_htonl(Cpu *c) { ret_x(c, __builtin_bswap32((uint32_t)arg_x(c, 0))); }
static void ov_htons(Cpu *c) { ret_x(c, __builtin_bswap16((uint16_t)arg_x(c, 0))); }
static void ov_inet_pton(Cpu *c) {   // int InetPton(int af, const char *, void *): IPv4
    const char *s = arg_p(c, 1);
    uint8_t *out = arg_p(c, 2);
    uint32_t ip;
    if ((int)arg_x(c, 0) != AF_INET_) return fail(c, LE_AFNOSUPPORT);
    if (!s || !out || !lan_ip_parse(s, &ip)) return ret_x(c, 0);
    out[0] = (uint8_t)(ip >> 24), out[1] = (uint8_t)(ip >> 16), out[2] = (uint8_t)(ip >> 8), out[3] = (uint8_t)ip;
    ret_x(c, 1);
}
static void ov_inet_ntop(Cpu *c) {   // const char *InetNtop(int af, const void *, char *, socklen_t)
    const uint8_t *a = arg_p(c, 1);
    char *out = arg_p(c, 2), buf[16];
    if ((int)arg_x(c, 0) != AF_INET_ || !a || !out) {
        lan_errno = (int)arg_x(c, 0) != AF_INET_ ? LE_AFNOSUPPORT : LE_INVAL;
        return ret_x(c, 0);
    }
    lan_ip_str((uint32_t)a[0] << 24 | (uint32_t)a[1] << 16 | (uint32_t)a[2] << 8 | a[3], buf);
    if (strlen(buf) + 1 > (uint32_t)arg_x(c, 3)) {
        lan_errno = LE_INVAL;   // ENOSPC on FreeBSD 13; the guest only checks for NULL
        return ret_x(c, 0);
    }
    memcpy(out, buf, strlen(buf) + 1);
    ret_x(c, (uint64_t)(uintptr_t)out);
}

// ==== nn::nifm: one request, accepted only as a local-network request
static void ov_nifm_initialize(Cpu *c) { ret_x(c, 0); }
static void ov_nifm_local_mode(Cpu *c) { lan_nifm_local((int)(arg_x(c, 0) & 1)); }
static void ov_nifm_submit(Cpu *c) { ENTER(c); lan_nifm_submit(); }
static void ov_nifm_cancel(Cpu *c) { lan_nifm_cancel(); (void)c; }
static void ov_nifm_available(Cpu *c) { ret_x(c, (uint64_t)lan_nifm_available()); }
static void ov_nifm_on_hold(Cpu *c) { ret_x(c, 0); }
// Result GetCurrentIpConfigInfo(in_addr *ip, *mask, *gateway, *dns1, *dns2): network byte order; no gateway, no DNS
static void ov_nifm_ipconfig(Cpu *c) {
    uint32_t ip, mask;
    if (!lan_nifm_ipconfig(&ip, &mask)) return ret_x(c, R_NIFM_UNAVAILABLE);
    uint32_t v[5] = {__builtin_bswap32(ip), __builtin_bswap32(mask), 0, 0, 0};
    for (int i = 0; i < 5; i++) {
        uint32_t *p = arg_p(c, i);
        if (p) *p = v[i];
    }
    ret_x(c, 0);
}

// ==== the table: every network name main imports (Splatoon 3 11.2.0: 30 nn::socket, 29 nnsocket*, 9 nn::nifm, 26
// nn::ldn). lan: the implementation in LAN play (NULL: ov_offline.c's stays). args: how SPLAT3_NET_LOG prints the
// arguments (i int, x hex, p pointer, s string, a sockaddr + length, o sockaddr out + length pointer, S send buffer +
// length, R receive buffer + capacity, v option value + length, P pollfd array + count, m msghdr, I in_addr out, b
// bool, - skipped). ret: i int (-1: errno), r Result, b bool, v void, p pointer, e errno value.
typedef struct {
    const char *name, *label;
    GuestFn lan;
    const char *args;
    char ret;
    char quiet;   // SPLAT3_NET_LOG=1 counts these instead of logging each: e every call, a EAGAIN/0-ready, c unchanged result
    GuestFn base;
    volatile int64_t quiet_calls;
    uint64_t last;
} NetFn;

#define SOCK(m) "_ZN2nn6socket" m
#define NIFM(m) "_ZN2nn4nifm" m
#define LDN(m) "_ZN2nn3ldn" m
static NetFn fns[] = {
    {SOCK("10InitializeEPvmmi"), "socket::Initialize", ov_initialize, "pxxi", 'r'},
    {SOCK("8FinalizeEv"), "socket::Finalize", ov_initialize, "", 'r'},
    {SOCK("6SocketENS0_6FamilyENS0_4TypeENS0_8ProtocolE"), "socket::Socket", ov_socket, "ixi", 'i'},
    {SOCK("6SocketEiii"), "socket::Socket", ov_socket, "ixi", 'i'},
    {SOCK("4BindEiPK8sockaddrj"), "socket::Bind", ov_bind, "ia-", 'i'},
    {SOCK("4BindEiPKNS0_8SockAddrEj"), "socket::Bind", ov_bind, "ia-", 'i'},
    {SOCK("10SetSockOptEiNS0_5LevelENS0_6OptionEPKvj"), "socket::SetSockOpt", ov_setsockopt, "ixxv-", 'i'},
    {SOCK("6SendToEiPKvmNS0_7MsgFlagEPKNS0_8SockAddrEj"), "socket::SendTo", ov_sendto, "iS-xa-", 'i'},
    {SOCK("6SendToEiPKvmiPK8sockaddrj"), "socket::SendTo", ov_sendto, "iS-xa-", 'i'},
    {SOCK("8RecvFromEiPvmNS0_7MsgFlagEPNS0_8SockAddrEPj"), "socket::RecvFrom", ov_recvfrom, "iR-xo-", 'i', 'a'},
    {SOCK("8RecvFromEiPvmiP8sockaddrPj"), "socket::RecvFrom", ov_recvfrom, "iR-xo-", 'i', 'a'},
    {SOCK("4PollEP6pollfdmi"), "socket::Poll", ov_poll, "P-i", 'i', 'a'},
    {SOCK("5CloseEi"), "socket::Close", ov_close, "i", 'i'},
    {SOCK("8ShutdownEii"), "socket::Shutdown", ov_shutdown, "ii", 'i'},
    {SOCK("7EventFdEmNS0_12EventFdFlagsE"), "socket::EventFd", ov_no_eventfd, "xx", 'i'},
    {SOCK("11EventFdReadEiPm"), "socket::EventFdRead", ov_bad_fd, "ip", 'i'},
    {SOCK("12EventFdWriteEim"), "socket::EventFdWrite", ov_bad_fd, "ix", 'i'},
    {SOCK("11GetAddrInfoEPKcS2_PK8addrinfoPPS3_"), "socket::GetAddrInfo", ov_getaddrinfo, "sspp", 'e'},
    {SOCK("11GetAddrInfoEPKcS2_PKNS0_8AddrInfoEPPS3_"), "socket::GetAddrInfo", ov_getaddrinfo, "sspp", 'e'},
    {SOCK("12FreeAddrInfoEP8addrinfo"), "socket::FreeAddrInfo", ov_freeaddrinfo, "p", 'v'},
    {SOCK("12FreeAddrInfoEPNS0_8AddrInfoE"), "socket::FreeAddrInfo", ov_freeaddrinfo, "p", 'v'},
    {SOCK("11GAIStrErrorENS0_7AiErrnoE"), "socket::GAIStrError", ov_gai_strerror, "i", 'p'},
    {SOCK("12GetLastErrnoEv"), "socket::GetLastErrno", ov_errno, "", 'e', 'e'},
    {SOCK("12GetLastErrorEv"), "socket::GetLastError", ov_errno, "", 'e', 'e'},
    {SOCK("8InetNtopEiPKvPcj"), "socket::InetNtop", ov_inet_ntop, "ippi", 'p'},
    {SOCK("8InetPtonEiPKcPv"), "socket::InetPton", ov_inet_pton, "isp", 'i'},
    {SOCK("9InetHtonlEj"), "socket::InetHtonl", ov_htonl, "x", 'p', 'e'},
    {SOCK("9InetHtonsEt"), "socket::InetHtons", ov_htons, "x", 'p', 'e'},
    {SOCK("9InetNtohlEj"), "socket::InetNtohl", ov_htonl, "x", 'p', 'e'},
    {SOCK("9InetNtohsEt"), "socket::InetNtohs", ov_htons, "x", 'p', 'e'},
    {"nnsocketSocket", "nnsocketSocket", ov_socket, "ixi", 'i'},
    {"nnsocketBind", "nnsocketBind", ov_bind, "ia-", 'i'},
    {"nnsocketConnect", "nnsocketConnect", ov_connect, "ia-", 'i'},
    {"nnsocketAccept", "nnsocketAccept", ov_stream_only, "ipp", 'i'},
    {"nnsocketListen", "nnsocketListen", ov_stream_only, "ii", 'i'},
    {"nnsocketClose", "nnsocketClose", ov_close, "i", 'i'},
    {"nnsocketShutdown", "nnsocketShutdown", ov_shutdown, "ii", 'i'},
    {"nnsocketFcntl", "nnsocketFcntl", ov_fcntl, "iix", 'i'},
    {"nnsocketGetSockName", "nnsocketGetSockName", ov_getsockname, "io-", 'i'},
    {"nnsocketGetPeerName", "nnsocketGetPeerName", ov_getpeername, "io-", 'i'},
    {"nnsocketGetSockOpt", "nnsocketGetSockOpt", ov_getsockopt, "ixxpp", 'i'},
    {"nnsocketSetSockOpt", "nnsocketSetSockOpt", ov_setsockopt, "ixxv-", 'i'},
    {"nnsocketPoll", "nnsocketPoll", ov_poll, "P-i", 'i', 'a'},
    {"nnsocketRecv", "nnsocketRecv", ov_recv, "iR-x", 'i', 'a'},
    {"nnsocketRecvMsg", "nnsocketRecvMsg", ov_recvmsg, "imx", 'i', 'a'},
    {"nnsocketSend", "nnsocketSend", ov_send, "iS-x", 'i'},
    {"nnsocketSendMsg", "nnsocketSendMsg", ov_sendmsg, "imx", 'i'},
    {"nnsocketGetAddrInfo", "nnsocketGetAddrInfo", ov_getaddrinfo, "sspp", 'e'},
    {"nnsocketFreeAddrInfo", "nnsocketFreeAddrInfo", ov_freeaddrinfo, "p", 'v'},
    {"nnsocketGAIStrError", "nnsocketGAIStrError", ov_gai_strerror, "i", 'p'},
    {"nnsocketGetHostByNameWithOptions", "nnsocketGetHostByNameWithOptions", ov_gethostbyname, "spp", 'p'},
    {"nnsocketGetLastErrno", "nnsocketGetLastErrno", ov_errno, "", 'e', 'e'},
    {"nnsocketSetLastErrno", "nnsocketSetLastErrno", ov_set_errno, "i", 'v', 'e'},
    {"nnsocketResolverSetOption", "nnsocketResolverSetOption", ov_resolver_set, "xpx", 'i'},
    {"nnsocketResolverGetOption", "nnsocketResolverGetOption", ov_resolver_get, "xpp", 'i'},
    {"nnsocketInetHtonl", "nnsocketInetHtonl", ov_htonl, "x", 'p', 'e'},
    {"nnsocketInetHtons", "nnsocketInetHtons", ov_htons, "x", 'p', 'e'},
    {"nnsocketInetNtohl", "nnsocketInetNtohl", ov_htonl, "x", 'p', 'e'},
    {"nnsocketInetNtohs", "nnsocketInetNtohs", ov_htons, "x", 'p', 'e'},
    {NIFM("10InitializeEv"), "nifm::Initialize", ov_nifm_initialize, "", 'r'},
    {NIFM("19SetLocalNetworkModeEb"), "nifm::SetLocalNetworkMode", ov_nifm_local_mode, "b", 'v'},
    {NIFM("27SubmitNetworkRequestAndWaitEv"), "nifm::SubmitNetworkRequestAndWait", ov_nifm_submit, "", 'v'},
    {NIFM("20CancelNetworkRequestEv"), "nifm::CancelNetworkRequest", ov_nifm_cancel, "", 'v'},
    {NIFM("18IsNetworkAvailableEv"), "nifm::IsNetworkAvailable", ov_nifm_available, "", 'b', 'c'},
    {NIFM("22IsNetworkRequestOnHoldEv"), "nifm::IsNetworkRequestOnHold", ov_nifm_on_hold, "", 'b', 'c'},
    {NIFM("22GetCurrentIpConfigInfoEP7in_addrS2_S2_S2_S2_"), "nifm::GetCurrentIpConfigInfo", ov_nifm_ipconfig, "IIIII", 'r'},
    {NIFM("22GetCurrentProxySettingEPNS0_12ProxySettingE"), "nifm::GetCurrentProxySetting", NULL, "p", 'r'},
    {NIFM("31HandleNetworkRequestErrorResultEv"), "nifm::HandleNetworkRequestErrorResult", NULL, "", 'r'},
    {LDN("10InitializeEv"), "ldn::Initialize", NULL, "", 'r'},
    {LDN("8FinalizeEv"), "ldn::Finalize", NULL, "", 'v'},
    {LDN("8GetStateEv"), "ldn::GetState", NULL, "", 'p', 'c'},
    {LDN("19GetDisconnectReasonEv"), "ldn::GetDisconnectReason", NULL, "", 'p', 'c'},
    {LDN("20MakeBroadcastAddressENS0_11Ipv4AddressENS0_10SubnetMaskE"), "ldn::MakeBroadcastAddress", NULL, "xx", 'p'},
    {LDN("22AttachStateChangeEventEPNS_2os15SystemEventTypeE"), "ldn::AttachStateChangeEvent", NULL, "p", 'v'},
    {LDN("10DisconnectEv"), "ldn::Disconnect", NULL, "", 'r'},
    {LDN("11OpenStationEv"), "ldn::OpenStation", NULL, "", 'r'},
    {LDN("11ScanPrivateEPNS0_11NetworkInfoEPiiRKNS0_10ScanFilterEi"), "ldn::ScanPrivate", NULL, "ppipi", 'r'},
    {LDN("11SetProtocolENS0_8ProtocolE"), "ldn::SetProtocol", NULL, "i", 'r'},
    {LDN("12CloseStationEv"), "ldn::CloseStation", NULL, "", 'r'},
    {LDN("13CreateNetworkERKNS0_13NetworkConfigERKNS0_14SecurityConfigERKNS0_10UserConfigE"), "ldn::CreateNetwork", NULL,
     "ppp", 'r'},
    {LDN("14ConnectPrivateERKNS0_13NetworkConfigERKNS0_14SecurityConfigERKNS0_17SecurityParameterERKNS0_10UserConfigEiNS0_"
         "13ConnectOptionE"), "ldn::ConnectPrivate", NULL, "ppppix", 'r'},
    {LDN("14DestroyNetworkEv"), "ldn::DestroyNetwork", NULL, "", 'r'},
    {LDN("14GetIpv4AddressEPNS0_11Ipv4AddressEPNS0_10SubnetMaskE"), "ldn::GetIpv4Address", NULL, "pp", 'r'},
    {LDN("14GetNetworkInfoEPNS0_11NetworkInfoE"), "ldn::GetNetworkInfo", NULL, "p", 'r', 'c'},
    {LDN("15OpenAccessPointEv"), "ldn::OpenAccessPoint", NULL, "", 'r'},
    {LDN("16CloseAccessPointEv"), "ldn::CloseAccessPoint", NULL, "", 'r'},
    {LDN("16SetAdvertiseDataEPKvm"), "ldn::SetAdvertiseData", NULL, "px", 'r'},
    {LDN("20AddAcceptFilterEntryENS0_10MacAddressE"), "ldn::AddAcceptFilterEntry", NULL, "x", 'r'},
    {LDN("20CreateNetworkPrivateERKNS0_13NetworkConfigERKNS0_14SecurityConfigERKNS0_17SecurityParameterERKNS0_10UserConfigE"
         "iPKNS0_12AddressEntryE"), "ldn::CreateNetworkPrivate", NULL, "ppppip", 'r'},
    {LDN("20GetSecurityParameterEPNS0_17SecurityParameterE"), "ldn::GetSecurityParameter", NULL, "p", 'r'},
    {LDN("22SetStationAcceptPolicyENS0_12AcceptPolicyE"), "ldn::SetStationAcceptPolicy", NULL, "i", 'r'},
    {LDN("4ScanEPNS0_11NetworkInfoEPiiRKNS0_10ScanFilterEi"), "ldn::Scan", NULL, "ppipi", 'r'},
    {LDN("6RejectENS0_11Ipv4AddressE"), "ldn::Reject", NULL, "x", 'r'},
    {LDN("7ConnectERKNS0_11NetworkInfoERKNS0_14SecurityConfigERKNS0_10UserConfigEiNS0_13ConnectOptionE"), "ldn::Connect",
     NULL, "pppix", 'r'},
};
#define NFNS ((int)(sizeof fns / sizeof *fns))

// ==== the call log (SPLAT3_NET_LOG=1: every call but the repetitive ones, counted per 5 s; 2: every call)
static int log_level;
static GuestFn offline_errno;   // ov_offline.c's GetLastErrno, for the errno of an offline call
static uint64_t log_t0;
static volatile int64_t summary_at;

// The logger finds its entry by the name the loader registered (hostfn_name): an open-addressing table filled once by
// lan_install, read-only afterwards.
static NetFn *hash_table[512];
static void hash_build(void) {
    for (int i = 0; i < NFNS; i++) {
        uint32_t h = 2166136261u;
        for (const char *p = fns[i].name; *p; p++) h = (h ^ (uint8_t)*p) * 16777619u;
        uint32_t k = h & 511;
        while (hash_table[k]) k = (k + 1) & 511;
        hash_table[k] = &fns[i];
    }
}

static NetFn *lookup(const char *name) {
    if (!name) return NULL;
    uint32_t h = 2166136261u;
    for (const char *p = name; *p; p++) h = (h ^ (uint8_t)*p) * 16777619u;
    for (uint32_t k = 0; k < 512; k++) {
        NetFn *f = hash_table[(h + k) & 511];
        if (!f) return NULL;
        if (!strcmp(f->name, name)) return f;
    }
    return NULL;
}

typedef struct { char *p; size_t left; } Out;
static void put(Out *o, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static void put(Out *o, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(o->p, o->left, fmt, ap);
    va_end(ap);
    if (n < 0) return;
    size_t k = (size_t)n < o->left ? (size_t)n : o->left ? o->left - 1 : 0;
    o->p += k, o->left -= k;
}

static void put_sa(Out *o, const uint8_t *sa, uint32_t len) {
    if (!sa) return put(o, "NULL");
    if (len < 8 || !mem_guest_readable((uint64_t)(uintptr_t)sa, 8)) return put(o, "%p/%u", (const void *)sa, len);
    char a[16];
    put(o, "%s:%u%s", lan_ip_str((uint32_t)sa[4] << 24 | (uint32_t)sa[5] << 16 | (uint32_t)sa[6] << 8 | sa[7], a),
        sa[2] << 8 | sa[3], sa[1] == AF_INET_ ? "" : "(not AF_INET)");
}

static void put_hex(Out *o, const uint8_t *p, uint64_t n) {
    if (!p || !mem_guest_readable((uint64_t)(uintptr_t)p, n < 16 ? n : 16)) return;
    put(o, " [");
    for (uint64_t i = 0; i < n && i < 16; i++) put(o, "%s%02x", i ? " " : "", p[i]);
    put(o, "%s]", n > 16 ? " .." : "");
}

static int quiet_result(const NetFn *f, uint64_t r, int err) {
    switch (f->quiet) {
    case 'e': return 1;
    case 'a': return ((int64_t)r == -1 && err == LE_AGAIN) || (f->args[0] == 'P' && r == 0);
    case 'c': return 0;   // decided by the caller (the result changed or not)
    default: return 0;
    }
}

static void summary(uint64_t now_ms) {
    char line[1024];
    line[0] = 0;
    Out o = {line, sizeof line};
    for (int i = 0; i < NFNS; i++) {
        int64_t n = plat_xchg64(&fns[i].quiet_calls, 0);
        if (n) put(&o, "%s %s x%lld", o.p == line ? "" : ",", fns[i].label, (long long)n);
    }
    if (o.p != line) rt_log("net: +%.3f quiet calls in the last 5 s:%s", (now_ms - log_t0) / 1000.0, line);
}

static void net_logger(Cpu *c) {
    NetFn *f = lookup(hostfn_name(c->pc));
    if (!f) rt_fatal("net log: no entry for %s", hostfn_name(c->pc) ? hostfn_name(c->pc) : "?");
    uint64_t a[8], lr = c->x[30];
    for (int i = 0; i < 8; i++) a[i] = c->x[i];
    f->base(c);
    uint64_t r = c->x[0];
    int err = 0;
    if (f->ret == 'i' && (int32_t)r == -1) {   // int and ssize_t calls fail with -1 and an errno
        if (lan_mode() != LAN_OFF) {
            err = lan_errno;
        } else if (offline_errno) {   // ov_offline.c keeps its own errno: read it as the guest would, then put r back
            offline_errno(c);
            err = (int)c->x[0];
            c->x[0] = r;
        }
    }
    uint64_t now = plat_now_ms();
    int quiet = log_level < 2 && quiet_result(f, r, err);
    if (log_level < 2 && f->quiet == 'c') {
        quiet = f->last == r + 1;   // +1: the first call (last 0) is never quiet
        f->last = r + 1;
    }
    int64_t due = summary_at;
    if (now >= (uint64_t)due && plat_cas64(&summary_at, (int64_t)now + 5000, due) == due) summary(now);
    if (quiet) {
        plat_inc64(&f->quiet_calls);
        return;
    }
    char line[768];
    line[0] = 0;
    Out o = {line, sizeof line};
    int ai = 0;
    for (const char *k = f->args; *k && ai < 8; k++, ai++) {
        uint64_t v = a[ai];
        if (*k == '-') continue;
        put(&o, "%s", ai ? ", " : "");
        switch (*k) {
        case 'i': put(&o, "%d", (int32_t)v); break;
        case 'x': put(&o, "%#llx", (unsigned long long)v); break;
        case 'b': put(&o, "%s", v & 1 ? "true" : "false"); break;
        case 'p': put(&o, "%#llx", (unsigned long long)v); break;
        case 's': {
            const char *s = (const char *)(uintptr_t)v;
            if (s && mem_guest_readable(v, 1)) put(&o, "\"%.64s\"", s);
            else put(&o, "NULL");
            break;
        }
        case 'a': put_sa(&o, (const uint8_t *)(uintptr_t)v, (uint32_t)a[ai + 1]); break;
        case 'o': {
            const uint32_t *len = (const uint32_t *)(uintptr_t)a[ai + 1];
            if ((int64_t)r >= 0 && v && len) put(&o, "from="), put_sa(&o, (const uint8_t *)(uintptr_t)v, *len);
            else put(&o, "%#llx", (unsigned long long)v);
            break;
        }
        case 'S': put(&o, "len %llu", (unsigned long long)a[ai + 1]), put_hex(&o, (const uint8_t *)(uintptr_t)v, a[ai + 1]); break;
        case 'R':
            put(&o, "cap %llu", (unsigned long long)a[ai + 1]);
            if ((int64_t)r > 0) put_hex(&o, (const uint8_t *)(uintptr_t)v, r);
            break;
        case 'v': {
            uint32_t len = (uint32_t)a[ai + 1];
            if (v && len == 4 && mem_guest_readable(v, 4)) put(&o, "%d (4 bytes)", *(const int32_t *)(uintptr_t)v);
            else put(&o, "%#llx (%u bytes)", (unsigned long long)v, len);
            break;
        }
        case 'P': {
            const LanPollFd *p = (const LanPollFd *)(uintptr_t)v;
            put(&o, "{");
            for (uint64_t i = 0; p && i < a[ai + 1] && i < 8; i++)
                put(&o, "%s%d:%#x->%#x", i ? " " : "", p[i].fd, p[i].events, p[i].revents);
            put(&o, "%s}", a[ai + 1] > 8 ? " .." : "");
            break;
        }
        case 'm': {
            const GuestMsg *m = (const GuestMsg *)(uintptr_t)v;
            if (m && mem_guest_readable(v, sizeof *m)) {
                put(&o, "msg{name=");
                put_sa(&o, (const uint8_t *)(uintptr_t)m->name, m->namelen);
                put(&o, ", %d iov}", m->iovlen);
            } else {
                put(&o, "%#llx", (unsigned long long)v);
            }
            break;
        }
        case 'I': {
            const uint8_t *p = (const uint8_t *)(uintptr_t)v;
            if (p && r == 0) put(&o, "%u.%u.%u.%u", p[0], p[1], p[2], p[3]);
            else put(&o, "%#llx", (unsigned long long)v);
            break;
        }
        default: put(&o, "?"); break;
        }
    }
    char res[64];
    switch (f->ret) {
    case 'i':
        if ((int64_t)r < 0 && (int32_t)r == -1) snprintf(res, sizeof res, "-1 errno %d", err);
        else snprintf(res, sizeof res, "%lld", (long long)(int64_t)r);
        break;
    case 'r': snprintf(res, sizeof res, "%#x", (uint32_t)r); break;
    case 'b': snprintf(res, sizeof res, "%s", r & 1 ? "true" : "false"); break;
    case 'e': snprintf(res, sizeof res, "%d", (int32_t)r); break;
    case 'v': snprintf(res, sizeof res, "void"); break;
    default: snprintf(res, sizeof res, "%#llx", (unsigned long long)r); break;
    }
    char where[96];
    rt_log("net: +%.3f t%u %s(%s) = %s from %s", (now - log_t0) / 1000.0, plat_thread_id(), f->label, line, res,
           ldr_describe(lr - 4, where, sizeof where));
}

// ==== installation (boot.c rt_boot, before ldr_load)
void lan_install(void) {
    const char *spec = cfg_get("SPLAT3_LAN"), *lv = cfg_get("SPLAT3_NET_LOG");
    log_level = lv ? atoi(lv) : 0;
    LanCfg c;
    char err[256];
    if (!lan_parse(spec, cfg_get("SPLAT3_LAN_ROOM"), cfg_get("SPLAT3_LAN_NAME"), cfg_get("SPLAT3_LAN_BIND"), &c, err,
                   sizeof err))
        rt_fatal("SPLAT3_LAN=%s: %s", spec ? spec : "", err);
    if (c.mode == LAN_OFF && !log_level) return;   // the default: nothing added, ov_offline.c answers
    if (c.mode != LAN_OFF && !lan_net_start(&c, err, sizeof err)) rt_fatal("LAN play: %s", err);
    log_t0 = plat_now_ms();
    summary_at = (int64_t)log_t0 + 5000;
    hash_build();
    offline_errno = hostfn_override("nnsocketGetLastErrno");
    int added = 0;
    for (int i = 0; i < NFNS; i++) {
        NetFn *f = &fns[i];
        f->base = c.mode != LAN_OFF && f->lan ? f->lan : hostfn_override(f->name);
        if (!f->base) continue;
        if (log_level) hostfn_add_override(f->name, net_logger), added++;
        else if (c.mode != LAN_OFF && f->lan) hostfn_add_override(f->name, f->lan), added++;
    }
    rt_log("lan: %s; %d network functions answered here%s", c.mode == LAN_OFF ? "LAN play off (offline answers)"
                                                            : c.mode == LAN_LAN ? "lan mode" : "relay mode",
           added, log_level ? ", every call logged (SPLAT3_NET_LOG)" : "");
}

// lan_probe.c: the guest-facing function of a name, as the loader would bind it in LAN play.
LanGuestFn lan_guest_fn(const char *name) {
    for (int i = 0; i < NFNS; i++)
        if (!strcmp(fns[i].name, name)) return fns[i].lan;
    return NULL;
}
int lan_guest_errno(void) { return lan_errno; }
