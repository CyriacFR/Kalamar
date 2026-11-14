// JSON string escaping, the one every JSON text the runtime writes by hand goes through: the automatic match's status
// and result files (auto_match.c), the Discord IPC frames (discord_rpc.c), the shader translator's requests
// (gfx_shader.c) and --list-match-options (match_options.c). Portable C, no OS calls.
//
// A JSON string (RFC 8259, section 7) may not hold '"', '\' or a control character below U+0020 as such: they are
// escaped (a backslash before '"' and '\', the \u00XX form for the others; DEL too, for the log viewers). Text from
// outside the runtime (the stage name a room's host sends, a player name read from the game, a translation) may hold
// any bytes: each byte that does not begin a well-formed UTF-8 sequence (a stray continuation byte, an overlong form, a
// surrogate, a code point past U+10FFFF, a sequence cut short) becomes U+FFFD, written as its \u escape, so the text
// stays valid UTF-8 and no byte can end the string early. The output is cut only between whole characters and escapes,
// and is always NUL-terminated.
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

enum { JSON_ESC_UTF8 = 0, JSON_ESC_ASCII = 1 };   // ASCII: every character past U+007E as \uXXXX (pairs above U+FFFF)

// The well-formed UTF-8 sequence (Unicode's table 3-7) at s, of at most n bytes: its length (1-4) and its code point in
// *cp; 0 when s does not begin one. Stops at the first byte that does not fit, so it never reads past a NUL.
static inline int json_utf8_seq(const unsigned char *s, size_t n, uint32_t *cp) {
    unsigned c = s[0], lo = 0x80, hi = 0xbf;
    int k;
    uint32_t v;
    if (c < 0x80) return *cp = c, 1;
    if (c >= 0xc2 && c <= 0xdf) k = 2, v = c & 0x1f;
    else if (c >= 0xe0 && c <= 0xef) k = 3, v = c & 0x0f, lo = c == 0xe0 ? 0xa0 : 0x80, hi = c == 0xed ? 0x9f : 0xbf;
    else if (c >= 0xf0 && c <= 0xf4) k = 4, v = c & 0x07, lo = c == 0xf0 ? 0x90 : 0x80, hi = c == 0xf4 ? 0x8f : 0xbf;
    else return 0;
    for (int i = 1; i < k; i++) {
        if ((size_t)i >= n) return 0;
        unsigned b = s[i];
        if (b < (i == 1 ? lo : 0x80) || b > (i == 1 ? hi : 0xbf)) return 0;
        v = v << 6 | (b & 0x3f);
    }
    *cp = v;
    return k;
}

// One character of s (at most n bytes) as JSON string contents into e (13 bytes at most, no NUL), its size in *len;
// returns the bytes of s it stands for.
static inline size_t json_esc_char(const unsigned char *s, size_t n, int flags, char e[13], size_t *len) {
    static const char hex[] = "0123456789abcdef";
    uint32_t cp;
    int k = json_utf8_seq(s, n, &cp), bad = !k;
    if (bad) cp = 0xfffd, k = 1;
    if (cp == '"' || cp == '\\') {
        e[0] = '\\', e[1] = (char)cp, *len = 2;
    } else if (cp >= 0x20 && cp < 0x7f) {
        e[0] = (char)cp, *len = 1;
    } else if (!bad && cp >= 0x80 && !(flags & JSON_ESC_ASCII)) {
        memcpy(e, s, (size_t)k), *len = (size_t)k;
    } else {
        uint32_t u[2] = {cp, 0};
        int units = 1;
        if (cp >= 0x10000) u[0] = 0xd800 + ((cp - 0x10000) >> 10), u[1] = 0xdc00 + ((cp - 0x10000) & 0x3ff), units = 2;
        *len = 0;
        for (int i = 0; i < units; i++) {
            char *o = e + *len;
            o[0] = '\\', o[1] = 'u';
            for (int d = 0; d < 4; d++) o[2 + d] = hex[(u[i] >> (12 - 4 * d)) & 15];
            *len += 6;
        }
    }
    return (size_t)k;
}

// s, up to n bytes or its NUL, as JSON string contents (without the quotes) into out, NUL-terminated within cap bytes;
// returns the bytes written before the NUL. flags: JSON_ESC_UTF8 or JSON_ESC_ASCII.
static inline size_t json_esc_n(char *out, size_t cap, const char *s, size_t n, int flags) {
    size_t o = 0;
    if (!cap) return 0;
    const unsigned char *p = (const unsigned char *)s;
    for (size_t i = 0; p && i < n && p[i];) {
        char e[13];
        size_t len, used = json_esc_char(p + i, n - i, flags, e, &len);
        if (o + len >= cap) break;
        memcpy(out + o, e, len);
        o += len, i += used;
    }
    out[o] = 0;
    return o;
}

static inline size_t json_esc(char *out, size_t cap, const char *s) { return json_esc_n(out, cap, s, (size_t)-1, JSON_ESC_UTF8); }

// s as JSON string contents into the file f, whole
static inline void json_fput(FILE *f, const char *s) {
    const unsigned char *p = (const unsigned char *)s;
    for (size_t i = 0; p && p[i];) {
        char e[13];
        size_t len;
        i += json_esc_char(p + i, (size_t)-1 - i, JSON_ESC_UTF8, e, &len);
        fwrite(e, 1, len, f);
    }
}

// JE(s): an escaped copy of s for one printf argument ("\"stage\": \"%s\"", JE(scene)), held by a compound literal of
// the enclosing block. Strings longer than JSON_ESC_TMP once escaped are cut (between whole characters).
#define JSON_ESC_TMP 1024
static inline const char *json_esc_tmp(char *buf, size_t cap, const char *s) {
    json_esc(buf, cap, s);
    return buf;
}
#define JE(s) json_esc_tmp((char[JSON_ESC_TMP]){0}, JSON_ESC_TMP, (s))
