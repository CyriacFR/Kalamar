// LAN play's decisions without OS calls (lan.h): what SPLAT3_LAN asks for, which destinations the guest may reach,
// and the relay protocol's datagrams (docs/lan-relay-protocol.md). lan_probe.c tests every function here.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "lan.h"

// ---- addresses
const char *lan_ip_str(uint32_t ip, char out[16]) {
    snprintf(out, 16, "%u.%u.%u.%u", ip >> 24, (ip >> 16) & 255, (ip >> 8) & 255, ip & 255);
    return out;
}

int lan_ip_parse(const char *s, uint32_t *ip) {
    uint32_t v = 0;
    for (int part = 0; part < 4; part++) {
        if (*s < '0' || *s > '9') return 0;
        unsigned n = 0;
        int digits = 0;
        while (*s >= '0' && *s <= '9') {
            n = n * 10 + (unsigned)(*s++ - '0');
            if (++digits > 3 || n > 255) return 0;
        }
        v = v << 8 | n;
        if (part < 3 && *s++ != '.') return 0;
    }
    if (*s) return 0;
    *ip = v;
    return 1;
}

uint32_t lan_prefix_mask(int prefix) { return prefix <= 0 ? 0 : prefix >= 32 ? 0xFFFFFFFFu : ~0u << (32 - prefix); }

int lan_mask_prefix(uint32_t mask) {
    int p = 0;
    while (p < 32 && (mask & (0x80000000u >> p))) p++;
    return lan_prefix_mask(p) == mask ? p : -1;
}

// The blocks a lan-mode interface may sit in, and the shortest prefix each allows.
static const struct { uint32_t net; int prefix; } private_blocks[] = {
    {0x0A000000u, 8},    // 10.0.0.0/8
    {0xAC100000u, 12},   // 172.16.0.0/12
    {0xC0A80000u, 16},   // 192.168.0.0/16
    {0xA9FE0000u, 16},   // 169.254.0.0/16 (link-local: a LAN without DHCP)
    {0x7F000000u, 8},    // 127.0.0.0/8 (loopback: two instances on one PC)
};

static int block_of(uint32_t ip) {
    for (int i = 0; i < (int)(sizeof private_blocks / sizeof *private_blocks); i++)
        if ((ip & lan_prefix_mask(private_blocks[i].prefix)) == private_blocks[i].net) return i;
    return -1;
}

int lan_private(uint32_t ip) { return block_of(ip) >= 0; }

int lan_net_ok(uint32_t ip, uint32_t mask, char *err, size_t cap) {
    char a[16];
    int prefix = lan_mask_prefix(mask), b = block_of(ip);
    if (prefix < 0) return snprintf(err, cap, "the subnet mask is not contiguous"), 0;
    if (b < 0) return snprintf(err, cap, "%s is not a private address (10/8, 172.16/12, 192.168/16, 169.254/16, 127/8)",
                               lan_ip_str(ip, a)), 0;
    if (prefix < private_blocks[b].prefix || prefix > 30)
        return snprintf(err, cap, "/%d: the subnet must lie inside the private block and be /30 or smaller", prefix), 0;
    if ((ip & ~mask) == 0 || (ip | mask) == 0xFFFFFFFFu)
        return snprintf(err, cap, "%s is the subnet's network or broadcast address", lan_ip_str(ip, a)), 0;
    return 1;
}

// ---- configuration
static int word(const char *s, const char *w) {
    for (; *w; s++, w++)
        if ((*s | 0x20) != *w) return 0;
    return !*s;
}

int lan_parse(const char *spec, const char *room, const char *name, const char *bind, LanCfg *out, char *err,
              size_t cap) {
    memset(out, 0, sizeof *out);
    if (!spec || !*spec || word(spec, "0") || word(spec, "off") || word(spec, "no") || word(spec, "false")) return 1;
    if (word(spec, "lan")) {
        out->mode = LAN_LAN;
        out->detect = 1;
    } else if (!strncmp(spec, "lan:", 4)) {
        char ip[32];
        const char *slash = strchr(spec + 4, '/');
        size_t n = slash ? (size_t)(slash - (spec + 4)) : 0;
        char *end;
        long prefix = slash ? strtol(slash + 1, &end, 10) : -1;
        if (!slash || n >= sizeof ip || !slash[1] || *end || prefix < 1 || prefix > 32)
            return snprintf(err, cap, "lan:<address>/<prefix> expected, as lan:192.168.1.20/24"), 0;
        memcpy(ip, spec + 4, n);
        ip[n] = 0;
        out->mode = LAN_LAN;
        if (!lan_ip_parse(ip, &out->ip)) return snprintf(err, cap, "'%s' is not an IPv4 address", ip), 0;
        out->mask = lan_prefix_mask((int)prefix);
        if (!lan_net_ok(out->ip, out->mask, err, cap)) return 0;
    } else if (!strncmp(spec, "relay:", 6)) {
        char ip[32];
        const char *colon = strchr(spec + 6, ':');
        size_t n = colon ? (size_t)(colon - (spec + 6)) : strlen(spec + 6);
        if (n >= sizeof ip) return snprintf(err, cap, "relay:<IPv4 address>[:<port>] expected"), 0;
        memcpy(ip, spec + 6, n);
        ip[n] = 0;
        out->mode = LAN_RELAY;
        out->relay_port = LAN_RELAY_DEFAULT_PORT;
        if (!lan_ip_parse(ip, &out->relay_ip))
            return snprintf(err, cap, "relay '%s': an IPv4 address is needed (the runtime resolves no names)", ip), 0;
        if (colon) {
            char *end;
            long port = strtol(colon + 1, &end, 10);
            if (!colon[1] || *end || port < 1 || port > 65535) return snprintf(err, cap, "relay port '%s'", colon + 1), 0;
            // a relay is never a DNS, DHCP, NTP, NetBIOS, SSDP or mDNS service: the HELLOs go to no such port
            if (lan_port_blocked((uint16_t)port))
                return snprintf(err, cap, "relay port %ld is a network service's (lan_port_blocked)", port), 0;
            out->relay_port = (uint16_t)port;
        }
        uint32_t r = out->relay_ip;
        if (r == 0 || r == 0xFFFFFFFFu || (r >> 28) == 0xE || (r >> 24) == 0)
            return snprintf(err, cap, "relay '%s' is not a unicast address", ip), 0;
        const char *rm = room && *room ? room : "DEFAULT";
        size_t rl = strlen(rm);
        if (rl > 32) return snprintf(err, cap, "SPLAT3_LAN_ROOM: 32 characters at most"), 0;
        for (size_t i = 0; i < rl; i++) {
            char ch = rm[i];
            int ok = (ch >= 'A' && ch <= 'Z') || (ch >= 'a' && ch <= 'z') || (ch >= '0' && ch <= '9') || ch == '-' ||
                     ch == '_';
            if (!ok) return snprintf(err, cap, "SPLAT3_LAN_ROOM: letters, digits, '-' and '_' only"), 0;
            out->room[i] = (char)(ch >= 'a' && ch <= 'z' ? ch - 32 : ch);
        }
        size_t nl = name ? strlen(name) : 0;
        if (nl > 32) return snprintf(err, cap, "SPLAT3_LAN_NAME: 32 bytes at most"), 0;
        for (size_t i = 0; i < nl; i++)
            if ((unsigned char)name[i] < 0x20 || name[i] == 0x7F)
                return snprintf(err, cap, "SPLAT3_LAN_NAME: no control characters"), 0;
        if (nl) memcpy(out->name, name, nl);
    } else {
        return snprintf(err, cap, "'%s': lan, lan:<address>/<prefix>, relay:<address>[:<port>] or off", spec), 0;
    }
    if (bind && *bind) {
        if (out->mode != LAN_LAN || !word(bind, "any"))
            return snprintf(err, cap, "SPLAT3_LAN_BIND: only 'any', in lan mode"), 0;
        out->bind_any = 1;
    }
    return 1;
}

// ---- allowlist
int lan_port_blocked(uint16_t port) {
    static const uint16_t ports[] = {53, 67, 68, 123, 137, 138, 139, 1900, 5350, 5351, 5353, 5355};
    for (size_t i = 0; i < sizeof ports / sizeof *ports; i++)
        if (port == ports[i]) return 1;
    return 0;
}

int lan_dst_check(const LanCfg *c, uint32_t dst, uint16_t port, uint32_t *wire, const char **why) {
    *wire = 0;
    if (!c || c->mode == LAN_OFF) return *why = "LAN play is off", 0;
    if (!c->ip || !c->mask) return *why = c->mode == LAN_RELAY ? "not joined to the relay" : "no interface", 0;
    if (port == 0) return *why = "port 0", 0;
    if (lan_port_blocked(port)) return *why = "a service port (DNS, DHCP, NTP, NetBIOS, SSDP, NAT-PMP, mDNS)", 0;
    if ((dst >> 28) == 0xE) return *why = "multicast", 0;
    uint32_t net = c->ip & c->mask, bcast = c->ip | ~c->mask;
    if (c->mode == LAN_RELAY && (net != LAN_VNET || c->mask != LAN_VNET_MASK))
        return *why = "the virtual address is outside 10.13.0.0/16", 0;
    if (dst == 0xFFFFFFFFu || dst == bcast) {
        *wire = bcast;
        return LAN_DST_BROADCAST;
    }
    if ((dst & c->mask) != net) return *why = c->mode == LAN_RELAY ? "outside the virtual subnet" : "outside the LAN subnet", 0;
    if (dst == net) return *why = "the subnet's network address", 0;
    if (c->mode == LAN_LAN) {
        if (!lan_private(dst)) return *why = "not a private address", 0;
        if (c->gateway && dst == c->gateway) return *why = "the default gateway", 0;
    }
    *wire = dst;
    return dst == c->ip ? LAN_DST_SELF : LAN_DST_UNICAST;
}

int lan_src_ok(const LanCfg *c, uint32_t src) {
    if (!c || c->mode == LAN_OFF || !c->ip || !c->mask) return 0;
    uint32_t net = c->ip & c->mask;
    if ((src & c->mask) != net || src == net || src == (c->ip | ~c->mask)) return 0;
    return c->mode == LAN_RELAY ? net == LAN_VNET : lan_private(src);
}

// ---- relay codec
static uint8_t *put8(uint8_t *p, uint8_t v) { *p = v; return p + 1; }
static uint8_t *put16(uint8_t *p, uint16_t v) { p[0] = (uint8_t)(v >> 8), p[1] = (uint8_t)v; return p + 2; }
static uint8_t *put32(uint8_t *p, uint32_t v) { return put16(put16(p, (uint16_t)(v >> 16)), (uint16_t)v); }
static uint8_t *put64(uint8_t *p, uint64_t v) { return put32(put32(p, (uint32_t)(v >> 32)), (uint32_t)v); }
static uint8_t *putb(uint8_t *p, const void *s, size_t n) { if (n) memcpy(p, s, n); return p + n; }
static uint16_t get16(const uint8_t *p) { return (uint16_t)(p[0] << 8 | p[1]); }
static uint32_t get32(const uint8_t *p) { return (uint32_t)get16(p) << 16 | get16(p + 2); }
static uint64_t get64(const uint8_t *p) { return (uint64_t)get32(p) << 32 | get32(p + 4); }

static uint8_t *header(uint8_t *p, uint8_t type) {
    p = putb(p, "S3LR", 4);
    p = put8(p, LR_VERSION);
    p = put8(p, type);
    return put16(p, 0);
}

static uint8_t *session(uint8_t *p, uint8_t type, uint32_t s, const uint8_t token[8]) {
    return putb(put32(header(p, type), s), token, 8);
}

size_t lr_hello(uint8_t *out, uint64_t nonce) {
    memset(out, 0, LR_HELLO_SIZE);
    put64(header(out, LR_HELLO), nonce);
    return LR_HELLO_SIZE;
}

size_t lr_challenge(uint8_t *out, uint64_t nonce, const uint8_t cookie[16]) {
    return (size_t)(putb(put64(header(out, LR_CHALLENGE), nonce), cookie, 16) - out);
}

size_t lr_join(uint8_t *out, uint64_t nonce, const uint8_t cookie[16], uint32_t want_vip, const char *room,
               const char *name) {
    size_t rl = strlen(room), nl = name ? strlen(name) : 0;
    if (rl > 32) rl = 32;
    if (nl > 32) nl = 32;
    memset(out, 0, 128);
    uint8_t *p = putb(put64(header(out, LR_JOIN), nonce), cookie, 16);
    p = put32(p, want_vip);
    p = put8(p, (uint8_t)rl);
    p = put8(p, (uint8_t)nl);
    p = put16(p, 0);
    p = putb(p, room, rl);
    p = putb(p, name, nl);
    size_t n = (size_t)(p - out);
    return n < LR_JOIN_MIN ? LR_JOIN_MIN : n;
}

size_t lr_join_ack(uint8_t *out, uint64_t nonce, uint32_t s, const uint8_t token[8], uint32_t vip, uint8_t prefix,
                   uint8_t max_members, uint8_t keepalive_s, uint8_t idle_s, uint16_t max_payload) {
    uint8_t *p = putb(put32(put64(header(out, LR_JOIN_ACK), nonce), s), token, 8);
    p = put32(p, vip);
    p = put8(put8(put8(put8(p, prefix), max_members), keepalive_s), idle_s);
    p = put16(put16(p, max_payload), 0);
    return (size_t)(p - out);
}

size_t lr_join_nak(uint8_t *out, uint64_t nonce, uint8_t reason, uint8_t detail) {
    uint8_t *p = put8(put8(put64(header(out, LR_JOIN_NAK), nonce), reason), detail);
    memset(p, 0, 6);
    return (size_t)(p + 6 - out);
}

size_t lr_keepalive(uint8_t *out, uint32_t s, const uint8_t token[8], uint32_t seq) {
    return (size_t)(put32(put32(session(out, LR_KEEPALIVE, s, token), seq), 0) - out);
}

size_t lr_keepalive_ack(uint8_t *out, uint32_t s, const uint8_t token[8], uint32_t seq, uint8_t members) {
    uint8_t *p = put8(put32(session(out, LR_KEEPALIVE_ACK, s, token), seq), members);
    memset(p, 0, 3);
    return (size_t)(p + 3 - out);
}

size_t lr_leave(uint8_t *out, uint32_t s, const uint8_t token[8]) {
    return (size_t)(session(out, LR_LEAVE, s, token) - out);
}

size_t lr_closed(uint8_t *out, uint32_t s, const uint8_t token[8], uint8_t reason) {
    uint8_t *p = put8(session(out, LR_CLOSED, s, token), reason);
    memset(p, 0, 3);
    return (size_t)(p + 3 - out);
}

size_t lr_member(uint8_t *out, uint32_t s, const uint8_t token[8], uint8_t event, uint32_t vip, const char *name) {
    size_t nl = name ? strlen(name) : 0;
    if (nl > 32) nl = 32;
    uint8_t *p = put16(put8(put8(session(out, LR_MEMBER, s, token), event), (uint8_t)nl), 0);
    return (size_t)(putb(put32(p, vip), name, nl) - out);
}

size_t lr_data(uint8_t *out, uint32_t s, const uint8_t token[8], uint32_t src_ip, uint16_t src_port, uint32_t dst_ip,
               uint16_t dst_port, const void *payload, size_t len) {
    if (len > LR_MAX_PAYLOAD) return 0;
    uint8_t *p = session(out, LR_DATA, s, token);
    p = put16(put32(p, src_ip), src_port);
    p = put16(put32(p, dst_ip), dst_port);
    return (size_t)(putb(p, payload, len) - out);
}

static int text_ok(const uint8_t *p, size_t n, int room) {
    for (size_t i = 0; i < n; i++) {
        uint8_t ch = p[i];
        if (room ? !((ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == '-' || ch == '_')
                 : (ch < 0x20 || ch == 0x7F))
            return 0;
    }
    return 1;
}

int lr_parse(const uint8_t *b, size_t n, LrMsg *m) {
    memset(m, 0, sizeof *m);
    if (n < 8 || n > LR_MAX_DATAGRAM || memcmp(b, "S3LR", 4)) return 0;
    m->version = b[4];
    m->type = b[5];
    if (m->version != LR_VERSION) return 0;
    const uint8_t *p = b + 8;
    switch (m->type) {
    case LR_HELLO:
        if (n < LR_HELLO_SIZE) return 0;
        m->nonce = get64(p);
        return 1;
    case LR_CHALLENGE:
        if (n < 32) return 0;
        m->nonce = get64(p);
        memcpy(m->cookie, p + 8, 16);
        return 1;
    case LR_JOIN: {
        if (n < LR_JOIN_MIN) return 0;
        m->nonce = get64(p);
        memcpy(m->cookie, p + 8, 16);
        m->vip = get32(p + 24);
        size_t rl = p[28], nl = p[29];
        if (rl < 1 || rl > 32 || nl > 32 || 40 + rl + nl > n) return 0;
        if (!text_ok(p + 32, rl, 1) || !text_ok(p + 32 + rl, nl, 0)) return 0;
        memcpy(m->room, p + 32, rl);
        memcpy(m->name, p + 32 + rl, nl);
        return 1;
    }
    case LR_JOIN_ACK:
        if (n < 40) return 0;
        m->nonce = get64(p);
        m->session = get32(p + 8);
        memcpy(m->token, p + 12, 8);
        m->vip = get32(p + 20);
        m->prefix = p[24], m->max_members = p[25], m->keepalive_s = p[26], m->idle_s = p[27];
        m->max_payload = get16(p + 28);
        return 1;
    case LR_JOIN_NAK:
        if (n < 24) return 0;
        m->nonce = get64(p);
        m->reason = p[8];
        m->detail = p[9];   // a reserved byte up to relay 1.1.12, zero: "not said"
        return 1;
    default:
        break;
    }
    // session messages
    if (n < 20) return 0;
    m->session = get32(p);
    memcpy(m->token, p + 4, 8);
    p += 12;
    switch (m->type) {
    case LR_KEEPALIVE:
        if (n < 28) return 0;
        m->seq = get32(p);
        return 1;
    case LR_KEEPALIVE_ACK:
        if (n < 28) return 0;
        m->seq = get32(p);
        m->members = p[4];
        return 1;
    case LR_LEAVE:
        return 1;
    case LR_CLOSED:
        if (n < 24) return 0;
        m->reason = p[0];
        return 1;
    case LR_MEMBER: {
        if (n < 28) return 0;
        size_t nl = p[1];
        if (nl > 32 || 28 + nl > n || !text_ok(p + 8, nl, 0)) return 0;
        m->reason = p[0];
        m->vip = get32(p + 4);
        memcpy(m->name, p + 8, nl);
        return 1;
    }
    case LR_DATA:
        if (n < LR_DATA_HEADER) return 0;
        m->src_ip = get32(p), m->src_port = get16(p + 4);
        m->dst_ip = get32(p + 6), m->dst_port = get16(p + 10);
        m->payload = b + LR_DATA_HEADER;
        m->len = n - LR_DATA_HEADER;
        return 1;
    default:
        return 0;
    }
}
