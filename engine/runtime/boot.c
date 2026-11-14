// Adapted from MacNativeGames games/totk/runtime/loader.c (lines 456-477) and services.c (lines 219-235) @ b75be8cf
// (MIT, Copyright (c) 2026 MacNativeGames contributors) and LAS Recomp runtime/main.c (lines 16-35, 141-159) @
// da35b83 (MIT, Copyright (c) 2026 Peiikai).
// The boot driver: the guest main thread runs the game's own nn::init::Start
// with three host callbacks, the second of which runs DT_INIT of every module (main, subsdk0, sdk; main, sdk for
// Splatoon 3); Start then calls nnMain. --boot checks two checkpoints on the way and ends at the first
// nvnBootstrapLoader call, at the return address of each in main (the target's boot_checkpoint and boot_nvn,
// recomp/targets.py; TotK 1.2.1: +0xe7a954 and +0xe7eb2c):
//   SetFocusHandlingMode   in nnMain, every module's constructor flag reads 2
//   nvnBootstrapLoader     in the graphics bring-up; logs the run statistics, exits 0
// A target value of 0 is logged instead of checked. With --seconds S the run continues past that call;
// rt_frame_presented ends it once the world clock passed S.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cfg.h"
#include "cheats.h"
#include "fs.h"
#include "gfx.h"
#include "mods.h"
#include "nvn_api.h"
#include "platform/platform.h"
#include "auto_match.h"
#include "scene_start.h"
#include "vk.h"
#include "platform/platform.h"

#define SYM_INIT_START "_ZN2nn4init5StartEmmPFvvES2_S2_"
#define CHECKPOINT_RET rt_target.boot_checkpoint
#define NVN_RET rt_target.boot_nvn
#define CTOR_DONE 2                      // a module's constructor flag after its DT_INIT ran the init_array

static volatile int32_t inits_done, checkpoint_seen;
uint64_t rt_guest_start_ns;

static const RtLoaded *module_named(const char *name) {
    for (int i = 0; i < ldr_count(); i++)
        if (!strcmp(ldr_module(i)->name, name)) return ldr_module(i);
    rt_fatal("boot: module %s is not loaded", name);
}

static uint32_t ctor_state(const RtLoaded *m) {
    if (!m->ctor_flag) rt_fatal("boot: %s: its DT_INIT does not read a constructor flag the loader recognises", m->name);
    return LD32(m->ctor_flag);
}

static const char *caller(const Cpu *c, char *buf, int n) { return ldr_describe(c->x[30] - 4, buf, n); }

// main.npdm META: the main thread's priority (+0xE), core (+0xF) and stack size (+0x1C). kernel.c builds the main
// thread with the 1.2.1 values (Splatoon 3 11.2.0 asks for the same); a different npdm means a different game build.
// A target without a main.npdm hash (recomp/targets.py checks it by program id: re-signed dumps differ in its bytes)
// is checked by the program id of its ACI0 section.
static void check_npdm(void) {
    size_t n = 0;
    int hashed = 0;
    for (int i = 0; i < rt_exefs_sha256_count; i++) hashed |= !strcmp(rt_exefs_sha256[i].file, "main.npdm");
    uint8_t *npdm;
    if (hashed) {
        npdm = ldr_read_verified(rt_config()->exefs, "main.npdm", &n);
    } else {
        char path[PLAT_PATH_MAX * 3];
        uint64_t size = 0;
        snprintf(path, sizeof path, "%s/main.npdm", rt_config()->exefs);
        npdm = plat_file_read(path, &size);
        if (!npdm) rt_fatal("boot: cannot read %s", path);
        n = (size_t)size;
    }
    if (n < 0x80 || memcmp(npdm, "META", 4)) rt_fatal("boot: main.npdm has no META header");
    uint32_t stack, aci0;
    memcpy(&stack, npdm + 0x1C, 4);
    memcpy(&aci0, npdm + 0x70, 4);
    int prio = npdm[0xE], core = npdm[0xF];
    uint64_t program = 0;
    if ((uint64_t)aci0 + 0x18 <= n && !memcmp(npdm + aci0, "ACI0", 4)) memcpy(&program, npdm + aci0 + 0x10, 8);
    free(npdm);
    if (program != rt_target.program_id)
        rt_fatal("boot: main.npdm is program %016llX, this build is %s %s (%016llX)", (unsigned long long)program,
                 rt_target.game, rt_target.display_version, (unsigned long long)rt_target.program_id);
    if (prio != 44 || core != 0 || stack != 0x100000)
        rt_fatal("boot: main.npdm asks for priority %d, core %d, stack %#x; kernel.c builds 44, 0, 0x100000", prio,
                 core, stack);
}

// ---- the three callbacks of nn::init::Start (rtld's job on hardware)
static void cb_loader_ready(Cpu *c) {
    (void)c;
    rt_log("boot: rtld callback: modules linked");
}

// DT_INIT registers the module with musl (__nnmusl_init_dso) and runs its init_array; the constructor flag reads 2
// afterwards (MacNativeGames loader.c:458-462).
// The order is the target's (main, subsdk0, sdk for TotK), else the load order; the target's init_array sizes (TotK
// 1.1.0 and 1.2.1: counted in the modules; 1.4.3: DT_INIT_ARRAYSZ; MacNativeGames' boot check expects the same) are
// checked when it has them.
static void cb_module_inits(Cpu *c) {
    if (plat_cas32(&inits_done, 0, 0)) rt_fatal("boot: module constructors requested twice");
    int n = rt_target.ninits ? rt_target.ninits : ldr_count();
    for (int k = 0; k < n; k++) {
        const RtLoaded *m = rt_target.ninits ? module_named(rt_target.inits[k].module) : ldr_module(k);
        uint64_t entries = m->init_array_size / 8;
        if (!m->init) rt_fatal("boot: %s has no DT_INIT", m->name);
        rt_log("boot: ENTER %s DT_INIT (%llu init_array entries)", m->name, (unsigned long long)entries);
        rt_call(c, m->init, 0);
        uint32_t state = ctor_state(m);
        rt_logf("boot: DT_INIT %s state=%u (%llu)\n", m->name, state, (unsigned long long)entries);
        if (state != CTOR_DONE) rt_fatal("boot: %s DT_INIT left its constructor flag at %u, not 2", m->name, state);
        if (rt_target.ninits && entries != rt_target.inits[k].entries)
            rt_fatal("boot: %s has %llu init_array entries, %s %s has %llu", m->name, (unsigned long long)entries,
                     rt_target.game, rt_target.display_version, (unsigned long long)rt_target.inits[k].entries);
    }
    if (rt_target.ninits && rt_target.ninits != ldr_count())
        rt_fatal("boot: %d modules loaded, the target lists %d", ldr_count(), rt_target.ninits);
    plat_xchg32(&inits_done, 1);
}

static void cb_module_finis(Cpu *c) {
    char lr[128];
    rt_fatal("boot: the game asked for module finalizers (from %s): the guest exited", caller(c, lr, sizeof lr));
}

// ---- checkpoints (host.h)
static void stats_line(void) {
    RtStats s = rt_stats();
    rt_logf("boot: passed (%llu SVCs, %llu threads, %llu host calls)\n", (unsigned long long)s.svcs,
           (unsigned long long)s.threads, (unsigned long long)s.host_calls);
}

void rt_boot_event(const char *what, Cpu *c, const char *detail) {
    char at[128];
    const RtLoaded *main_mod = module_named("main");
    uint64_t ret = c->x[30];
    caller(c, at, sizeof at);
    if (!strcmp(what, "checkpoint")) {
        if (CHECKPOINT_RET && ret != main_mod->base + CHECKPOINT_RET)
            rt_fatal("boot: checkpoint %s reached from %s, expected main+%#llx", detail, at,
                     (unsigned long long)CHECKPOINT_RET);
        if (!plat_cas32(&inits_done, 0, 0)) rt_fatal("boot: checkpoint %s before DT_INIT", detail);
        for (int i = 0; i < ldr_count(); i++)
            if (ctor_state(ldr_module(i)) != CTOR_DONE)
                rt_fatal("boot: checkpoint %s: %s constructor flag reads %u", detail, ldr_module(i)->name,
                         ctor_state(ldr_module(i)));
        plat_xchg32(&checkpoint_seen, 1);
        rt_logf("boot: checkpoint %s from main+%#llx\n", detail, (unsigned long long)(ret - main_mod->base));
        return;
    }
    if (!strcmp(what, "nvn")) {
        if (NVN_RET && ret != main_mod->base + NVN_RET)
            rt_fatal("boot: first NVN call %s(\"%s\") from %s, expected main+%#llx", "nvnBootstrapLoader", detail, at,
                     (unsigned long long)NVN_RET);
        if (!plat_cas32(&checkpoint_seen, 0, 0)) {
            if (CHECKPOINT_RET) rt_fatal("boot: first NVN call before the SetFocusHandlingMode checkpoint");
            rt_log("boot: first NVN call before any SetFocusHandlingMode call");
        }
        rt_logf("boot: first NVN call nvnBootstrapLoader(\"%s\") from main+%#llx\n", detail,
               (unsigned long long)(ret - main_mod->base));
        if (rt_config()->seconds) return;   // --seconds: the run goes on into the world
        if (rt_trace & RT_TRACE_HOSTCALL) hostfn_report();
        stats_line();
        fflush(stdout);
        fflush(stderr);
        plat_exit(0);   // other guest threads are still running; nothing of theirs needs an orderly end
    }
    rt_fatal("boot: unknown boot event %s (%s) from %s", what, detail ? detail : "", at);
}

// ---- --seconds S
// rt_write_png (declared in vk.h) is shared with the TOTK_DUMP frame dump (vk_dump.c). No compression library is
// linked: each row gets the PNG filter with the smallest sum of residuals (the usual heuristic) and the rows are
// deflated with one dynamic Huffman block per MiB, literals only. A 1080p game frame comes out at about a third of the
// stored size. Stored blocks made every full-size shot 8.3 MB, and image viewers that cap a picture's size (5 MB is a
// common limit) re-encode such files with a 256-colour palette: smooth gradients then look posterised in the viewer
// while the file's pixels are smooth. Pictures whose alpha is 255 everywhere are written as RGB.
static uint32_t crc32_update(uint32_t crc, const uint8_t *p, size_t n) {
    static uint32_t table[256];
    if (!table[1])
        for (uint32_t i = 0; i < 256; i++) {
            uint32_t c = i;
            for (int k = 0; k < 8; k++) c = c & 1 ? 0xEDB88320u ^ (c >> 1) : c >> 1;
            table[i] = c;
        }
    for (size_t i = 0; i < n; i++) crc = table[(crc ^ p[i]) & 0xFF] ^ (crc >> 8);
    return crc;
}

static void png_chunk(FILE *f, const char *type, const uint8_t *data, uint32_t n) {
    uint8_t hdr[8] = {(uint8_t)(n >> 24), (uint8_t)(n >> 16), (uint8_t)(n >> 8), (uint8_t)n};
    memcpy(hdr + 4, type, 4);
    uint32_t crc = crc32_update(crc32_update(~0u, hdr + 4, 4), data, n) ^ ~0u;
    uint8_t tail[4] = {(uint8_t)(crc >> 24), (uint8_t)(crc >> 16), (uint8_t)(crc >> 8), (uint8_t)crc};
    fwrite(hdr, 1, 8, f);
    if (n) fwrite(data, 1, n, f);
    fwrite(tail, 1, 4, f);
}

typedef struct { uint8_t *p; uint64_t n; uint64_t bits; int nb; } BitOut;

static void bits_put(BitOut *o, uint32_t v, int n) {   // deflate order: least significant bit first
    o->bits |= (uint64_t)v << o->nb;
    o->nb += n;
    while (o->nb >= 8) o->p[o->n++] = (uint8_t)o->bits, o->bits >>= 8, o->nb -= 8;
}

static int by_weight(const void *a, const void *b) {
    uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
    return x < y ? -1 : x > y;
}

// Huffman code lengths for freq[0..n) (n <= 288; a zero frequency gets no code), none longer than maxlen: the
// frequencies are halved until the tree fits. A lone symbol gets length 1.
static void huff_lengths(const uint32_t *freq, int n, int maxlen, uint8_t *len) {
    uint64_t f[288];
    for (int i = 0; i < n; i++) f[i] = freq[i];
    for (;;) {
        uint64_t leaf[288];   // weight << 9 | symbol, sorted by weight
        int m = 0;
        for (int i = 0; i < n; i++)
            if (f[i]) leaf[m++] = f[i] << 9 | (uint64_t)i;
        memset(len, 0, (size_t)n);
        if (m == 1) len[leaf[0] & 511] = 1;
        if (m <= 1) return;
        qsort(leaf, (size_t)m, sizeof *leaf, by_weight);
        // Two-queue construction: nodes 0..m-1 are the leaves, m.. the internal nodes in creation order, whose
        // weights never decrease; a parent always has a higher index than its children.
        uint64_t w[2 * 288];
        int parent[2 * 288], li = 0, ni = m, nn = m;
        for (int i = 0; i < m; i++) w[i] = leaf[i] >> 9;
        while (nn < 2 * m - 1) {
            int pick[2];
            for (int k = 0; k < 2; k++) pick[k] = li < m && (ni >= nn || w[li] <= w[ni]) ? li++ : ni++;
            w[nn] = w[pick[0]] + w[pick[1]];
            parent[pick[0]] = parent[pick[1]] = nn++;
        }
        int depth[2 * 288], deepest = 0;
        depth[nn - 1] = 0;
        for (int i = nn - 2; i >= 0; i--) depth[i] = depth[parent[i]] + 1;
        for (int i = 0; i < m; i++) {
            len[leaf[i] & 511] = (uint8_t)depth[i];
            if (depth[i] > deepest) deepest = depth[i];
        }
        if (deepest <= maxlen) return;
        for (int i = 0; i < n; i++)
            if (f[i]) f[i] = (f[i] + 1) / 2;
    }
}

// Canonical codes for the lengths, bit-reversed for deflate's least-significant-bit-first order.
static void huff_codes(const uint8_t *len, int n, uint16_t *code) {
    int count[16] = {0}, next[16] = {0};
    for (int i = 0; i < n; i++) count[len[i]]++;
    count[0] = 0;
    for (int b = 1, c = 0; b < 16; b++) next[b] = c = (c + count[b - 1]) << 1;
    for (int i = 0; i < n; i++) {
        if (!len[i]) continue;
        uint32_t c = (uint32_t)next[len[i]]++, r = 0;
        for (int k = 0; k < len[i]; k++) r |= ((c >> k) & 1) << (len[i] - 1 - k);
        code[i] = (uint16_t)r;
    }
}

// One dynamic-Huffman deflate block of literals: 257 literal/length codes (no lengths used), one distance code of one
// bit (RFC 1951 3.2.7: a lone distance code may be incomplete), and the code lengths sent without repeat codes.
static void deflate_literals(BitOut *o, const uint8_t *p, uint64_t n, int last) {
    uint32_t freq[257] = {0}, clf[19] = {0};
    for (uint64_t i = 0; i < n; i++) freq[p[i]]++;
    freq[256] = 1;
    uint8_t len[258], cll[19];
    uint16_t code[257], clc[19];
    huff_lengths(freq, 257, 15, len);
    len[257] = 1;   // the distance code
    for (int i = 0; i < 258; i++) clf[len[i]]++;
    int used = 0;
    for (int i = 0; i < 19; i++) used += clf[i] != 0;
    if (used < 2) clf[clf[0] ? 1 : 0]++;   // the code-length code must be complete: never a lone symbol
    huff_lengths(clf, 19, 7, cll);
    huff_codes(len, 257, code);
    huff_codes(cll, 19, clc);
    static const uint8_t order[19] = {16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15};
    int hclen = 19;
    while (hclen > 4 && !cll[order[hclen - 1]]) hclen--;
    bits_put(o, (uint32_t)last, 1);
    bits_put(o, 2, 2);                       // dynamic Huffman codes
    bits_put(o, 0, 5);                       // HLIT: 257 literal/length codes
    bits_put(o, 0, 5);                       // HDIST: 1 distance code
    bits_put(o, (uint32_t)hclen - 4, 4);
    for (int i = 0; i < hclen; i++) bits_put(o, cll[order[i]], 3);
    for (int i = 0; i < 258; i++) bits_put(o, clc[len[i]], cll[len[i]]);
    for (uint64_t i = 0; i < n; i++) bits_put(o, code[p[i]], len[p[i]]);
    bits_put(o, code[256], len[256]);
}

static int paeth(int a, int b, int c) {
    int pa = abs(b - c), pb = abs(a - c), pc = abs(a + b - 2 * c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

void rt_write_png(const char *path, const uint8_t *rgba, uint32_t w, uint32_t h) {
    int opaque = 1;
    for (uint64_t i = 3; i < 4ull * w * h && opaque; i += 4) opaque = rgba[i] == 255;
    const uint32_t bpp = opaque ? 3 : 4;
    uint64_t stride = (uint64_t)bpp * w, row = 1 + stride, raw_n = row * h;
    uint8_t *raw = malloc(raw_n), *cur = malloc(stride), *prev = calloc(1, stride), *cand = malloc(stride);
    if (!raw || !cur || !prev || !cand) rt_fatal("boot: out of memory for the shot");
    for (uint32_t y = 0; y < h; y++) {
        const uint8_t *s = rgba + 4ull * w * y;
        for (uint32_t x = 0; x < w; x++) memcpy(cur + (uint64_t)bpp * x, s + 4ull * x, bpp);
        uint8_t *dst = raw + y * row;
        uint64_t best = UINT64_MAX;
        for (int f = 0; f < 5; f++) {   // none, sub, up, average, paeth
            uint64_t sum = 0;
            for (uint64_t i = 0; i < stride; i++) {
                int a = i >= bpp ? cur[i - bpp] : 0, b = prev[i], c = i >= bpp ? prev[i - bpp] : 0;
                int pr = f == 0 ? 0 : f == 1 ? a : f == 2 ? b : f == 3 ? (a + b) / 2 : paeth(a, b, c);
                cand[i] = (uint8_t)(cur[i] - pr);
                sum += (uint64_t)abs((int8_t)cand[i]);
            }
            if (sum < best) best = sum, dst[0] = (uint8_t)f, memcpy(dst + 1, cand, stride);
        }
        uint8_t *t = prev;
        prev = cur, cur = t;
    }
    free(cur), free(prev), free(cand);
    BitOut o = {.p = malloc(64 + raw_n * 2)};   // a literal takes at most 15 bits
    if (!o.p) rt_fatal("boot: out of memory for the shot");
    o.p[o.n++] = 0x78, o.p[o.n++] = 0x01;
    const uint64_t blk = 1u << 20;
    for (uint64_t off = 0; off < raw_n; off += blk)
        deflate_literals(&o, raw + off, raw_n - off < blk ? raw_n - off : blk, off + blk >= raw_n);
    if (o.nb) bits_put(&o, 0, 8 - o.nb);
    uint8_t *z = o.p;
    uint64_t zn = o.n;
    uint32_t a = 1, b = 0;
    for (uint64_t off = 0; off < raw_n; off += 5552) {   // Adler-32, reduced every 5552 bytes as zlib does
        uint64_t k = raw_n - off < 5552 ? raw_n - off : 5552;
        for (uint64_t i = 0; i < k; i++) a += raw[off + i], b += a;
        a %= 65521, b %= 65521;
    }
    uint32_t ad = b << 16 | a;
    z[zn++] = (uint8_t)(ad >> 24), z[zn++] = (uint8_t)(ad >> 16), z[zn++] = (uint8_t)(ad >> 8), z[zn++] = (uint8_t)ad;
    FILE *f = fopen(path, "wb");
    if (!f) rt_fatal("boot: cannot write %s", path);
    static const uint8_t sig[8] = {0x89, 'P', 'N', 'G', 13, 10, 26, 10};
    uint8_t ihdr[13] = {(uint8_t)(w >> 24), (uint8_t)(w >> 16), (uint8_t)(w >> 8), (uint8_t)w,
                        (uint8_t)(h >> 24), (uint8_t)(h >> 16), (uint8_t)(h >> 8), (uint8_t)h, 8, opaque ? 2 : 6, 0, 0,
                        0};
    fwrite(sig, 1, 8, f);
    png_chunk(f, "IHDR", ihdr, 13);
    png_chunk(f, "IDAT", z, (uint32_t)zn);
    png_chunk(f, "IEND", NULL, 0);
    if (fclose(f)) rt_fatal("boot: cannot write %s", path);
    free(z);
    free(raw);
}

static void save_shot(const tex_t *t, const char *path) {
    int fmt = t->d.format;
    int bgra = fmt == NVN_FORMAT_BGRA8 || fmt == NVN_FORMAT_BGRA8_SRGB;
    if (!bgra && fmt != NVN_FORMAT_RGBA8 && fmt != NVN_FORMAT_RGBA8_SRGB)
        rt_fatal("boot: --shot supports 8-bit RGBA and BGRA targets, the presented texture has NVN format %#x", fmt);
    uint32_t w = t->d.width, h = t->d.height;
    uint8_t *px = malloc(4ull * w * h);
    if (!px) rt_fatal("boot: out of memory for the shot");
    view_t view;
    memset(&view, 0, sizeof view);
    NVNcopyRegion r = {.width = (int)w, .height = (int)h, .depth = 1};
    if (gfx_replay_async()) gfx_backend()->finish();   // the presented frame may still be queued for the replay
    gfx_backend()->read_texels(t, &view, &r, px, 4ull * w, 4ull * w * h);
    if (bgra)
        for (uint64_t i = 0; i < 4ull * w * h; i += 4) {
            uint8_t x = px[i];
            px[i] = px[i + 2];
            px[i + 2] = x;
        }
    rt_write_png(path, px, w, h);
    free(px);
    rt_logf("boot: shot %s (%ux%u)\n", path, w, h);
}

static void shot_scaled(const tex_t *t, const char *path, int full);

// TOTK_SHOTS=lo-hi:step [TOTK_SHOTS_DIR=dir]: every step-th present in [lo, hi] saved as <dir>/f<n>.png (n the
// present), box-filtered to 480x270; TOTK_SHOTS=w<lo>-<hi>:<step> takes them by world-clock seconds instead, the first
// present at or after each step (full size with TOTK_SHOTS_FULL=1), for finding short glitches. Pipeline builds stay asynchronous, so the
// pictures show what a player sees.
static void series_shot(const tex_t *t, uint64_t n) {
    static int parsed;
    static unsigned long long lo, hi, step, next;
    static const char *dir;
    static int full, world;
    if (!parsed) {
        full = cfg_get("TOTK_SHOTS_FULL") != NULL;
        const char *e = cfg_get("TOTK_SHOTS");
        double wl, wh, ws;
        if (e && e[0] == 'w' && sscanf(e + 1, "%lf-%lf:%lf", &wl, &wh, &ws) == 3 && ws > 0)   // world-clock seconds
            world = 1, lo = (unsigned long long)(wl * 1e6), hi = (unsigned long long)(wh * 1e6),
            step = (unsigned long long)(ws * 1e6), next = lo;
        else if (!e || sscanf(e, "%llu-%llu:%llu", &lo, &hi, &step) != 3 || !step) hi = 0;
        dir = cfg_get("TOTK_SHOTS_DIR") ? cfg_get("TOTK_SHOTS_DIR") : ".";
        if (hi) plat_file_mkdir_p(dir);
        parsed = 1;
    }
    if (world) {   // the first present at or after each step of the world clock
        int64_t w = frame_step_world_us();
        if (w < 0 || (unsigned long long)w < next || next > hi) return;
        while (next <= (unsigned long long)w) next += step;
    } else if (n < lo || n > hi || (n - lo) % step) {
        return;
    }
    char path[PLAT_PATH_MAX];
    snprintf(path, sizeof path, "%s/f%05llu.png", dir, (unsigned long long)n);
    shot_scaled(t, path, full);
}

void rt_shot_small(const tex_t *t, const char *path) { shot_scaled(t, path, 0); }

// The presented texture t box-filtered to 480x270 (full: at its own size) into path, its mean colour logged.
static void shot_scaled(const tex_t *t, const char *path, int full) {
    int fmt = t->d.format;
    int bgra = fmt == NVN_FORMAT_BGRA8 || fmt == NVN_FORMAT_BGRA8_SRGB;
    if (!bgra && fmt != NVN_FORMAT_RGBA8 && fmt != NVN_FORMAT_RGBA8_SRGB) return;
    uint32_t w = t->d.width, h = t->d.height, ow = full ? w : 480, oh = full ? h : 270;
    uint8_t *px = malloc(4ull * w * h), *out = malloc(4ull * ow * oh);
    if (!px || !out) rt_fatal("boot: out of memory for a TOTK_SHOTS frame");
    view_t view;
    memset(&view, 0, sizeof view);
    NVNcopyRegion r = {.width = (int)w, .height = (int)h, .depth = 1};
    if (gfx_replay_async()) gfx_backend()->finish();   // the presented frame may still be queued for the replay
    gfx_backend()->read_texels(t, &view, &r, px, 4ull * w, 4ull * w * h);
    for (uint32_t y = 0; y < oh; y++)
        for (uint32_t x = 0; x < ow; x++) {
            uint32_t x0 = x * w / ow, x1 = (x + 1) * w / ow, y0 = y * h / oh, y1 = (y + 1) * h / oh;
            if (x1 <= x0) x1 = x0 + 1;
            if (y1 <= y0) y1 = y0 + 1;
            uint32_t sum[4] = {0}, k = 0;
            for (uint32_t sy = y0; sy < y1; sy++)
                for (uint32_t sx = x0; sx < x1; sx++, k++)
                    for (int c = 0; c < 4; c++) sum[c] += px[4ull * (sy * (uint64_t)w + sx) + (uint32_t)c];
            uint8_t *o = out + 4ull * (y * ow + x);
            for (int c = 0; c < 4; c++) o[c] = (uint8_t)(sum[bgra && c != 3 ? 2 - c : c] / k);
            o[3] = 255;
        }
    rt_write_png(path, out, ow, oh);
    // The picture's mean colour in the log: a scripted run whose whole 3D scene came out darker (a dark, red-tinted
    // character creation at about a half of the usual mean was seen in one run of an earlier build, not reproduced
    // since) shows in the log of every run that takes shots, without opening the pictures.
    uint64_t sum[3] = {0};
    for (uint64_t i = 0; i < 4ull * ow * oh; i += 4) sum[0] += out[i], sum[1] += out[i + 1], sum[2] += out[i + 2];
    double np = (double)ow * oh;
    rt_log("boot: shot %s: mean RGB %.1f %.1f %.1f", path, sum[0] / np, sum[1] / np, sum[2] / np);
    free(out);
    free(px);
}

// A requested shot (rt_shot_request): the path waits here until the next present takes it.
static char *shot_pending;

void rt_shot_request(const char *png) {
    char *p = strdup(png);
    if (p) free(__atomic_exchange_n(&shot_pending, p, __ATOMIC_ACQ_REL));
}

// A --seconds run ends at the first present after the world clock (game time since the save's first frame,
// frame_step_world_us) passed S seconds, so a run covers the same game time at any frame rate; any other frame is
// counted by the back end only.
void rt_frame_presented(const tex_t *t, uint64_t n) {
    const RtConfig *cfg = rt_config();
    series_shot(t, n);
    if (__atomic_load_n(&shot_pending, __ATOMIC_RELAXED)) {
        char *p = __atomic_exchange_n(&shot_pending, NULL, __ATOMIC_ACQ_REL);
        if (p) {
            int fmt = t->d.format;
            if (fmt == NVN_FORMAT_BGRA8 || fmt == NVN_FORMAT_BGRA8_SRGB || fmt == NVN_FORMAT_RGBA8 ||
                fmt == NVN_FORMAT_RGBA8_SRGB) {
                char *sep = strrchr(p, '/'), *bs = strrchr(p, '\\');
                if (bs > sep) sep = bs;
                if (sep) {   // the folder may not exist yet
                    char c0 = *sep;
                    *sep = 0;
                    plat_file_mkdir_p(p);
                    *sep = c0;
                }
                save_shot(t, p);   // logs "boot: shot <path>"
            } else {
                rt_log("boot: shot %s skipped: the presented texture has NVN format %#x", p, fmt);
            }
            free(p);
        }
    }
    int64_t w = cfg->seconds ? frame_step_world_us() : -1;
    if (w < 0 || (double)w < cfg->seconds * 1e6) return;
    if (cfg->shot) save_shot(t, cfg->shot);
    GfxStats g = gfx_stats();
    rt_logf("boot: frames %llu presented (nvn calls %llu, distinct %llu, unimplemented %llu, validation errors %llu, "
           "shader failures %llu, draws skipped %llu)\n", (unsigned long long)g.frames,
           (unsigned long long)g.nvn_calls, (unsigned long long)g.nvn_distinct, (unsigned long long)g.unimplemented,
           (unsigned long long)g.validation_errors, (unsigned long long)g.shader_failures,
           (unsigned long long)g.draws_skipped);
    stats_line();
    fflush(stdout);
    fflush(stderr);
    fs_save_flush();
    fs_match_end();
    vk_pipeline_save();   // no-op without the Vulkan back end
    playtime_save();
    prof_write();
    fflush(stderr);
    plat_exit(0);
}

// ---- steps 9-14
static uint64_t cb_addr[3];

static void guest_main(Cpu *c) {
    uint32_t handle = kern_main_thread(&c);
    uint64_t start = ldr_symbol(SYM_INIT_START);
    if (!start) rt_fatal("boot: %s is not exported", SYM_INIT_START);
    char d[128];
    rt_log("boot: ENTER nn::init::Start at %s, main thread %#x", ldr_describe(start, d, sizeof d), handle);
    // nn::init::Start(main thread handle, arguments region, loader_ready, module_inits, module_finis)
    rt_call(c, start, 5, (uint64_t)handle, (uint64_t)0, cb_addr[0], cb_addr[1], cb_addr[2]);
    rt_fatal("boot: nn::init::Start returned");
}

// The prewarm (shaders and pipelines recorded in earlier runs) runs on its own thread while the modules load, and the
// guest waits for it at most TOTK_PREWARM_WAIT seconds (default 0: not at all; "full": until it is done, as before).
// Its builds then go on behind the game at a lower priority (vk_pipeline_guest_starts), and a draw that needs one of
// its pipelines first gets it built ahead of the rest. A player on a hard disk with an AMD card waited 62 s here
// (26 s reading 2,478 loose shader files, 31 s for the driver to build 2,428 pipelines) before the game even opened
// its window, while the game itself needs a few seconds of its own before its first draw.
static PlatMutex warm_lock = PLAT_MUTEX_INIT;
static PlatCond warm_cv = PLAT_COND_INIT;
static int warm_done;

static void prewarm_main(void *arg) {
    (void)arg;
    vk_pipeline_prewarm();
    const char *prep = cfg_get("TOTK_TEST_PREPARE_SHADERS");   // Task 2.3 test hook; Task 2.6 builds the preparation
    if (prep && !strcmp(prep, "1")) {
        int cpus = (int)plat_cpu_usable();
        gfx_shader_prepare_recorded(cpus, cpus / 2 > 16 ? 16 : cpus / 2 < 1 ? 1 : cpus / 2);
    }
    plat_mutex_lock(&warm_lock);
    warm_done = 1;
    plat_cond_broadcast(&warm_cv);
    plat_mutex_unlock(&warm_lock);
    return;
}

// Milliseconds the guest waits for the prewarm at most; UINT32_MAX: until it is done.
static uint32_t prewarm_wait_ms(void) {
    const char *e = cfg_get("TOTK_PREWARM_WAIT");
    if (!e || !*e) return 0;
    if (!strcmp(e, "full")) return UINT32_MAX;
    double s = atof(e);
    return s <= 0 ? 0 : s >= 3600 ? 3600000u : (uint32_t)(s * 1000);
}

void rt_boot(void) {
    rt_log("boot: %s %s (target %s, program %016llX)", rt_target.game, rt_target.display_version, rt_target.name,
           (unsigned long long)rt_target.program_id);
    check_npdm();
    if (!rt_config()->shot) vk_pipeline_async_enable();   // a --shot image must not depend on build timing
    // Shaders and pipelines recorded in earlier runs are made ready while the modules load (prewarm_main).
    int warm = plat_thread_spawn(prewarm_main, NULL, "prewarm", 0);
    lan_install();   // SPLAT3_LAN: LAN play's overrides of nn::socket and nn::nifm (none without it)
    ldr_load(rt_config()->exefs);
    rt_patches_init();
    mods_exefs_switch();   // after the patch sets' settings: an enabled mod's ExeFS patches on
    frame_step_init();
    res_init();
    scene_start_init();   // Splatoon 3: start_scene / SPLAT3_START, TOTK_FN_TRACE (function-entry hooks)
    automatch_init();     // Splatoon 3: SPLAT3_MATCH (automatic LAN match entry, the match result)
    cheats_init();   // after the patch sets: a cheat's set follows its setting
    cb_addr[0] = hostfn_register("<rtld loader_ready>", cb_loader_ready);
    cb_addr[1] = hostfn_register("<rtld module_inits>", cb_module_inits);
    cb_addr[2] = hostfn_register("<rtld module_finis>", cb_module_finis);
    hostfn_seal();
    uint64_t held = plat_now_ms();
    uint32_t cap = prewarm_wait_ms();
    if (!warm) prewarm_main(NULL);   // no thread: the old way, in full before the guest
    plat_mutex_lock(&warm_lock);
    for (uint64_t now; !warm_done && (cap == UINT32_MAX || (now = plat_now_ms()) - held < cap);)
        plat_cond_wait_ms(&warm_cv, &warm_lock, cap == UINT32_MAX ? 1000 : (uint32_t)(cap - (now - held)) + 1);
    int done = warm_done;
    plat_mutex_unlock(&warm_lock);
    vk_pipeline_guest_starts();
    held = plat_now_ms() - held;
    // The read counts tell a slow start on a hard disk from a busy CPU: every loose cache file is a seek there.
    uint64_t rd = 0, rb = 0;
    GfxShaderDisk d;
    gfx_shader_disk_stats(&d);
    plat_io_counters(&rd, &rb);
    rt_log("boot: guest starts %llu ms after process start (%llu ms of it waiting for the prewarm, which %s; "
           "prewarm_wait %s); so far %llu read requests (%.0f MB) by the process, %llu shader cache files read or looked "
           "up, %llu shader cache pack reads", (unsigned long long)(plat_process_uptime_ns() / 1000000),
           (unsigned long long)held, done ? "is done" : "goes on behind the game", cap == UINT32_MAX ? "full" :
           cfg_get("TOTK_PREWARM_WAIT") && *cfg_get("TOTK_PREWARM_WAIT") ? cfg_get("TOTK_PREWARM_WAIT") : "0",
           (unsigned long long)rd, rb / 1e6, (unsigned long long)d.opens, (unsigned long long)d.pack_reads);
    rt_guest_start_ns = kern_now_ns();
    kern_run_on_guest_main(guest_main);
    rt_fatal("boot: the guest main thread ended");
}
