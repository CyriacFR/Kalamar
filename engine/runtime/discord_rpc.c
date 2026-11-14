// Discord presence ("Rich Presence") of the game, in the player's Discord status, through the local IPC of a running
// Discord client (platform.h's plat_ipc_*). Nothing leaves the machine but the activity itself, given to the Discord
// application of this PC, and nothing is sent while TOTK_DISCORD_RPC is off (the default: the status is information
// about the player's session).
//
// What is shown depends on the target:
// - the upstream game: "Game 1.4.3 · Recomp 0.3.7" with the play time, under the project's application (the default of
//   TOTK_DISCORD_APP_ID, registered as "TotK Recomp") and its art (TOTK_DISCORD_IMAGE); a player who wants their own
//   name or art registers an application at discord.com/developers and puts its id in totk.ini;
// - any other target (the neutral brand, brand.c): only under the application the launcher names. The launcher writes
//   discord_app_id and discord_rpc into the settings file when the player switches its own presence on or off
//   (launcher docs/discord-presence.md); there is no default application, so without one this file never opens a pipe.
//   The words are the launcher's minimal ones: "In game" (UI_INK_DISCORD_IN_GAME, in the menu language), the logo
//   kalamar_logo with the brand's name as its hover text, and the time since the game started. No version, no path,
//   no name.
//
// One source at a time. Started by the launcher (SPLAT3_DISCORD_PRESENCE=launcher, which it passes to every game it
// starts), the game never opens Discord's pipe, whatever its settings: the launcher shows a presence of its own, with
// what the game cannot know (the room, the search, the match), and two sources would replace each other's status. Started
// on its own, a game of another target also takes the lock "Kalamar.DiscordPresence" (plat_named_lock_try) that the
// launcher takes: while a launcher shows the presence, this game does not, and the other way round.
//
// One host thread owns the connection and never holds a virtual core (kernel.c: host work runs without one). It waits
// for the settings to be read (cfg_ready: it starts before cfg_init), re-reads them when cfg_version changes (a menu
// change reaches it within half a second; only a new application id reconnects, a new picture or menu language is
// simply sent again), connects to the first of Discord's discord-ipc-0..9 pipes that answers the handshake with READY
// (a pipe that closes, refuses or stays silent for 2 s: the next one), writes the activity once per connection and again
// only when it changes, and otherwise looks at the pipe every half second without waiting: a Ping is answered, an error
// answer is logged, a Close or a broken pipe means Discord went (looked for again 30 s later). An application id Discord
// refuses (Close with code 4000) is logged and tried again 10 minutes later. Switched off, the presence is cleared,
// then the pipe closed; when the process ends Discord clears it by itself.
//
// The protocol is the documented IPC one (discord-rpc's src/rpc_connection.h): a frame is <opcode u32 little
// endian><length u32><length bytes of JSON>, with Handshake 0, Frame 1, Close 2, Ping 3 and Pong 4. The handshake
// (opcode 0) carries {"v":1,"client_id":...} and is answered by a Ready frame (opcode 1, "evt":"READY"); every command,
// SET_ACTIVITY here, is a frame (opcode 1) answered by one; a SET_ACTIVITY without an activity clears it. A client that
// speaks another numbering (an early arRPC did) answers the handshake with a Close. Frames over 16 KiB are not
// Discord's and end the connection. --probe-discord makes the same decision and the same exchange once;
// SPLAT3_DISCORD_IPC=<name> names a stand-in's pipes (<name>-0..9) instead of Discord's, for tests.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "brand.h"
#include "cfg.h"
#include "host.h"
#include "json_esc.h"
#include "platform/platform.h"
#include "ui_text.h"

enum { OP_HANDSHAKE = 0, OP_FRAME = 1, OP_CLOSE = 2, OP_PING = 3, OP_PONG = 4 };

#define APP_ID_CAP 24    // a Discord snowflake: at most 20 digits
#define IMAGE_KEY_CAP 64
#define FRAME_CAP 16384  // the Ready frame carries the user's public profile and the client's config: a few KiB
#define JSON_CAP 1024
#define POLL_MS 500
#define ANSWER_MS 2000
#define RETRY_MS 30000
#define REFUSED_RETRY_MS 600000
#define PIPE_BASE "discord-ipc"
#define NEUTRAL_IMAGE "kalamar_logo"   // the logo the launcher's application carries (launcher docs/discord-presence.md)
#define LOCK_NAME "Kalamar.DiscordPresence"

// ---- the decision: who shows the presence, under which application, through which pipes

// The launcher started this game and shows the presence itself.
static int launcher_driven(void) {
    const char *v = cfg_env("SPLAT3_DISCORD_PRESENCE");
    return v && !strcmp(v, "launcher");
}

// The pipes' name: Discord's, or a stand-in's (SPLAT3_DISCORD_IPC, tests). A value that is not a plain name opens no
// pipe at all, never Discord's instead: 0.
static int pipe_base(char *out, size_t cap) {
    const char *v = cfg_env("SPLAT3_DISCORD_IPC");
    if (!v) {
        snprintf(out, cap, "%s", PIPE_BASE);
        return 1;
    }
    size_t n = strlen(v);
    if (!n || n + 3 > cap) return 0;
    for (const char *p = v; *p; p++)
        if ((*p < 'a' || *p > 'z') && (*p < 'A' || *p > 'Z') && (*p < '0' || *p > '9') && *p != '-' && *p != '_' && *p != '.')
            return 0;
    snprintf(out, cap, "%s", v);
    return 1;
}

// A settings value that fits the JSON below without escaping: the app id is digits, an asset key letters, digits,
// '_' and '-' (what Discord's developer portal accepts for an asset key).
static int valid_app_id(const char *s) {
    size_t n = strlen(s);
    if (!n || n > 20) return 0;
    for (const char *p = s; *p; p++)
        if (*p < '0' || *p > '9') return 0;
    return 1;
}

static int valid_image_key(const char *s) {
    size_t n = strlen(s);
    if (!n || n >= IMAGE_KEY_CAP) return 0;
    for (const char *p = s; *p; p++)
        if ((*p < 'a' || *p > 'z') && (*p < 'A' || *p > 'Z') && (*p < '0' || *p > '9') && *p != '_' && *p != '-')
            return 0;
    return 1;
}

// The application: the setting when it is one; else the upstream target's default (the project's application), and
// none for any other target. A value that does not fit is logged (the player sees why their own app does not show).
static void read_app_id(char *out, size_t cap, int quiet) {
    const char *v = cfg_get("TOTK_DISCORD_APP_ID");
    if (v && *v && valid_app_id(v)) {
        snprintf(out, cap, "%s", v);
        return;
    }
    if (v && *v && !quiet) rt_log("discord: DISCORD_APP_ID is not a Discord application id (digits only)");
    snprintf(out, cap, "%s", RT_TOTK ? cfg_meta("TOTK_DISCORD_APP_ID")->def : "");
}

static void read_image(char *out, size_t cap) {
    const char *v = cfg_get("TOTK_DISCORD_IMAGE");
    if (v && *v && valid_image_key(v)) {
        snprintf(out, cap, "%s", v);
        return;
    }
    if (v && *v) rt_log("discord: DISCORD_IMAGE is not an asset key (letters, digits, _ and -)");
    snprintf(out, cap, "%s", RT_TOTK ? cfg_meta("TOTK_DISCORD_IMAGE")->def : NEUTRAL_IMAGE);
}

int discord_offered(void) {
    if (launcher_driven()) return 0;
    char id[APP_ID_CAP];
    read_app_id(id, sizeof id, 1);   // asked at every frame of the menu: no log
    return id[0] != 0;
}

// ---- the wire

static void le32(uint8_t *at, uint32_t v) {
    at[0] = (uint8_t)v, at[1] = (uint8_t)(v >> 8), at[2] = (uint8_t)(v >> 16), at[3] = (uint8_t)(v >> 24);
}

static uint32_t rd32(const uint8_t *at) { return at[0] | at[1] << 8 | at[2] << 16 | (uint32_t)at[3] << 24; }

// A connection: the pipe and what it has sent that is not a whole frame yet.
typedef struct {
    PlatIpc *ipc;
    size_t n;
    uint8_t in[FRAME_CAP + 8];
} Conn;

static int write_frame(Conn *c, uint32_t op, const char *json, size_t len) {
    uint8_t head[8];
    le32(head, op);
    le32(head + 4, (uint32_t)len);
    return plat_ipc_write(c->ipc, head, sizeof head) && plat_ipc_write(c->ipc, json, len);
}

// What the pipe has, waiting at most ms for the first bytes. 0 when the connection broke or sent a frame too big for a
// Discord answer.
static int fill(Conn *c, uint32_t ms) {
    if (c->n >= sizeof c->in) return 0;
    int got = plat_ipc_read(c->ipc, c->in + c->n, sizeof c->in - c->n, ms);
    if (got < 0) return 0;
    c->n += (size_t)got;
    return 1;
}

// The next whole frame: 1 (*op, its payload NUL-terminated in out, cut at cap-1), 0 not yet, -1 a frame over FRAME_CAP.
// A Ping is answered here with its own payload, and taken like any frame.
static int take(Conn *c, uint32_t *op, char *out, size_t cap) {
    if (c->n < 8) return 0;
    uint32_t len = rd32(c->in + 4);
    if (len > FRAME_CAP) return -1;
    if (c->n < 8 + (size_t)len) return 0;
    *op = rd32(c->in);
    size_t keep = len < cap - 1 ? len : cap - 1;
    memcpy(out, c->in + 8, keep);
    out[keep] = 0;
    if (*op == OP_PING && !write_frame(c, OP_PONG, (const char *)c->in + 8, len)) return -1;
    memmove(c->in, c->in + 8 + len, c->n - 8 - len);
    c->n -= 8 + len;
    return 1;
}

// The value of the member "key" of a JSON payload, past the colon and any blank (JSON allows them; Discord's own
// answers have none, a stand-in's may), or NULL.
static const char *json_member(const char *s, const char *key) {
    char want[24];
    snprintf(want, sizeof want, "\"%s\"", key);
    for (const char *p = strstr(s, want); p; p = strstr(p + 1, want)) {
        const char *v = p + strlen(want);
        while (*v == ' ' || *v == '\t' || *v == '\r' || *v == '\n') v++;
        if (*v != ':') continue;
        v++;
        while (*v == ' ' || *v == '\t' || *v == '\r' || *v == '\n') v++;
        return v;
    }
    return NULL;
}

// The number of "code" in a Close payload or an error answer; 0 without one.
static int json_code(const char *s) {
    const char *v = json_member(s, "code");
    return v ? atoi(v) : 0;
}

static int is_evt(const char *s, const char *evt) {
    const char *v = json_member(s, "evt");
    size_t n = strlen(evt);
    return v && *v == '"' && !strncmp(v + 1, evt, n) && v[1 + n] == '"';
}

static void conn_close(Conn *c) {
    if (!c) return;
    plat_ipc_close(c->ipc);
    free(c);
}

static uint32_t next_nonce(void) {
    static volatile int32_t n;
    return (uint32_t)plat_inc32(&n);
}

// The first of <base>-0..9 that completes a handshake with READY, else NULL. *refused gets the code of a pipe that
// refused the application (4000: an invalid client id, which no other Discord takes either: the search stops there).
static Conn *connect_client(const char *base, const char *app_id, int *refused) {
    *refused = 0;
    for (int i = 0; i < 10; i++) {
        char name[80], json[96];
        snprintf(name, sizeof name, "%s-%d", base, i);
        PlatIpc *p = plat_ipc_connect(name);
        if (!p) continue;
        Conn *c = malloc(sizeof *c);
        if (!c) {
            plat_ipc_close(p);
            return NULL;
        }
        c->ipc = p, c->n = 0;
        int len = snprintf(json, sizeof json, "{\"v\":1,\"client_id\":\"%s\"}", app_id);
        int verdict = 0;   // 1 ready, -1 give this pipe up
        if (!write_frame(c, OP_HANDSHAKE, json, (size_t)len)) verdict = -1;
        uint64_t deadline = plat_now_ms() + ANSWER_MS;
        char answer[512];
        uint64_t t;
        while (!verdict && (t = plat_now_ms()) < deadline) {
            if (!fill(c, (uint32_t)(deadline - t))) {
                verdict = -1;
                break;
            }
            uint32_t op = 0;
            int r;
            while (!verdict && (r = take(c, &op, answer, sizeof answer)) != 0) {
                if (r < 0) verdict = -1;
                else if (op == OP_CLOSE || (op == OP_FRAME && is_evt(answer, "ERROR"))) *refused = json_code(answer), verdict = -1;
                else if (op == OP_FRAME && is_evt(answer, "READY")) verdict = 1;
            }
        }
        if (verdict == 1) return c;
        conn_close(c);   // this pipe answered but not with a Ready frame in 2 s: the next may belong to a client
        if (*refused == 4000) return NULL;
    }
    return NULL;
}

// Everything after the nonce of the SET_ACTIVITY frame: the same text means the same activity (no resend). The start
// is the process's start, so Discord counts the time since the game started.
static void activity_args(char *out, size_t cap, const char *image) {
    uint64_t start = (plat_unix_time_ns() - plat_process_uptime_ns()) / 1000000000ull;
    unsigned long pid = (unsigned long)plat_process_id();
    // Every string through json_esc.h: the image key is checked (valid_image_key), the version is the build's own, and
    // the menu text and the brand's name hold no quote today (a translation could).
    char image_e[IMAGE_KEY_CAP * 6], version[128], details[480], large[192];
    json_esc(image_e, sizeof image_e, image);
    json_esc(version, sizeof version, rt_port_version);
    if (RT_TOTK) {
        // the game update and the port version as the details line ("Game 1.4.3 · Recomp 0.3.7": the port version
        // without its +commit suffix, the art's hover text keeps the full one; the dot goes as a JSON escape)
        int v = rt_game_version;
        char short_version[64];
        json_esc_n(short_version, sizeof short_version, rt_port_version, strcspn(rt_port_version, "+"), JSON_ESC_UTF8);
        snprintf(out, cap,
                 "\"args\":{\"pid\":%lu,\"activity\":{\"details\":\"Game %d.%d.%d \\u00b7 Recomp %s\",\"timestamps\":"
                 "{\"start\":%llu},\"assets\":{\"large_image\":\"%s\",\"large_text\":\"TotK Recomp %s\"}}}}",
                 pid, v / 100, v / 10 % 10, v % 10, short_version, (unsigned long long)start, image_e, version);
        return;
    }
    json_esc(details, sizeof details, menu_text(UI_INK_DISCORD_IN_GAME));
    json_esc(large, sizeof large, rt_brand()->name);
    snprintf(out, cap,
             "\"args\":{\"pid\":%lu,\"activity\":{\"details\":\"%s\",\"timestamps\":{\"start\":%llu},\"assets\":"
             "{\"large_image\":\"%s\",\"large_text\":\"%s\"},\"instance\":false}}}",
             pid, details, (unsigned long long)start, image_e, large);
}

// One SET_ACTIVITY: args as activity_args wrote them, or NULL to clear the presence. 0 when the connection broke.
static int send_activity(Conn *c, const char *args) {
    char json[JSON_CAP + 64];
    int len = args ? snprintf(json, sizeof json, "{\"cmd\":\"SET_ACTIVITY\",\"nonce\":\"%u\",%s", next_nonce(), args)
                   : snprintf(json, sizeof json, "{\"cmd\":\"SET_ACTIVITY\",\"nonce\":\"%u\",\"args\":{\"pid\":%lu}}",
                              next_nonce(), (unsigned long)plat_process_id());
    return len > 0 && (size_t)len < sizeof json && write_frame(c, OP_FRAME, json, (size_t)len);
}

// Reads what the pipe has without waiting. 0 when Discord went (a Close, a broken pipe); *code gets the code of an
// error answer (an activity Discord did not take). 16 reads at most: a peer on the pipe that never stops writing gets
// the next look half a second later, not this thread forever.
static int poll_conn(Conn *c, int *code) {
    char answer[512];
    for (int reads = 0; reads < 16; reads++) {
        size_t before = c->n;
        if (!fill(c, 0)) return 0;
        uint32_t op = 0;
        int r;
        while ((r = take(c, &op, answer, sizeof answer)) != 0) {
            if (r < 0 || op == OP_CLOSE) return 0;
            if (op == OP_FRAME && is_evt(answer, "ERROR")) *code = json_code(answer);
        }
        if (c->n == before) return 1;   // nothing more came
    }
    return 1;   // more is coming: the next look takes it
}

// Clears the presence when one is shown (and waits briefly for Discord to take it), then closes the pipe.
static void conn_end(Conn *c, int shown) {
    if (!c) return;
    if (shown && send_activity(c, NULL)) {
        uint64_t deadline = plat_now_ms() + 400;
        char answer[512];
        uint32_t op = 0;
        int done = 0;
        while (!done && plat_now_ms() < deadline && fill(c, 20))
            while (!done && take(c, &op, answer, sizeof answer) > 0) done = op == OP_FRAME;
    }
    conn_close(c);
}

// ---- the presence thread

enum { SAID_NOTHING, SAID_BUSY, SAID_NONE, SAID_REFUSED };

static void discord_main(void *arg) {
    (void)arg;
    while (!cfg_ready()) plat_sleep_ms(POLL_MS);
    char base[64];
    if (!pipe_base(base, sizeof base)) {
        rt_log("discord: SPLAT3_DISCORD_IPC is not a plain pipe name: no presence");
        return;
    }
    char app_id[APP_ID_CAP] = "", image[IMAGE_KEY_CAP] = "", shown[JSON_CAP] = "", args[JSON_CAP];
    read_app_id(app_id, sizeof app_id, 0);
    if (!app_id[0]) {
        rt_log("discord: no Discord application named (discord_app_id): no presence");
        return;   // a target without a default application: nothing can name one during the run (no menu row for it)
    }
    unsigned seen = cfg_version() - 1;
    Conn *c = NULL;
    PlatNamedLock *lock = NULL;
    uint64_t retry_at = 0;
    int said = SAID_NOTHING, said_code = 0;
    for (;;) {
        unsigned v = cfg_version();
        if (v != seen) {   // the settings changed (the menu writes them): pick the new ones up at once
            seen = v;
            char id[APP_ID_CAP];
            read_app_id(id, sizeof id, 0);
            read_image(image, sizeof image);
            if (strcmp(id, app_id)) {   // a new application needs a new handshake; anything else is just sent again
                conn_end(c, shown[0]), c = NULL, shown[0] = 0;
                snprintf(app_id, sizeof app_id, "%s", id);
                retry_at = 0, said = SAID_NOTHING;
            }
        }
        if (!app_id[0] || !cfg_bool("TOTK_DISCORD_RPC")) {
            if (c) rt_log("discord: switched off: the presence is cleared");
            conn_end(c, shown[0]), c = NULL, shown[0] = 0;
            plat_named_lock_release(lock), lock = NULL;
            said = SAID_NOTHING, retry_at = 0;
            plat_sleep_ms(POLL_MS);
            continue;
        }
        uint64_t now = plat_now_ms();
        if (!c && now >= retry_at) {
            retry_at = now + RETRY_MS;
            if (!RT_TOTK && !lock && !(lock = plat_named_lock_try(LOCK_NAME))) {
                if (said != SAID_BUSY) rt_log("discord: another Kalamar of this PC shows the presence; trying again every 30 s");
                said = SAID_BUSY;
            } else {
                int refused = 0;
                c = connect_client(base, app_id, &refused);
                if (c) {
                    rt_log("discord: connected (app %s)", app_id);
                    said = SAID_NOTHING, shown[0] = 0;
                } else if (refused == 4000) {
                    if (said != SAID_REFUSED) rt_log("discord: Discord refused the application %s (code 4000); trying again every 10 minutes", app_id);
                    said = SAID_REFUSED, retry_at = now + REFUSED_RETRY_MS;
                    plat_named_lock_release(lock), lock = NULL;
                } else {
                    if (said != SAID_NONE) rt_log("discord: no Discord client answered; trying again every 30 s while the setting is on");
                    said = SAID_NONE;
                }
            }
        }
        if (c) {
            int code = 0;
            if (!poll_conn(c, &code)) {
                rt_log("discord: Discord closed the connection; looking for it again in 30 s");
                conn_close(c), c = NULL, shown[0] = 0;
                retry_at = now + RETRY_MS;
            } else {
                if (code && code != said_code) rt_log("discord: Discord refused the activity (code %d)", code);
                said_code = code;
                activity_args(args, sizeof args, image);
                if (strcmp(args, shown)) {
                    if (send_activity(c, args)) {
                        snprintf(shown, sizeof shown, "%s", args);
                    } else {
                        rt_log("discord: the connection broke; looking for Discord again in 30 s");
                        conn_close(c), c = NULL, shown[0] = 0;
                        retry_at = now + RETRY_MS;
                    }
                }
            }
        }
        plat_sleep_ms(POLL_MS);
    }
}

void discord_init(void) {
    if (launcher_driven()) {
        // The one trace of this case: the pipe is never opened, the thread never started.
        rt_log("discord: started by the launcher, which shows the Discord presence: this game stays quiet");
        return;
    }
    if (!plat_thread_spawn(discord_main, NULL, "discord", 0)) rt_log("discord: cannot start the presence thread");
}

// ---- --probe-discord: the decision and the exchange above once, without the game and whatever TOTK_DISCORD_RPC says;
// needs no game files. Reads the environment (TOTK_DISCORD_APP_ID, TOTK_DISCORD_IMAGE) and the defaults.

int probe_discord(void) {
    static int fails;
#define CHECK(c, what) do { if (!(c)) { fails++; fprintf(stderr, "FAIL discord: %s\n", what); } } while (0)
    if (launcher_driven()) {
        fprintf(stderr, "discord: probe skipped: started by the launcher (SPLAT3_DISCORD_PRESENCE=launcher), which shows "
                        "the presence; no pipe opened\n");
        return 0;
    }
    char base[64], app_id[APP_ID_CAP], image[IMAGE_KEY_CAP], args[JSON_CAP];
    if (!pipe_base(base, sizeof base)) {
        fprintf(stderr, "FAIL discord: SPLAT3_DISCORD_IPC is not a plain pipe name; no pipe opened\n");
        return 1;
    }
    read_app_id(app_id, sizeof app_id, 0);
    read_image(image, sizeof image);
    if (!app_id[0]) {
        fprintf(stderr, "discord: probe skipped: no Discord application named (discord_app_id); no pipe opened\n");
        return 0;
    }
    PlatNamedLock *lock = NULL;
    if (!RT_TOTK && !(lock = plat_named_lock_try(LOCK_NAME))) {
        fprintf(stderr, "discord: probe skipped: another Kalamar of this PC shows the presence; no pipe opened\n");
        return 0;
    }
    int refused = 0;
    Conn *c = connect_client(base, app_id, &refused);
    if (!c) {
        plat_named_lock_release(lock);
        if (refused) {
            fprintf(stderr, "FAIL discord: the client refused the application %s (code %d)\n", app_id, refused);
            return 1;
        }
        fprintf(stderr, "discord: probe skipped: no client on %s-0..9\n", base);
        return 0;
    }
    fprintf(stderr, "discord: connected (app %s)\n", app_id);
    activity_args(args, sizeof args, image);
    CHECK(send_activity(c, args), "SET_ACTIVITY written");
    // its answer: a frame, not an error
    uint64_t deadline = plat_now_ms() + ANSWER_MS;
    char answer[512];
    uint32_t op = 0;
    int answered = 0, error = 0;
    while (!answered && plat_now_ms() < deadline && fill(c, 50))
        while (!answered && take(c, &op, answer, sizeof answer) > 0)
            if (op == OP_FRAME) answered = 1, error = is_evt(answer, "ERROR");
    CHECK(answered, "SET_ACTIVITY answered");
    CHECK(!error, "SET_ACTIVITY taken (no error answer)");
    fprintf(stderr, "discord: activity set: {%s\n", args);
    // A person looks at the real Discord: the status stays ten seconds. A stand-in's test does not wait.
    if (!cfg_env("SPLAT3_DISCORD_IPC")) {
        fprintf(stderr, "discord: the status shows it for ten seconds, then it is cleared and the connection closes\n");
        plat_sleep_ms(10000);
    }
    conn_end(c, 1);
    plat_named_lock_release(lock);
    fprintf(stderr, "discord: probe %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
#undef CHECK
}
