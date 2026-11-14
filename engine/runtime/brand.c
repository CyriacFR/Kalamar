// The port's identity for the running game target (brand.h). The only place where the neutral name is chosen:
// change KLM_NAME to rename the port. Nothing here is the name or artwork of a game or of its publisher.
#include <string.h>

#include "brand.h"
#include "host.h"
#include "platform/platform.h"

#define KLM_NAME "Kalamar"

static const RtBrand g_upstream = {"TotK Recomp", "totk", "TOTK_", "totk.ini", "TotKRecompWindow", 1};
static const RtBrand g_neutral = {KLM_NAME, "kalamar", "KLM_", "kalamar.ini", "KalamarWindow", 0};

const RtBrand *rt_brand(void) { return rt_target.family == RT_GAME_TOTK ? &g_upstream : &g_neutral; }

const char *rt_brand_game_version(void) { return rt_target.display_version; }

static int word_char(char c) { return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_'; }

size_t rt_brand_vars(char *s, size_t n) {
    const RtBrand *b = rt_brand();
    size_t pl = strlen(b->prefix);
    if (b->upstream || pl > 5) return n;
    static const char *const older[] = {"TOTK_", "SPLAT3_"};
    size_t o = 0;
    for (size_t i = 0; i < n;) {
        size_t k = 0;
        if (!i || !word_char(s[i - 1]))
            for (size_t j = 0; j < sizeof older / sizeof *older && !k; j++) {
                size_t l = strlen(older[j]);
                char c = i + l < n ? s[i + l] : 0;
                if (i + l < n && !memcmp(s + i, older[j], l) && ((c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9'))) k = l;
            }
        if (k) {
            memmove(s + o, b->prefix, pl);   // o <= i and pl <= k: never past what is still to read
            o += pl;
            i += k;
        } else {
            s[o++] = s[i++];
        }
    }
    return o;
}

char *rt_brand_text(const char *in, char *out, size_t n) {
    const RtBrand *b = rt_brand();
    if (!n) return out;
    if (b->upstream) {
        strncpy(out, in, n - 1);
        out[n - 1] = 0;
        return out;
    }
    static char exe[260];
    if (!exe[0]) {
        char path[1024];
        const char *base = "";
        if (plat_exe_path(path, sizeof path)) {
            char *cut = plat_last_sep(path);
            base = cut ? cut + 1 : path;
        }
        strncpy(exe, *base ? base : "game.exe", sizeof exe - 1);
    }
    const struct { const char *from, *to; } map[] = {
        {"TotK Recomp", b->name}, {"totk.ini", b->ini}, {"totk.exe", exe}, {"TOTK_", b->prefix}, {"SPLAT3_", b->prefix},
    };
    size_t o = 0;
    while (*in && o + 1 < n) {
        size_t k = 0, len = 0;
        for (; k < sizeof map / sizeof *map; k++) {
            len = strlen(map[k].from);
            if (!strncmp(in, map[k].from, len)) break;
        }
        if (k == sizeof map / sizeof *map) {
            out[o++] = *in++;
            continue;
        }
        for (const char *t = map[k].to; *t && o + 1 < n; t++) out[o++] = *t;
        in += len;
    }
    out[o] = 0;
    return out;
}
