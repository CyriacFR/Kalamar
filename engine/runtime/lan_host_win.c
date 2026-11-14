// LAN play's host sockets on Windows (lan.h): UDP over Winsock and the adapter list. ws2_32.dll and iphlpapi.dll are
// loaded from System32 the first time LAN play starts (lan_host_init), so the executable imports no networking library
// and a run without SPLAT3_LAN never loads one.
//
// Every socket is non-blocking (lan_net.c waits with lan_host_wait), may send broadcasts, and has SIO_UDP_CONNRESET
// off: without it an ICMP port-unreachable for an earlier datagram fails the next recvfrom with WSAECONNRESET, which a
// BSD stack never reports on an unconnected UDP socket.
#include <winsock2.h>
#include <ws2tcpip.h>
#include <iphlpapi.h>
#include <mstcpip.h>
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "lan.h"
#include "platform/platform.h"

#ifndef SIO_UDP_CONNRESET
#define SIO_UDP_CONNRESET _WSAIOW(IOC_VENDOR, 12)
#endif
#ifndef SIO_UDP_NETRESET
#define SIO_UDP_NETRESET _WSAIOW(IOC_VENDOR, 15)
#endif

struct LanHost { SOCKET s; uint16_t port; };

// Byte order without htons and friends, which ws2_32.dll exports: calling them would import it.
#define BE16(x) __builtin_bswap16((uint16_t)(x))
#define BE32(x) __builtin_bswap32((uint32_t)(x))

static struct {
    int (WSAAPI *startup)(WORD, LPWSADATA);
    int (WSAAPI *last_error)(void);
    SOCKET (WSAAPI *socket)(int, int, int);
    int (WSAAPI *closesocket)(SOCKET);
    int (WSAAPI *bind)(SOCKET, const struct sockaddr *, int);
    int (WSAAPI *sendto)(SOCKET, const char *, int, int, const struct sockaddr *, int);
    int (WSAAPI *recvfrom)(SOCKET, char *, int, int, struct sockaddr *, int *);
    int (WSAAPI *setsockopt)(SOCKET, int, int, const char *, int);
    int (WSAAPI *getsockname)(SOCKET, struct sockaddr *, int *);
    int (WSAAPI *ioctlsocket)(SOCKET, long, u_long *);
    int (WSAAPI *wsaioctl)(SOCKET, DWORD, LPVOID, DWORD, LPVOID, DWORD, LPDWORD, LPWSAOVERLAPPED,
                           LPWSAOVERLAPPED_COMPLETION_ROUTINE);
    int (WSAAPI *poll)(LPWSAPOLLFD, ULONG, INT);
    ULONG (WINAPI *adapters)(ULONG, ULONG, PVOID, PIP_ADAPTER_ADDRESSES, PULONG);
    int ready;
} ws;

#define SYM(lib, field, name)                                                                                      \
    do {                                                                                                           \
        *(void **)&ws.field = plat_lib_sym(lib, name);                                                             \
        if (!ws.field) return snprintf(err, cap, "%s has no %s", #lib, name), 0;                                   \
    } while (0)

int lan_host_init(char *err, size_t cap) {
    if (ws.ready) return 1;
    void *ws2_32 = plat_lib_open("ws2_32.dll"), *iphlpapi = plat_lib_open("iphlpapi.dll");
    if (!ws2_32 || !iphlpapi) return snprintf(err, cap, "cannot load %s", ws2_32 ? "iphlpapi.dll" : "ws2_32.dll"), 0;
    SYM(ws2_32, startup, "WSAStartup");
    SYM(ws2_32, last_error, "WSAGetLastError");
    SYM(ws2_32, socket, "socket");
    SYM(ws2_32, closesocket, "closesocket");
    SYM(ws2_32, bind, "bind");
    SYM(ws2_32, sendto, "sendto");
    SYM(ws2_32, recvfrom, "recvfrom");
    SYM(ws2_32, setsockopt, "setsockopt");
    SYM(ws2_32, getsockname, "getsockname");
    SYM(ws2_32, ioctlsocket, "ioctlsocket");
    SYM(ws2_32, wsaioctl, "WSAIoctl");
    SYM(ws2_32, poll, "WSAPoll");
    SYM(iphlpapi, adapters, "GetAdaptersAddresses");
    WSADATA d;
    int r = ws.startup(MAKEWORD(2, 2), &d);
    if (r) return snprintf(err, cap, "WSAStartup failed (%d)", r), 0;
    ws.ready = 1;
    return 1;
}

// Winsock error -> the guest's errno.
static int guest_errno(int e) {
    switch (e) {
    case WSAEWOULDBLOCK: return LE_AGAIN;
    case WSAEINTR: return LE_INTR;
    case WSAEBADF: case WSAENOTSOCK: return LE_BADF;
    case WSAEACCES: return LE_ACCES;
    case WSAEFAULT: return LE_FAULT;
    case WSAEINVAL: return LE_INVAL;
    case WSAEMFILE: return LE_MFILE;
    case WSAEMSGSIZE: return LE_MSGSIZE;
    case WSAEADDRINUSE: return LE_ADDRINUSE;
    case WSAEADDRNOTAVAIL: return LE_ADDRNOTAVAIL;
    case WSAENETDOWN: return LE_NETDOWN;
    case WSAENETUNREACH: return LE_NETUNREACH;
    case WSAEHOSTUNREACH: return LE_HOSTUNREACH;
    case WSAECONNRESET: return LE_CONNRESET;
    case WSAENOBUFS: return LE_NOBUFS;
    case WSAEISCONN: return LE_ISCONN;
    case WSAENOTCONN: return LE_NOTCONN;
    case WSAETIMEDOUT: return LE_TIMEDOUT;
    case WSAECONNREFUSED: return LE_CONNREFUSED;
    case WSAEAFNOSUPPORT: return LE_AFNOSUPPORT;
    default: return LE_NETDOWN;
    }
}

static struct sockaddr_in sin_of(uint32_t ip, uint16_t port) {
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = BE16(port);
    a.sin_addr.s_addr = BE32(ip);
    return a;
}

LanHost *lan_host_udp(uint32_t ip, uint16_t port, int *err) {
    if (!ws.ready) return *err = LE_NETDOWN, NULL;
    SOCKET s = ws.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (s == INVALID_SOCKET) return *err = guest_errno(ws.last_error()), NULL;
    u_long on = 1;
    BOOL off = FALSE, yes = TRUE;
    DWORD got = 0;
    // No SO_REUSEADDR: on Windows it lets a second socket take a bound unicast port, which FreeBSD's does not.
    struct sockaddr_in a = sin_of(ip, port);
    if (ws.ioctlsocket(s, FIONBIO, &on) || ws.setsockopt(s, SOL_SOCKET, SO_BROADCAST, (const char *)&yes, sizeof yes) ||
        ws.bind(s, (const struct sockaddr *)&a, sizeof a)) {
        *err = guest_errno(ws.last_error());
        ws.closesocket(s);
        return NULL;
    }
    ws.wsaioctl(s, SIO_UDP_CONNRESET, &off, sizeof off, NULL, 0, &got, NULL, NULL);
    ws.wsaioctl(s, SIO_UDP_NETRESET, &off, sizeof off, NULL, 0, &got, NULL, NULL);
    struct sockaddr_in b;
    int bl = sizeof b;
    if (ws.getsockname(s, (struct sockaddr *)&b, &bl)) {
        *err = guest_errno(ws.last_error());
        ws.closesocket(s);
        return NULL;
    }
    LanHost *h = calloc(1, sizeof *h);
    if (!h) {
        ws.closesocket(s);
        return *err = LE_NOMEM, NULL;
    }
    h->s = s;
    h->port = BE16(b.sin_port);
    return h;
}

uint16_t lan_host_port(LanHost *h) { return h ? h->port : 0; }

long lan_host_sendto(LanHost *h, const void *p, size_t n, uint32_t ip, uint16_t port, int *err) {
    struct sockaddr_in a = sin_of(ip, port);
    if (n > 65507) return *err = LE_MSGSIZE, -1;
    int r = ws.sendto(h->s, (const char *)p, (int)n, 0, (const struct sockaddr *)&a, sizeof a);
    if (r == SOCKET_ERROR) return *err = guest_errno(ws.last_error()), -1;
    return r;
}

long lan_host_recvfrom(LanHost *h, void *p, size_t n, uint32_t *ip, uint16_t *port, int *trunc, int *err) {
    struct sockaddr_in a;
    int al = sizeof a;
    *trunc = 0;
    int r = ws.recvfrom(h->s, (char *)p, n > 0x7FFFFFFF ? 0x7FFFFFFF : (int)n, 0, (struct sockaddr *)&a, &al);
    if (r == SOCKET_ERROR) {
        int e = ws.last_error();
        if (e != WSAEMSGSIZE) return *err = guest_errno(e), -1;
        *trunc = 1;   // Winsock keeps the first n bytes and drops the rest, as BSD does with MSG_TRUNC
        r = (int)n;
    }
    *ip = BE32(a.sin_addr.s_addr);
    *port = BE16(a.sin_port);
    return r;
}

int lan_host_wait(LanHost *const *h, int n, int *ready, int timeout_ms) {
    WSAPOLLFD f[64];
    if (n > 64) n = 64;
    for (int i = 0; i < n; i++) f[i] = (WSAPOLLFD){.fd = h[i]->s, .events = POLLRDNORM};
    int r = n ? ws.poll(f, (ULONG)n, timeout_ms) : (Sleep(timeout_ms < 0 ? INFINITE : (DWORD)timeout_ms), 0);
    int count = 0;
    for (int i = 0; i < n; i++) count += ready[i] = r > 0 && (f[i].revents & (POLLRDNORM | POLLERR | POLLHUP)) != 0;
    return count;
}

int lan_host_setbuf(LanHost *h, int rcv, int bytes) {
    return ws.setsockopt(h->s, SOL_SOCKET, rcv ? SO_RCVBUF : SO_SNDBUF, (const char *)&bytes, sizeof bytes) == 0;
}

int lan_host_broadcast(LanHost *h, int on) {
    BOOL v = on ? TRUE : FALSE;
    return ws.setsockopt(h->s, SOL_SOCKET, SO_BROADCAST, (const char *)&v, sizeof v) == 0;
}

void lan_host_close(LanHost *h) {
    if (!h) return;
    ws.closesocket(h->s);
    free(h);
}

// ---- adapters
typedef int (*AdapterFn)(void *ctx, const IP_ADAPTER_ADDRESSES *a, uint32_t ip, uint32_t mask, uint32_t gw);

static void each_address(AdapterFn fn, void *ctx) {
    ULONG size = 16 << 10;
    IP_ADAPTER_ADDRESSES *list = NULL;
    for (int tries = 0; tries < 4; tries++) {
        free(list);
        list = malloc(size);
        if (!list) return;
        ULONG r = ws.adapters(AF_INET, GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST |
                                           GAA_FLAG_SKIP_DNS_SERVER, NULL, list, &size);
        if (r == ERROR_SUCCESS) break;
        if (r != ERROR_BUFFER_OVERFLOW) {
            free(list);
            return;
        }
    }
    for (IP_ADAPTER_ADDRESSES *a = list; a; a = a->Next) {
        if (a->OperStatus != IfOperStatusUp) continue;
        uint32_t gw = 0;
        for (IP_ADAPTER_GATEWAY_ADDRESS_LH *g = a->FirstGatewayAddress; g && !gw; g = g->Next)
            if (g->Address.lpSockaddr->sa_family == AF_INET)
                gw = BE32(((struct sockaddr_in *)g->Address.lpSockaddr)->sin_addr.s_addr);
        for (IP_ADAPTER_UNICAST_ADDRESS_LH *u = a->FirstUnicastAddress; u; u = u->Next) {
            if (u->Address.lpSockaddr->sa_family != AF_INET) continue;
            uint32_t ip = BE32(((struct sockaddr_in *)u->Address.lpSockaddr)->sin_addr.s_addr);
            if (fn(ctx, a, ip, lan_prefix_mask(u->OnLinkPrefixLength), gw)) {
                free(list);
                return;
            }
        }
    }
    free(list);
}

typedef struct { uint32_t ip, mask, gw; int score; char desc[96]; } Pick;

static int pick_one(void *ctx, const IP_ADAPTER_ADDRESSES *a, uint32_t ip, uint32_t mask, uint32_t gw) {
    Pick *p = ctx;
    char why[128];
    if (a->IfType == IF_TYPE_SOFTWARE_LOOPBACK || !lan_net_ok(ip, mask, why, sizeof why) || (ip >> 24) == 127) return 0;
    // An adapter with a default gateway is the one the LAN is on; Ethernet and Wi-Fi before virtual switches.
    int score = (gw ? 4 : 0) + (a->IfType == IF_TYPE_ETHERNET_CSMACD || a->IfType == IF_TYPE_IEEE80211 ? 2 : 0) +
                (wcsstr(a->Description, L"Virtual") || wcsstr(a->Description, L"Hyper-V") ? 0 : 1);
    if (score <= p->score) return 0;
    p->ip = ip, p->mask = mask, p->gw = gw, p->score = score;
    snprintf(p->desc, sizeof p->desc, "%ls", a->FriendlyName ? a->FriendlyName : L"?");
    return 0;
}

int lan_host_detect(uint32_t *ip, uint32_t *mask, uint32_t *gateway, char *desc, size_t cap) {
    Pick p = {.score = -1};
    if (!ws.ready) return 0;
    each_address(pick_one, &p);
    if (p.score < 0) return 0;
    *ip = p.ip, *mask = p.mask, *gateway = p.gw;
    snprintf(desc, cap, "%s", p.desc);
    return 1;
}

typedef struct { uint32_t ip, mask, gw; int found; } Find;

static int find_one(void *ctx, const IP_ADAPTER_ADDRESSES *a, uint32_t ip, uint32_t mask, uint32_t gw) {
    Find *f = ctx;
    (void)a;
    if (ip != f->ip) return 0;
    f->mask = mask, f->gw = gw, f->found = 1;
    return 1;
}

int lan_host_find(uint32_t ip, uint32_t *mask, uint32_t *gateway) {
    Find f = {.ip = ip};
    if (!ws.ready) return 0;
    each_address(find_one, &f);
    if (f.found) *mask = f.mask, *gateway = f.gw;
    return f.found;
}
