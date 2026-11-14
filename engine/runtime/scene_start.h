// Splatoon 3's start scene and the function-entry trace (scene_start.c).
#pragma once
#include <stdint.h>

void scene_start_init(void);                     // boot.c, after ldr_load: start_scene / SPLAT3_START, TOTK_FN_TRACE
void scene_fn_hit(uint64_t addr, uint64_t lr, uint64_t x0);   // main.c fnprof_hit, while FNPROF_SCENE or FNPROF_TRACE
const char *scene_start_value(void);             // the start_scene value in effect, NULL when none
void fn_census_mark(const char *name);           // TOTK_FN_CENSUS: end the current window, start window `name`
                                                 // ("off": none)
