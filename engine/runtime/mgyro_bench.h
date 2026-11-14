// TOTK_MGYRO_BENCH: a test bench for the mouse as the six-axis sensor (mgyro_bench.c, docs/controls.md "Mouse as the
// gyro: measurements"). While it runs, a simulated mouse sends counts into the motion thread in place of the real mouse
// and the input script, the player's own input is ignored, and every six-axis read of the game, every sample made and
// every present go to a CSV file with the game's aim direction read from its memory.
#pragma once
#include <stdint.h>

#include "input.h"

// 1 when TOTK_MGYRO_BENCH is set (read once): every hook below returns at once otherwise.
int mgb_on(void);
// Motion thread (input.c mouse_gyro_collect): the counts the simulated mouse sent since the last call.
void mgb_take(uint64_t now, double *dx, double *dy);
// input.c: the yaw axis of the mouse gyro's states (MouseGyroCfg.yaw_body): def, unless a bench test sets it.
int mgb_yaw_body(int def);
// input.c: 1 when the game's Y levels the virtual controller too: def, unless the recenter test sets it.
int mgb_level_on_y(int def);
// input.c: the reads held still at a reversal of the yaw (mgyro_frame_yaw): def, unless a bench test sets it.
int mgb_reverse_hold(int def);
// Motion thread (input.c mouse_gyro_tick): the samples about to be pushed, oldest first. The bench numbers and logs
// them, and its field tests may change what they report.
void mgb_samples(MotionState *s, int n, uint64_t now);
// Game thread (ov_input.c six_fill): the states the game is handed, newest first, n of them live (0: at rest).
void mgb_read(const MotionState *s, int n, uint64_t now);
// Game thread (nvn_api_obj.c nvnQueuePresentTexture, after the back end took it): the presented texture (a tex_t),
// the game's present number n, when the game presented and the display time the present is held to.
void mgb_present(const void *tex, uint64_t n, uint64_t now, uint64_t slot);
// Present thread (vk_backend.c present_main): the host present of the frame held to display time slot went out at now.
void mgb_flip(uint64_t slot, uint64_t now);
