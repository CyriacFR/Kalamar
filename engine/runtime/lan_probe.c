// LAN play's probes (lan.h). No game files.
//   --probe-lan        SPLAT3_LAN parsing; the allowlist, with every mode refusing public addresses; the relay codec;
//                      then the guest ABI end to end in lan mode on the loopback interface (127.0.0.2/8): sockets,
//                      sockaddr layouts, errno values, broadcast, poll, msghdr, nifm, name lookups
//   --probe-lan-peer   one of two (or more) processes started with the same SPLAT3_LAN family of settings: acts as
//                      the session library's LAN module does (sockets on UDP 49152 and 35000, broadcast discovery, unicast replies)
//                      through the guest-facing functions, and passes once it has seen another peer's broadcast and
//                      three unicast round trips. SPLAT3_PEER_SECONDS (default 20) bounds the run.
// The guest-facing functions run on a Cpu of this thread, as the loader would call them; arguments are host pointers,
// which guest code also uses (guest addresses are host addresses).
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cfg.h"
#include "host.h"
#include "lan.h"
#include "platform/platform.h"

static int failures;
#define CHECK(cond, ...)                                                                                              \
    do {                                                                                                              \
        if (!(cond)) {                                                                                                \
            failures++;                                                                                               \
            printf("  FAIL %s:%d: ", __FILE__, __LINE__);                                                             \
            printf(__VA_ARGS__);                                                                                      \
            printf("\n");                                                                                             \
        }                                                                                                             \
    } while (0)

static uint32_t ip4(int a, int b, int c, int d) { return (uint32_t)a << 24 | (uint32_t)b << 16 | (uint32_t)c << 8 | (uint32_t)d; }

// ---- configuration and allowlist
static void check_parse(void) {
    LanCfg c;
    char err[256];
    CHECK(lan_parse(NULL, NULL, NULL, NULL, &c, err, sizeof err) && c.mode == LAN_OFF, "unset is off");
    CHECK(lan_parse("off", NULL, NULL, NULL, &c, err, sizeof err) && c.mode == LAN_OFF, "off");
    CHECK(lan_parse("0", NULL, NULL, NULL, &c, err, sizeof err) && c.mode == LAN_OFF, "0");
    CHECK(lan_parse("lan", NULL, NULL, NULL, &c, err, sizeof err) && c.mode == LAN_LAN && c.detect, "lan");
    CHECK(lan_parse("lan:192.168.1.23/24", NULL, NULL, NULL, &c, err, sizeof err) && c.ip == ip4(192, 168, 1, 23) &&
          c.mask == 0xFFFFFF00u, "lan with an address");
    CHECK(lan_parse("lan:127.0.0.2/8", NULL, NULL, "any", &c, err, sizeof err) && c.bind_any, "loopback, bind any");
    static const char *const bad[] = {"lan:8.8.8.8/24", "lan:192.168.1.23/15", "lan:10.0.0.1/7", "lan:192.168.1.0/24",
                                      "lan:192.168.1.255/24", "lan:100.64.0.1/16", "lan:1.2.3.4/8", "lan:192.168.1.23",
                                      "lan:192.168.1.256/24", "lan:172.32.0.1/16", "relay:example.com:47313",
                                      "relay:", "relay:1.2.3.4:0", "relay:1.2.3.4:70000", "relay:0.0.0.0",
                                      "relay:127.0.0.1:53", "relay:192.168.1.1:123", "relay:10.0.0.1:1900",
                                      "relay:255.255.255.255", "relay:224.0.0.1", "wan", "on", "1"};
    for (size_t i = 0; i < sizeof bad / sizeof *bad; i++)
        CHECK(!lan_parse(bad[i], NULL, NULL, NULL, &c, err, sizeof err), "'%s' accepted", bad[i]);
    CHECK(lan_parse("relay:203.0.113.9", "abc-1_Z", "Player 1", NULL, &c, err, sizeof err) && c.mode == LAN_RELAY &&
          c.relay_ip == ip4(203, 0, 113, 9) && c.relay_port == LAN_RELAY_DEFAULT_PORT && !strcmp(c.room, "ABC-1_Z") &&
          !strcmp(c.name, "Player 1"), "relay with room and name: %s", err);
    CHECK(lan_parse("relay:127.0.0.1:5000", NULL, NULL, NULL, &c, err, sizeof err) && c.relay_port == 5000 &&
          !strcmp(c.room, "DEFAULT"), "relay default room");
    CHECK(!lan_parse("relay:127.0.0.1", "bad room!", NULL, NULL, &c, err, sizeof err), "room with a space");
    CHECK(!lan_parse("relay:127.0.0.1", "0123456789012345678901234567890123", NULL, NULL, &c, err, sizeof err),
          "33-character room");
    CHECK(!lan_parse("relay:127.0.0.1", NULL, "a\nb", NULL, &c, err, sizeof err), "name with a newline");
    CHECK(!lan_parse("relay:127.0.0.1", NULL, NULL, "any", &c, err, sizeof err), "bind any in relay mode");
    printf("lan: SPLAT3_LAN parsing: %s\n", failures ? "FAIL" : "ok");
}

// Public addresses (DNS resolvers, CDNs, cloud ranges, documentation ranges) and ports a game service would use.
static const uint32_t publics[] = {0x08080808u, 0x01010101u, 0x09090909u, 0x344F0001u, 0x68100001u, 0xCB007109u,
                                   0x97650101u, 0x17200001u, 0x23C00001u, 0xC6336401u, 0x0D200001u, 0x64400001u,
                                   0xAC200001u, 0xC0A90001u, 0x0B000001u, 0xFFFFFFFEu};
static const uint16_t ports[] = {1, 53, 80, 123, 443, 3478, 5228, 8080, 35000, 40000, 49152, 49153, 65535};

static void check_allowlist(void) {
    int before = failures;
    LanCfg off = {0}, lan = {.mode = LAN_LAN, .ip = ip4(192, 168, 1, 23), .mask = 0xFFFFFF00u, .gateway = ip4(192, 168, 1, 1)};
    LanCfg lan10 = {.mode = LAN_LAN, .ip = ip4(10, 0, 0, 5), .mask = 0xFF000000u};
    LanCfg loop = {.mode = LAN_LAN, .ip = ip4(127, 0, 0, 2), .mask = 0xFF000000u};
    LanCfg relay = {.mode = LAN_RELAY, .ip = ip4(10, 13, 0, 7), .mask = LAN_VNET_MASK, .relay_ip = ip4(203, 0, 113, 9),
                    .relay_port = 47313};
    LanCfg relay_unjoined = {.mode = LAN_RELAY, .relay_ip = ip4(203, 0, 113, 9), .relay_port = 47313};
    LanCfg relay_bad = {.mode = LAN_RELAY, .ip = ip4(8, 8, 0, 7), .mask = 0xFFFF0000u};   // a hostile JOIN_ACK
    const LanCfg *modes[] = {&off, &lan, &lan10, &loop, &relay, &relay_unjoined, &relay_bad};
    const char *names[] = {"off", "lan 192.168.1.23/24", "lan 10.0.0.5/8", "lan 127.0.0.2/8", "relay 10.13.0.7",
                           "relay not joined", "relay outside 10.13/16"};
    uint32_t wire;
    const char *why;
    int checked = 0;
    // The proof: no mode lets a public address through, on any port, and none accepts one as a source.
    for (size_t m = 0; m < sizeof modes / sizeof *modes; m++)
        for (size_t i = 0; i < sizeof publics / sizeof *publics; i++) {
            for (size_t p = 0; p < sizeof ports / sizeof *ports; p++, checked++)
                CHECK(!lan_dst_check(modes[m], publics[i], ports[p], &wire, &why) && wire == 0,
                      "%s: public %08x:%u allowed", names[m], publics[i], ports[p]);
            CHECK(!lan_src_ok(modes[m], publics[i]), "%s: public source %08x accepted", names[m], publics[i]);
        }
    // The relay's own (public) address is not a destination the guest can name either.
    CHECK(!lan_dst_check(&relay, relay.relay_ip, 47313, &wire, &why), "relay mode: the relay address itself");
    // What each mode does allow.
    CHECK(lan_dst_check(&lan, ip4(192, 168, 1, 40), 49152, &wire, &why) == LAN_DST_UNICAST && wire == ip4(192, 168, 1, 40),
          "lan: a LAN peer");
    CHECK(lan_dst_check(&lan, 0xFFFFFFFFu, 35000, &wire, &why) == LAN_DST_BROADCAST && wire == ip4(192, 168, 1, 255),
          "lan: 255.255.255.255 becomes the directed broadcast");
    CHECK(lan_dst_check(&lan, ip4(192, 168, 1, 255), 35000, &wire, &why) == LAN_DST_BROADCAST, "lan: directed broadcast");
    CHECK(lan_dst_check(&lan, ip4(192, 168, 1, 23), 35000, &wire, &why) == LAN_DST_SELF, "lan: itself");
    CHECK(!lan_dst_check(&lan, ip4(192, 168, 1, 1), 49152, &wire, &why), "lan: the gateway");
    CHECK(!lan_dst_check(&lan, ip4(192, 168, 1, 0), 49152, &wire, &why), "lan: the network address");
    CHECK(!lan_dst_check(&lan, ip4(192, 168, 2, 9), 49152, &wire, &why), "lan: another private subnet");
    CHECK(!lan_dst_check(&lan, ip4(192, 168, 1, 40), 53, &wire, &why), "lan: DNS to a LAN host");
    CHECK(!lan_dst_check(&lan, ip4(192, 168, 1, 40), 5351, &wire, &why), "lan: NAT-PMP");
    CHECK(!lan_dst_check(&lan, ip4(239, 255, 255, 250), 1900, &wire, &why), "lan: SSDP multicast");
    CHECK(!lan_dst_check(&lan, ip4(224, 0, 0, 1), 35000, &wire, &why), "lan: multicast");
    CHECK(!lan_dst_check(&lan, ip4(192, 168, 1, 40), 0, &wire, &why), "lan: port 0");
    CHECK(!lan_dst_check(&lan, ip4(127, 0, 0, 1), 49152, &wire, &why), "lan: loopback from a LAN interface");
    CHECK(lan_dst_check(&loop, ip4(127, 0, 0, 3), 49152, &wire, &why) == LAN_DST_UNICAST, "loopback: the other instance");
    CHECK(lan_dst_check(&loop, 0xFFFFFFFFu, 35000, &wire, &why) == LAN_DST_BROADCAST && wire == ip4(127, 255, 255, 255),
          "loopback: broadcast");
    CHECK(lan_dst_check(&relay, ip4(10, 13, 0, 9), 49152, &wire, &why) == LAN_DST_UNICAST, "relay: a member");
    CHECK(lan_dst_check(&relay, 0xFFFFFFFFu, 35000, &wire, &why) == LAN_DST_BROADCAST && wire == ip4(10, 13, 255, 255),
          "relay: broadcast");
    CHECK(!lan_dst_check(&relay, ip4(10, 14, 0, 9), 49152, &wire, &why), "relay: outside 10.13/16");
    CHECK(!lan_dst_check(&relay, ip4(192, 168, 1, 40), 49152, &wire, &why), "relay: the real LAN");
    CHECK(lan_src_ok(&lan, ip4(192, 168, 1, 40)) && !lan_src_ok(&lan, ip4(192, 168, 2, 40)) &&
          !lan_src_ok(&lan, ip4(192, 168, 1, 255)), "lan: sources");
    CHECK(lan_src_ok(&relay, ip4(10, 13, 3, 4)) && !lan_src_ok(&relay, ip4(10, 12, 3, 4)), "relay: sources");
    CHECK(lan_port_blocked(53) && lan_port_blocked(5353) && !lan_port_blocked(35000) && !lan_port_blocked(49152), "ports");
    char err[128];
    CHECK(lan_net_ok(ip4(192, 168, 1, 23), 0xFFFFFF00u, err, sizeof err) &&
          !lan_net_ok(ip4(203, 0, 113, 9), 0xFFFFFF00u, err, sizeof err) &&
          !lan_net_ok(ip4(192, 168, 1, 23), 0xFF00FF00u, err, sizeof err) && lan_mask_prefix(0xFFFFF000u) == 20 &&
          lan_mask_prefix(0xFF00FF00u) == -1, "interface checks");
    printf("lan: allowlist, %d public destinations refused in each of %d modes: %s\n", checked / 7, 7,
           failures > before ? "FAIL" : "ok");
}

// ---- relay codec
static void check_codec(void) {
    int before = failures;
    uint8_t b[LR_MAX_DATAGRAM + 8], cookie[16], token[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    LrMsg m;
    for (int i = 0; i < 16; i++) cookie[i] = (uint8_t)(0xA0 + i);
    size_t n = lr_hello(b, 0x1122334455667788ull);
    CHECK(n == LR_HELLO_SIZE && !memcmp(b, "S3LR\x01\x01\x00\x00", 8) && lr_parse(b, n, &m) && m.type == LR_HELLO &&
          m.nonce == 0x1122334455667788ull, "HELLO");
    CHECK(!lr_parse(b, 63, &m), "a short HELLO");
    n = lr_challenge(b, 7, cookie);
    CHECK(n == 32 && lr_parse(b, n, &m) && m.type == LR_CHALLENGE && m.nonce == 7 && !memcmp(m.cookie, cookie, 16),
          "CHALLENGE");
    n = lr_join(b, 7, cookie, ip4(10, 13, 0, 4), "ROOM-1", "Inkling");
    CHECK(n == LR_JOIN_MIN && lr_parse(b, n, &m) && m.type == LR_JOIN && m.vip == ip4(10, 13, 0, 4) &&
          !strcmp(m.room, "ROOM-1") && !strcmp(m.name, "Inkling") && !memcmp(m.cookie, cookie, 16), "JOIN");
    b[40] = 'r';
    CHECK(!lr_parse(b, n, &m), "JOIN with a lower-case room code");
    n = lr_join(b, 7, cookie, 0, "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", "0123456789abcdef0123456789abcdef");
    CHECK(n == 104 && lr_parse(b, n, &m) && strlen(m.room) == 32 && strlen(m.name) == 32, "JOIN at its largest");
    n = lr_join_ack(b, 7, 0xDEADBEEF, token, ip4(10, 13, 0, 4), 16, 12, 5, 30, LR_MAX_PAYLOAD);
    CHECK(n == 40 && lr_parse(b, n, &m) && m.type == LR_JOIN_ACK && m.session == 0xDEADBEEF && !memcmp(m.token, token, 8) &&
          m.vip == ip4(10, 13, 0, 4) && m.prefix == 16 && m.max_members == 12 && m.keepalive_s == 5 && m.idle_s == 30 &&
          m.max_payload == 1440, "JOIN_ACK");
    n = lr_join_nak(b, 7, LR_NAK_ROOM_FULL, LR_NAK_DETAIL_NONE);
    CHECK(n == 24 && lr_parse(b, n, &m) && m.reason == LR_NAK_ROOM_FULL && m.detail == 0, "JOIN_NAK");
    n = lr_join_nak(b, 7, LR_NAK_ROOM_FULL, LR_NAK_DETAIL_SOURCE);
    CHECK(n == 24 && b[17] == 1 && lr_parse(b, n, &m) && m.reason == LR_NAK_ROOM_FULL && m.detail == LR_NAK_DETAIL_SOURCE,
          "JOIN_NAK with its detail (byte 17)");
    n = lr_keepalive(b, 9, token, 42);
    CHECK(n == 28 && lr_parse(b, n, &m) && m.type == LR_KEEPALIVE && m.session == 9 && m.seq == 42, "KEEPALIVE");
    n = lr_keepalive_ack(b, 9, token, 42, 3);
    CHECK(n == 28 && lr_parse(b, n, &m) && m.seq == 42 && m.members == 3, "KEEPALIVE_ACK");
    n = lr_leave(b, 9, token);
    CHECK(n == 20 && lr_parse(b, n, &m) && m.type == LR_LEAVE, "LEAVE");
    n = lr_closed(b, 9, token, LR_CLOSED_IDLE);
    CHECK(n == 24 && lr_parse(b, n, &m) && m.type == LR_CLOSED && m.reason == LR_CLOSED_IDLE, "CLOSED");
    n = lr_member(b, 9, token, 1, ip4(10, 13, 0, 5), "Octo");
    CHECK(n == 32 && lr_parse(b, n, &m) && m.type == LR_MEMBER && m.reason == 1 && m.vip == ip4(10, 13, 0, 5) &&
          !strcmp(m.name, "Octo"), "MEMBER");
    static uint8_t payload[LR_MAX_PAYLOAD + 1];
    for (size_t i = 0; i < sizeof payload; i++) payload[i] = (uint8_t)i;
    n = lr_data(b, 9, token, ip4(10, 13, 0, 4), 49152, ip4(10, 13, 255, 255), 35000, payload, LR_MAX_PAYLOAD);
    CHECK(n == LR_MAX_DATAGRAM && lr_parse(b, n, &m) && m.type == LR_DATA && m.src_ip == ip4(10, 13, 0, 4) &&
          m.src_port == 49152 && m.dst_ip == ip4(10, 13, 255, 255) && m.dst_port == 35000 && m.len == LR_MAX_PAYLOAD &&
          !memcmp(m.payload, payload, LR_MAX_PAYLOAD), "DATA at its largest");
    CHECK(!lr_data(b, 9, token, 0, 0, 0, 0, payload, LR_MAX_PAYLOAD + 1), "DATA over the limit");
    n = lr_data(b, 9, token, 1, 2, 3, 4, NULL, 0);
    CHECK(n == 32 && lr_parse(b, n, &m) && m.len == 0, "empty DATA");
    // byte for byte, the test vectors of docs/lan-relay-protocol.md
    static const uint8_t v_join[64] = {
        0x53, 0x33, 0x4c, 0x52, 0x01, 0x03, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad, 0xae, 0xaf,
        0x00, 0x00, 0x00, 0x00, 0x06, 0x07, 0x00, 0x00, 0x52, 0x4f, 0x4f, 0x4d, 0x2d, 0x31, 0x49, 0x6e,
        0x6b, 0x6c, 0x69, 0x6e, 0x67};
    static const uint8_t v_ack[40] = {
        0x53, 0x33, 0x4c, 0x52, 0x01, 0x04, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        0xc0, 0xff, 0xee, 0x01, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x0a, 0x0d, 0x00, 0x02,
        0x10, 0x0c, 0x05, 0x1e, 0x05, 0xa0, 0x00, 0x00};
    static const uint8_t v_data[36] = {
        0x53, 0x33, 0x4c, 0x52, 0x01, 0x10, 0x00, 0x00, 0xc0, 0xff, 0xee, 0x01, 0x11, 0x22, 0x33, 0x44,
        0x55, 0x66, 0x77, 0x88, 0x0a, 0x0d, 0x00, 0x02, 0xc0, 0x00, 0x0a, 0x0d, 0xff, 0xff, 0x88, 0xb8,
        0xde, 0xad, 0xbe, 0xef};
    static const uint8_t v_token[8] = {0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88};
    n = lr_join(b, 0x0123456789ABCDEFull, cookie, 0, "ROOM-1", "Inkling");
    CHECK(n == sizeof v_join && !memcmp(b, v_join, n), "JOIN test vector");
    n = lr_join_ack(b, 0x0123456789ABCDEFull, 0xC0FFEE01, v_token, ip4(10, 13, 0, 2), 16, 12, 5, 30, 1440);
    CHECK(n == sizeof v_ack && !memcmp(b, v_ack, n), "JOIN_ACK test vector");
    n = lr_data(b, 0xC0FFEE01, v_token, ip4(10, 13, 0, 2), 49152, ip4(10, 13, 255, 255), 35000, "\xde\xad\xbe\xef", 4);
    CHECK(n == sizeof v_data && !memcmp(b, v_data, n), "DATA test vector");
    b[4] = 2;
    CHECK(!lr_parse(b, n, &m) && m.version == 2, "another version");
    b[4] = 1, b[0] = 'X';
    CHECK(!lr_parse(b, n, &m), "bad magic");
    b[0] = 'S', b[5] = 0x7F;
    CHECK(!lr_parse(b, n, &m), "unknown type");
    printf("lan: relay protocol codec: %s\n", failures > before ? "FAIL" : "ok");
}

// ---- the guest ABI
static Cpu *cpu;

static int64_t call(const char *name, int n, ...) {
    LanGuestFn f = lan_guest_fn(name);
    if (!f) rt_fatal("probe-lan: no LAN implementation of %s", name);
    va_list ap;
    va_start(ap, n);
    for (int i = 0; i < 8; i++) cpu->x[i] = i < n ? va_arg(ap, uint64_t) : 0;
    va_end(ap);
    cpu->x[30] = 0;
    f(cpu);
    return (int64_t)cpu->x[0];
}
#define A(x) ((uint64_t)(uintptr_t)(x))
#define I(x) ((uint64_t)(int64_t)(x))

static const char *const S_SOCKET = "_ZN2nn6socket6SocketENS0_6FamilyENS0_4TypeENS0_8ProtocolE";
static const char *const S_BIND = "_ZN2nn6socket4BindEiPKNS0_8SockAddrEj";
static const char *const S_SETOPT = "_ZN2nn6socket10SetSockOptEiNS0_5LevelENS0_6OptionEPKvj";
static const char *const S_SENDTO = "_ZN2nn6socket6SendToEiPKvmNS0_7MsgFlagEPKNS0_8SockAddrEj";
static const char *const S_RECVFROM = "_ZN2nn6socket8RecvFromEiPvmNS0_7MsgFlagEPNS0_8SockAddrEPj";
static const char *const S_ERRNO = "_ZN2nn6socket12GetLastErrorEv";
static const char *const S_CLOSE = "_ZN2nn6socket5CloseEi";

static void sa(uint8_t out[16], uint32_t ip, uint16_t port) {
    uint8_t b[16] = {16, 2, (uint8_t)(port >> 8), (uint8_t)port, (uint8_t)(ip >> 24), (uint8_t)(ip >> 16), (uint8_t)(ip >> 8),
                     (uint8_t)ip};
    memcpy(out, b, 16);
}
static uint32_t sa_ip(const uint8_t *s) { return ip4(s[4], s[5], s[6], s[7]); }
static uint16_t sa_port(const uint8_t *s) { return (uint16_t)(s[2] << 8 | s[3]); }
static int64_t err_now(void) { return call(S_ERRNO, 0); }

static int udp(void) { return (int)call(S_SOCKET, 3, I(2), I(2), I(17)); }
static int64_t bind_to(int fd, uint32_t ip, uint16_t port) {
    uint8_t a[16];
    sa(a, ip, port);
    return call(S_BIND, 3, I(fd), A(a), I(16));
}
static int64_t setopt(int fd, int level, int opt, int v) { return call(S_SETOPT, 5, I(fd), I(level), I(opt), A(&v), I(4)); }
static int64_t send_to(int fd, const void *p, size_t n, uint32_t ip, uint16_t port) {
    uint8_t a[16];
    sa(a, ip, port);
    return call(S_SENDTO, 6, I(fd), A(p), (uint64_t)n, I(0), A(a), I(16));
}
static int64_t recv_from(int fd, void *p, size_t n, int flags, uint8_t from[16]) {
    uint32_t len = 16;
    return call(S_RECVFROM, 6, I(fd), A(p), (uint64_t)n, I(flags), A(from), A(&len));
}
static uint16_t port_of(int fd) {
    uint8_t a[16] = {0};
    uint32_t len = 16;
    call("nnsocketGetSockName", 3, I(fd), A(a), A(&len));
    return sa_port(a);
}
// Waits up to ms for a datagram on fd (MSG_DONTWAIT polling, as the session library's receive thread does).
static int64_t recv_wait(int fd, void *p, size_t n, uint8_t from[16], int ms) {
    uint64_t t0 = plat_now_ms();
    for (;;) {
        int64_t r = recv_from(fd, p, n, 0x80, from);
        if (r >= 0 || err_now() != LE_AGAIN || plat_now_ms() - t0 > (uint64_t)ms) return r;
        plat_sleep_ms(1);
    }
}

// A datagram the receive observer drops (auto_match.c drops a session answer without the host's name; a source outside
// the subnet takes the same path) never reaches the guest's buffer, and one non-blocking receive looks at
// LAN_DROPS_PER_CALL of them at most before it answers EAGAIN, however many are queued (otherwise a refused
// datagram would be copied into the guest's buffer before the check, and one call would go through every queued
// one).
static volatile int32_t dropped_seen;
static int drop_marked(int ev, uint16_t lport, uint32_t ip, const void *p, long n) {
    (void)lport, (void)ip;
    if (ev != LAN_OBS_RECV || n < 4 || memcmp(p, "DROP", 4)) return 1;
    plat_inc32(&dropped_seen);
    return 0;
}

static void check_dropped(int a, int b, uint32_t me, uint16_t pb) {
    uint8_t got[64], from[16];
    enum { QUEUED = 100 };
    dropped_seen = 0;
    lan_set_observer(drop_marked);
    for (int i = 0; i < QUEUED; i++) send_to(a, "DROP-ME", 7, me, pb);
    plat_sleep_ms(50);
    memset(got, '.', sizeof got);
    CHECK(recv_from(b, got, sizeof got, 0x80, from) == -1 && err_now() == LE_AGAIN, "only dropped datagrams: EAGAIN");
    CHECK(got[0] == '.', "a dropped datagram's bytes reached the guest's buffer (\"%.7s\")", (const char *)got);
    CHECK(dropped_seen == LAN_DROPS_PER_CALL, "one non-blocking receive looked at %d dropped datagrams (%d at most)",
          (int)dropped_seen, LAN_DROPS_PER_CALL);
    for (int i = 0; i < 4 && dropped_seen < QUEUED; i++) recv_from(b, got, sizeof got, 0x80, from);
    CHECK(dropped_seen == QUEUED, "the next receives drop the rest (%d of %d)", (int)dropped_seen, QUEUED);
    send_to(a, "KEEP", 4, me, pb);
    CHECK(recv_wait(b, got, sizeof got, from, 1000) == 4 && !memcmp(got, "KEEP", 4), "a datagram after the dropped ones");
    lan_set_observer(NULL);
}

static void check_abi(void) {
    int before = failures;
    LanCfg c;
    char err[256];
    uint32_t me = ip4(127, 0, 0, 2);
    if (!lan_parse("lan:127.0.0.2/8", NULL, NULL, NULL, &c, err, sizeof err) || !lan_net_start(&c, err, sizeof err)) {
        CHECK(0, "lan mode on 127.0.0.2/8: %s", err);
        return;
    }
    // nifm: nothing until a local request is submitted; an internet request stays unavailable
    uint32_t cfgv[5] = {0};
    CHECK(call("_ZN2nn4nifm10InitializeEv", 0) == 0 && call("_ZN2nn4nifm18IsNetworkAvailableEv", 0) == 0,
          "nifm available before a request");
    call("_ZN2nn4nifm19SetLocalNetworkModeEb", 1, I(0));
    call("_ZN2nn4nifm27SubmitNetworkRequestAndWaitEv", 0);
    CHECK(call("_ZN2nn4nifm18IsNetworkAvailableEv", 0) == 0, "an internet request was accepted");
    CHECK((uint32_t)call("_ZN2nn4nifm22GetCurrentIpConfigInfoEP7in_addrS2_S2_S2_S2_", 5, A(&cfgv[0]), A(&cfgv[1]),
                         A(&cfgv[2]), A(&cfgv[3]), A(&cfgv[4])) != 0, "IP configuration without a request");
    call("_ZN2nn4nifm19SetLocalNetworkModeEb", 1, I(1));
    call("_ZN2nn4nifm27SubmitNetworkRequestAndWaitEv", 0);
    CHECK(call("_ZN2nn4nifm18IsNetworkAvailableEv", 0) == 1, "the local request was not accepted");
    CHECK(call("_ZN2nn4nifm22GetCurrentIpConfigInfoEP7in_addrS2_S2_S2_S2_", 5, A(&cfgv[0]), A(&cfgv[1]), A(&cfgv[2]),
               A(&cfgv[3]), A(&cfgv[4])) == 0 && cfgv[0] == __builtin_bswap32(me) && cfgv[1] == __builtin_bswap32(0xFF000000u) &&
          !cfgv[2] && !cfgv[3] && !cfgv[4], "IP configuration %08x/%08x", cfgv[0], cfgv[1]);

    // stream sockets and other families never open
    CHECK(call("nnsocketSocket", 3, I(2), I(1), I(6)) == -1 && call("nnsocketGetLastErrno", 0) == LE_NETDOWN, "TCP socket");
    CHECK(call("_ZN2nn6socket6SocketEiii", 3, I(28), I(2), I(17)) == -1 && err_now() == LE_AFNOSUPPORT, "IPv6 socket");
    CHECK(call("nnsocketSocket", 3, I(2), I(3), I(0)) == -1, "raw socket");
    // no name is ever resolved
    uint64_t res = 0xDEAD;
    CHECK(call("nnsocketGetAddrInfo", 4, A("example.com"), A("443"), A(NULL), A(&res)) != 0 && res == 0, "DNS lookup");
    CHECK(call("_ZN2nn6socket11GetAddrInfoEPKcS2_PKNS0_8AddrInfoEPPS3_", 4, A("10.0.0.1"), A(NULL), A(NULL), A(&res)) != 0,
          "numeric lookup");
    CHECK(call("nnsocketGetHostByNameWithOptions", 3, A("nintendo.net"), A(NULL), A(NULL)) == 0, "gethostbyname");

    int a = udp(), b = udp();
    CHECK(a >= 0 && b >= 0 && a != b, "sockets %d %d", a, b);
    CHECK(bind_to(a, 0, 0) == 0 && bind_to(b, 0, 0) == 0, "bind 0.0.0.0:0");
    CHECK(bind_to(a, 0, 0) == -1 && err_now() == LE_INVAL, "second bind");
    int x = udp();
    CHECK(bind_to(x, ip4(127, 0, 0, 9), 0) == -1 && err_now() == LE_ADDRNOTAVAIL, "bind to an address not ours");
    CHECK(bind_to(x, 0, port_of(b)) == -1 && err_now() == LE_ADDRINUSE, "bind to a port in use");
    call(S_CLOSE, 1, I(x));
    uint16_t pa = port_of(a), pb = port_of(b);
    CHECK(pa && pb && pa != pb, "ports %u %u", pa, pb);
    uint8_t got[64], from[16];
    CHECK(recv_from(b, got, sizeof got, 0x80, from) == -1 && err_now() == LE_AGAIN, "empty socket: EAGAIN");
    CHECK(send_to(a, "hello", 5, me, pb) == 5, "unicast send");
    CHECK(recv_wait(b, got, sizeof got, from, 1000) == 5 && !memcmp(got, "hello", 5) && from[0] == 16 && from[1] == 2 &&
          sa_ip(from) == me && sa_port(from) == pa, "unicast receive and its sockaddr");
    // refused destinations fail before anything is sent, with ENETUNREACH, and are counted
    uint64_t refused = lan_refusals();
    for (size_t i = 0; i < sizeof publics / sizeof *publics; i++)
        CHECK(send_to(a, "x", 1, publics[i], 443) == -1 && err_now() == LE_NETUNREACH, "send to public %08x", publics[i]);
    CHECK(send_to(a, "x", 1, ip4(192, 168, 1, 7), 49152) == -1, "send to another private network");
    CHECK(send_to(a, "x", 1, ip4(127, 0, 0, 3), 53) == -1, "DNS port on the LAN");
    CHECK(lan_refusals() - refused == sizeof publics / sizeof *publics + 2, "refusals counted");
    uint8_t dst[16];
    sa(dst, publics[0], 80);
    CHECK(call("nnsocketConnect", 3, I(a), A(dst), I(16)) == -1 && call("nnsocketGetLastErrno", 0) == LE_NETUNREACH,
          "connect to a public address");
    // broadcast needs SO_BROADCAST and reaches the sender's own socket too
    CHECK(send_to(a, "bc", 2, 0xFFFFFFFFu, pb) == -1 && err_now() == LE_ACCES, "broadcast without SO_BROADCAST");
    CHECK(setopt(a, 0xFFFF, 0x20, 1) == 0 && setopt(a, 0xFFFF, 4, 1) == 0 && setopt(a, 0, 4, 64) == 0 &&
          setopt(a, 0xFFFF, 0x200, 1) == 0, "Pia's socket options");
    CHECK(setopt(a, 0, 12, 1) == -1 && err_now() == LE_NOPROTOOPT, "multicast membership");
    CHECK(send_to(a, "bc", 2, 0xFFFFFFFFu, pb) == 2, "broadcast send");
    CHECK(recv_wait(b, got, sizeof got, from, 1000) == 2 && !memcmp(got, "bc", 2) && sa_ip(from) == me, "broadcast receive");
    // poll
    LanPollFd pf[2] = {{b, LAN_POLLIN, 0}, {999, LAN_POLLIN, 0}};
    CHECK(call("nnsocketPoll", 3, A(pf), I(2), I(0)) == 1 && pf[0].revents == 0 && pf[1].revents == LAN_POLLNVAL,
          "poll, nothing queued");
    send_to(a, "p", 1, me, pb);
    CHECK(call("_ZN2nn6socket4PollEP6pollfdmi", 3, A(pf), I(1), I(1000)) == 1 && pf[0].revents == LAN_POLLIN, "poll ready");
    CHECK(recv_from(b, got, sizeof got, 0, from) == 1, "blocking receive of a queued datagram");
    // a blocking receive honours SO_RCVTIMEO; O_NONBLOCK through fcntl
    int64_t tv[2] = {0, 100000};
    CHECK(call("nnsocketSetSockOpt", 5, I(b), I(0xFFFF), I(0x1006), A(tv), I(16)) == 0, "SO_RCVTIMEO");
    uint64_t t0 = plat_now_ms();
    CHECK(recv_from(b, got, sizeof got, 0, from) == -1 && err_now() == LE_AGAIN && plat_now_ms() - t0 >= 90,
          "blocking receive timed out");
    CHECK(call("nnsocketFcntl", 3, I(b), I(4), I(4)) == 0 && call("nnsocketFcntl", 3, I(b), I(3), I(0)) == 6, "O_NONBLOCK");
    t0 = plat_now_ms();
    CHECK(call("nnsocketRecv", 4, I(b), A(got), I(sizeof got), I(0)) == -1 && plat_now_ms() - t0 < 50, "non-blocking recv");
    // msghdr
    struct { uint64_t base, len; } iov[2] = {{A("ab"), 2}, {A("cde"), 3}};
    struct { uint64_t name; uint32_t namelen, p0; uint64_t iov; int32_t iovlen, p1; uint64_t control; uint32_t controllen;
             int32_t flags; } msg = {A(dst), 16, 0, A(iov), 2, 0, 0, 0, 0};
    sa(dst, me, pb);
    CHECK(call("nnsocketSendMsg", 3, I(a), A(&msg), I(0)) == 5, "sendmsg");
    uint8_t r1[3], r2[8], rfrom[16];
    struct { uint64_t base, len; } riov[2] = {{A(r1), 3}, {A(r2), 8}};
    msg = (typeof(msg)){A(rfrom), 16, 0, A(riov), 2, 0, 0, 0, 0};
    plat_sleep_ms(20);
    CHECK(call("nnsocketRecvMsg", 3, I(b), A(&msg), I(0)) == 5 && !memcmp(r1, "abc", 3) && !memcmp(r2, "de", 2) &&
          sa_port(rfrom) == pa && msg.namelen == 16, "recvmsg");
    // truncation keeps the head of the datagram, as BSD does
    send_to(a, "0123456789", 10, me, pb);
    plat_sleep_ms(20);
    CHECK(recv_from(b, got, 4, 0x80, from) == 4 && !memcmp(got, "0123", 4), "truncated receive");
    check_dropped(a, b, me, pb);
    // getsockopt, close
    int v = 0;
    uint32_t vl = 4;
    CHECK(call("nnsocketGetSockOpt", 5, I(a), I(0xFFFF), I(0x1008), A(&v), A(&vl)) == 0 && v == 2, "SO_TYPE");
    CHECK(call(S_CLOSE, 1, I(a)) == 0 && call(S_CLOSE, 1, I(a)) == -1 && err_now() == LE_BADF, "close twice");
    CHECK(send_to(a, "x", 1, me, pb) == -1 && err_now() == LE_BADF, "send on a closed socket");
    CHECK(call("nnsocketListen", 2, I(b), I(1)) == -1 && call("nnsocketGetLastErrno", 0) == LE_OPNOTSUPP, "listen");
    call(S_CLOSE, 1, I(b));
    printf("lan: guest ABI in lan mode on 127.0.0.2/8 (sockets, sockaddr, errno, broadcast, poll, msghdr, nifm): %s\n",
           failures > before ? "FAIL" : "ok");
}

int probe_lan(void) {
    cpu = plat_aligned_alloc(sizeof(Cpu), 64);
    memset(cpu, 0, sizeof *cpu);
    check_parse();
    check_allowlist();
    check_codec();
    check_abi();
    // A name two members of the relay room joined with, or this player's own name, names
    // nobody (teams by name seat no one by it), rather than the first member found with it
    int roster = lan_roster_selftest();
    CHECK(!roster, "the relay room's names: %d wrong answers", roster);
    printf("lan: relay room names, a name held twice or this player's own names nobody: %s\n", roster ? "FAIL" : "ok");
    // a relay's refusal is kept for the error the match stops with (auto_match.c), and forgotten once joined
    int naks = lan_relay_nak_selftest();
    CHECK(!naks, "the relay's refusals: %d wrong answers", naks);
    printf("lan: the relay's refusals kept until a join: %s\n", naks ? "FAIL" : "ok");
    printf("probe-lan: %s (%d failures)\n", failures ? "FAILED" : "passed", failures);
    return failures ? 1 : 0;
}

// ---- --probe-lan-peer
int probe_lan_peer(void) {
    cpu = plat_aligned_alloc(sizeof(Cpu), 64);
    memset(cpu, 0, sizeof *cpu);
    LanCfg c;
    char err[256], a[16], b[16];
    const char *spec = cfg_get("SPLAT3_LAN");
    if (!lan_parse(spec, cfg_get("SPLAT3_LAN_ROOM"), cfg_get("SPLAT3_LAN_NAME"), cfg_get("SPLAT3_LAN_BIND"), &c, err,
                   sizeof err) || c.mode == LAN_OFF) {
        printf("probe-lan-peer: SPLAT3_LAN=%s: %s\n", spec ? spec : "", c.mode == LAN_OFF ? "set a lan or relay mode" : err);
        return 2;
    }
    if (!lan_net_start(&c, err, sizeof err)) {
        printf("probe-lan-peer: %s\n", err);
        return 1;
    }
    const char *secs = cfg_get("SPLAT3_PEER_SECONDS");
    uint64_t limit = (secs ? (uint64_t)atoi(secs) : 20) * 1000, t0 = plat_now_ms();
    // nifm as the game asks (main+0x44f3e00: SetLocalNetworkMode(true), SubmitNetworkRequestAndWait,
    // IsNetworkAvailable, then GetCurrentIpConfigInfo for the session library's address and mask)
    call("_ZN2nn4nifm10InitializeEv", 0);
    call("_ZN2nn4nifm19SetLocalNetworkModeEb", 1, I(1));
    call("_ZN2nn4nifm27SubmitNetworkRequestAndWaitEv", 0);
    uint32_t v[5];
    if (!call("_ZN2nn4nifm18IsNetworkAvailableEv", 0) ||
        call("_ZN2nn4nifm22GetCurrentIpConfigInfoEP7in_addrS2_S2_S2_S2_", 5, A(&v[0]), A(&v[1]), A(&v[2]), A(&v[3]),
             A(&v[4]))) {
        printf("probe-lan-peer: the local network request failed\n");
        return 1;
    }
    uint32_t me = __builtin_bswap32(v[0]), mask = __builtin_bswap32(v[1]), bcast = me | ~mask;
    printf("probe-lan-peer: %s/%d, broadcast %s\n", lan_ip_str(me, a), lan_mask_prefix(mask), lan_ip_str(bcast, b));
    // The session library's LAN sockets (main+0x95cdfc): transport 49152, discovery 35000; SO_REUSEADDR before bind, SO_BROADCAST after
    int tr = udp(), disc = udp();
    setopt(tr, 0xFFFF, 4, 1);
    setopt(disc, 0xFFFF, 4, 1);
    if (bind_to(tr, 0, 49152) || bind_to(disc, 0, 35000)) {
        printf("probe-lan-peer: bind 49152/35000 failed (errno %lld)\n", (long long)err_now());
        return 1;
    }
    setopt(tr, 0xFFFF, 0x20, 1);
    setopt(disc, 0xFFFF, 0x20, 1);
    // refused in this mode too
    int refused_ok = send_to(tr, "x", 1, 0x08080808u, 53) == -1 && err_now() == LE_NETUNREACH &&
                     send_to(tr, "x", 1, 0x01010101u, 443) == -1 && err_now() == LE_NETUNREACH;
    uint8_t buf[256], from[16];
    int hellos = 0, own = 0, pongs = 0, pings = 0, late = 0;
    uint32_t peer = 0;
    uint64_t next_hello = 0, next_ping = 0;
    uint32_t seq = 0;
    // SPLAT3_PEER_HOLD=1: run the whole time and also need three round trips in the last 4 s (a relay restarted
    // meanwhile must have taken the peers back at the same addresses)
    int hold = cfg_get("SPLAT3_PEER_HOLD") && atoi(cfg_get("SPLAT3_PEER_HOLD"));
    while (plat_now_ms() - t0 < limit &&
           (hold || !(hellos && pongs >= 3 && pings >= 3 && plat_now_ms() - t0 > 1500))) {
        uint64_t now = plat_now_ms();
        if (now >= next_hello) {
            char msg[64];
            int n = snprintf(msg, sizeof msg, "HELLO %s", lan_ip_str(me, a));
            if (send_to(tr, msg, (size_t)n, 0xFFFFFFFFu, 35000) != n) printf("probe-lan-peer: broadcast failed (errno %lld)\n",
                                                                         (long long)err_now());
            next_hello = now + 250;
        }
        if (peer && now >= next_ping) {
            char msg[32];
            int n = snprintf(msg, sizeof msg, "PING %u", ++seq);
            send_to(tr, msg, (size_t)n, peer, 49152);
            next_ping = now + 100;
        }
        int64_t n = recv_from(disc, buf, sizeof buf - 1, 0x80, from);
        if (n > 0) {
            buf[n] = 0;
            if (sa_ip(from) == me) own++;
            else if (!strncmp((char *)buf, "HELLO ", 6)) {
                if (!peer) printf("probe-lan-peer: discovered %s by broadcast\n", lan_ip_str(sa_ip(from), a));
                peer = sa_ip(from);
                hellos++;
            }
        }
        n = recv_from(tr, buf, sizeof buf - 1, 0x80, from);
        if (n > 0) {
            buf[n] = 0;
            if (!strncmp((char *)buf, "PING ", 5)) {
                buf[1] = 'O';   // PING n -> PONG n, back to the sender's address and port
                send_to(tr, buf, (size_t)n, sa_ip(from), sa_port(from));
                pings++;
            } else if (!strncmp((char *)buf, "PONG ", 5)) {
                pongs++;
                late += plat_now_ms() - t0 + 4000 >= limit;
            }
        }
        if (n < 0 && err_now() != LE_AGAIN) printf("probe-lan-peer: receive failed (errno %lld)\n", (long long)err_now());
        plat_sleep_ms(2);
    }
    int ok = hellos && pongs >= 3 && pings >= 3 && refused_ok && (!hold || late >= 3);
    uint32_t now_ip = 0, now_mask = 0;
    lan_nifm_ipconfig(&now_ip, &now_mask);
    printf("probe-lan-peer: %s: %s, peer %s, broadcasts heard %d (own %d), pings answered %d, pongs %d (%d in the last "
           "4 s), public destinations %s, %llu refusals%s\n", ok ? "passed" : "FAILED", lan_ip_str(me, a),
           peer ? lan_ip_str(peer, b) : "none", hellos, own, pings, pongs, late, refused_ok ? "refused" : "NOT refused",
           (unsigned long long)lan_refusals(), now_ip == me ? "" : ", ADDRESS CHANGED");
    if (now_ip != me) ok = 0;
    // linger so the other peer gets its last answers
    uint64_t t1 = plat_now_ms();
    while (plat_now_ms() - t1 < 1500) {
        int64_t n = recv_from(tr, buf, sizeof buf - 1, 0x80, from);
        if (n > 0 && !strncmp((char *)buf, "PING ", 5)) buf[1] = 'O', send_to(tr, buf, (size_t)n, sa_ip(from), sa_port(from));
        recv_from(disc, buf, sizeof buf, 0x80, from);
        plat_sleep_ms(2);
    }
    lan_leave();
    return ok ? 0 : 1;
}
