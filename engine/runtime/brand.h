// The port's identity as the player meets it (brand.c): window title and class, settings file, environment prefix.
// One brand per game target: the upstream game's keeps its own names, any other target gets the neutral one.
#pragma once
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    const char *name;       // window title, menu header, system info: "Kalamar"
    const char *id;         // short lower-case id: Wayland app id
    const char *prefix;     // environment prefix shown to the player; the older ones ("TOTK_", "SPLAT3_") work too
    const char *ini;        // settings file name, read first
    const char *wndclass;   // Win32 window class of the game window
    int upstream;           // 1: the upstream game's own target (its existing strings and icon)
} RtBrand;

const RtBrand *rt_brand(void);

// Copy in to out (n bytes) with the upstream names replaced by the brand's: "TotK Recomp", "totk.ini", "totk.exe",
// "TOTK_", "SPLAT3_". Text of an upstream target comes out unchanged. Returns out.
char *rt_brand_text(const char *in, char *out, size_t n);
// The runtime's variables by their documented names in the n bytes at s, in place (the new prefix is shorter): KLM_X
// where the code wrote TOTK_X or SPLAT3_X (cfg.h cfg_env), for a name in capitals that starts a word. The log lines
// (main.c rt_log) and the automatic match's error reasons go through it. Unchanged on an upstream target. Returns the
// new length.
size_t rt_brand_vars(char *s, size_t n);
// The game update of the running target ("1.4.3", "11.2.0").
const char *rt_brand_game_version(void);

#ifdef __cplusplus
}
#endif
