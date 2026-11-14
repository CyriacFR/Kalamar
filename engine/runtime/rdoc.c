// RenderDoc capture mode (debug tool), TOTK_RDOC. RenderDoc's Vulkan layer hides VK_EXT_external_memory_host from
// the application (RenderDoc 1.47; its capture option 12, AllowUnsupportedVendorExtensions, does not bring it back),
// and the back end imports the guest's CPU-visible pools with it. Any TOTK_RDOC but empty or 0 turns on capture mode:
// - those pools get shadow storage (vk_memory.c, vk_mem_shadow_wanted): Vulkan memory of their own, which the pages
//   the CPU wrote are copied into before each submit and the GPU's writes copied back from at its fence, so the device
//   needs no host-memory import (vk_device.c) and every byte a draw reads lies in memory RenderDoc records;
// - when the process already holds renderdoc.dll (the game was started by `renderdoccmd capture` or from qrenderdoc),
//   the in-application API captures what TOTK_RDOC lists, comma-separated: <n> the n-th presented frame, w<s> the first
//   frame presented after the world clock (frame_step_world_us) passed s seconds, boot the work before the first
//   present (from the device's creation, rdoc_device_ready); anything else (say "keys") captures
//   only on RenderDoc's own keys, F12 and Print Screen. The library is never loaded here: without it the mode changes
//   the memory path only, which is how its pictures are compared with a normal run's.
// TOTK_RDOC_DIR: the captures' folder (<dir>/splat3_frame<n>.rdc); unset, the launcher's file name template stays.
// Only the API fields used are declared here (layout of RENDERDOC_API_1_4_1, renderdoc_app.h, MIT), so no RenderDoc
// header is vendored.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cfg.h"
#include "host.h"
#include "vk.h"
#include "platform/platform.h"

#define RDOC_API_1_4_1 10401
#define RDOC_MAX_TARGETS 64
#ifdef _WIN32
#define RDOC_LIB "renderdoc.dll"
#else
#define RDOC_LIB "librenderdoc.so"
#endif

typedef struct {
    void (*GetAPIVersion)(int *major, int *minor, int *patch);
    void *fn1_10[10];   // SetCaptureOptionU32 .. UnloadCrashHandler
    void (*SetCaptureFilePathTemplate)(const char *);
    void *GetCaptureFilePathTemplate;
    uint32_t (*GetNumCaptures)(void);
    uint32_t (*GetCapture)(uint32_t idx, char *filename, uint32_t *pathlength, uint64_t *timestamp);
    void *TriggerCapture, *IsTargetControlConnected, *LaunchReplayUI, *SetActiveWindow;
    void (*StartFrameCapture)(void *device, void *window);
    uint32_t (*IsFrameCapturing)(void);
    uint32_t (*EndFrameCapture)(void *device, void *window);
} RdocApi;

typedef struct {
    uint64_t frame;     // presented frame to capture, 0: none
    int64_t world_us;   // or the world clock to capture at, -1: none
    int done;
} RdocTarget;

static RdocApi *api;
static int mode = -1;   // capture mode, decided at the first rdoc_capture_mode
static RdocTarget targets[RDOC_MAX_TARGETS];
static int n_targets;
static int capturing;
static uint64_t capture_frame;

int rdoc_capture_mode(void) {
    if (mode < 0) {
        const char *e = cfg_get("TOTK_RDOC");
        mode = e && *e && strcmp(e, "0") != 0;
    }
    return mode;
}

int rdoc_active(void) { return api != NULL; }

static int boot_capture;   // TOTK_RDOC lists "boot": the work before the first present, from the device's creation on

static void parse_targets(const char *e) {
    for (const char *p = e; *p && n_targets < RDOC_MAX_TARGETS;) {
        char *end;
        RdocTarget t = {.world_us = -1};
        if (!strncmp(p, "boot", 4) && (p[4] == ',' || !p[4])) {
            boot_capture = 1;
        } else if (*p == 'w') {
            double s = strtod(p + 1, &end);
            if (end != p + 1 && s >= 0) t.world_us = (int64_t)(s * 1e6);
        } else {
            t.frame = strtoull(p, &end, 10);
            if (end == p) t.frame = 0;
        }
        if (t.frame || t.world_us >= 0) targets[n_targets++] = t;
        p = strchr(p, ',');
        if (!p) break;
        p++;
    }
}

void rdoc_init(void) {
    if (!rdoc_capture_mode() || api) return;
    const char *e = cfg_get("TOTK_RDOC");
    void *m = plat_lib_loaded(RDOC_LIB);
    if (!m) {
        rt_log("rdoc: capture mode (TOTK_RDOC=%s): the pools get shadow storage; %s is not in the process, so "
               "nothing is captured (start the game with renderdoccmd capture or from qrenderdoc)", e, RDOC_LIB);
        return;
    }
    int (*get_api)(int, void **) = (int (*)(int, void **))plat_lib_sym(m, "RENDERDOC_GetAPI");
    void *a = NULL;
    if (!get_api || get_api(RDOC_API_1_4_1, &a) != 1 || !a) { rt_log("rdoc: RENDERDOC_GetAPI(1.4.1) failed"); return; }
    api = a;
    parse_targets(e);
    int major = 0, minor = 0, patch = 0;
    api->GetAPIVersion(&major, &minor, &patch);
    const char *dir = cfg_get("TOTK_RDOC_DIR");
    char full[PLAT_PATH_MAX], tmpl[PLAT_PATH_MAX + 16];
    if (dir && *dir && plat_full_path(dir, full, sizeof full)) {
        plat_file_mkdir_p(full);
        snprintf(tmpl, sizeof tmpl, "%s/splat3", full);
        api->SetCaptureFilePathTemplate(tmpl);
    }
    rt_log("rdoc: capture mode with RenderDoc (API %d.%d.%d): %d capture(s) listed in TOTK_RDOC=%s, F12 or Print "
           "Screen for more; captures go to %s", major, minor, patch, n_targets, e,
           dir && *dir ? full : "the launcher's template");
}

// Called by vk_device.c once the device exists. "boot" in TOTK_RDOC captures the work the game does before its first
// present (Splatoon 3 builds its menu environment cube maps there): the capture starts here and ends after that present.
void rdoc_device_ready(void) {
    if (!api || !boot_capture || capturing) return;
    api->StartFrameCapture(NULL, NULL);
    capturing = 1;
    capture_frame = 0;
    rt_log("rdoc: capture of frame 0 started (the work before the first present)");
}

// Called by vk_backend.c present_image after frame `frame` was queued and the present drained. A capture started here
// holds the next frame's submits and its blit and present, and ends after that frame's present.
void rdoc_after_present(uint64_t frame) {
    if (!api) return;
    if (capturing) {
        api->EndFrameCapture(NULL, NULL);
        capturing = 0;
        char path[PLAT_PATH_MAX];
        uint32_t len = sizeof path, n = api->GetNumCaptures();
        if (n && api->GetCapture(n - 1, path, &len, NULL))
            rt_log("rdoc: frame %llu captured (world clock %.3f s): %s", (unsigned long long)capture_frame,
                   frame_step_world_us() / 1e6, path);
        else rt_log("rdoc: frame %llu capture failed", (unsigned long long)capture_frame);
    }
    int64_t w = frame_step_world_us();
    for (int i = 0; i < n_targets; i++) {
        RdocTarget *t = &targets[i];
        if (t->done || !(t->world_us >= 0 ? w >= t->world_us : t->frame == frame + 1)) continue;
        t->done = 1;
        if (capturing) continue;
        api->StartFrameCapture(NULL, NULL);
        capturing = 1;
        capture_frame = frame + 1;
        rt_log("rdoc: capture of frame %llu started (world clock %.3f s at frame %llu)",
               (unsigned long long)capture_frame, w / 1e6, (unsigned long long)frame);
    }
}
