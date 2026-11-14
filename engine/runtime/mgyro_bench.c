// TOTK_MGYRO_BENCH=<tests>: a test bench for the mouse as the six-axis sensor (mgyro_bench.h). A simulated mouse sends
// counts into the motion thread where the real mouse's raw counts enter it (input.c mouse_gyro_collect), so the counts
// go through the same conversion, sample grid and sensor ring as a player's. The game's aim is read from its memory: at
// the start the bench finds the unit vectors in the game's heap that turn with the virtual controller (a scan of every
// float triple of length 1, then three turns that must move them by consistent angles about one fixed axis), and from
// then on each six-axis read of the game logs those vectors with the controller's own yaw and pitch.
//
// <tests>: "all" or a list (commas or spaces) of look, idle, lin, step, phase, rate, rt, fields, pitchyaw, yawaxis,
// reverse, cap, recenter, limits, hitch, fps, image; the finder always runs first, after a Y that recentres the camera.
// TOTK_MGYRO_BENCH_OUT: the CSV file (default mgyro-bench.csv);
// TOTK_MGYRO_BENCH_DELAY: seconds after the first live six-axis read before the bench starts (default 40, the test
// range is up by then); TOTK_MGYRO_BENCH_EXIT=1 closes the game when the tests are done. tools/mgyro_bench.py turns the
// CSV into the tables of docs/controls.md.
//
// CSV lines (times in ns of kern_now_ns, angles in degrees, counts as the mouse sends them: x right, y down):
//   H,<key>,<value>                                  bench settings
//   E,<t>,<read>,<text>                              a test's step ("lin axis=x v=1200 n=1200", "end", ...)
//   T,<t>,<dx>,<dy>                                  counts the motion thread took from the simulated mouse
//   S,<sampling>,<t>,<yaw>,<pitch>,<n>               a sample pushed: the controller's attitude in it, n samples in its tick
//   R,<read>,<t>,<sampling>,<new>,<present>,<yaw>,<pitch>,<inj x>,<inj y>,<aim vectors x,y,z...>
//                                                    a six-axis read: the newest sample handed to the game, how many
//                                                    are new since the last read, the controller in that sample, every
//                                                    count sent so far, and the aim vectors now
//   P,<present>,<t>,<slot>,<read>                    the game's present call and the display time it is held to
//   F,<slot>,<t>                                     the host present of that frame
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cfg.h"
#include "gfx.h"
#include "host.h"
#include "input.h"
#include "mgyro_bench.h"
#include "platform/platform.h"

enum { AIMS = 10, SRING = 8192 };

static int bench_on = -1;
int mgb_on(void) {
    if (bench_on < 0) {
        const char *e = cfg_get("TOTK_MGYRO_BENCH");
        bench_on = e && *e && strcmp(e, "0") != 0;
    }
    return bench_on;
}

// ---- the CSV: lines appended under out_lock by any thread, written by the bench thread
static PlatMutex out_lock = PLAT_MUTEX_INIT;
static char *obuf;
static size_t olen, ocap;
static FILE *ofile;
static volatile int32_t closed;   // the bench is done: nothing more is kept

static void out(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void out(const char *fmt, ...) {
    if (__atomic_load_n(&closed, __ATOMIC_RELAXED)) return;
    char line[640];
    va_list ap;
    va_start(ap, fmt);
    int printed = vsnprintf(line, sizeof line, fmt, ap);
    va_end(ap);
    if (printed <= 0) return;
    // vsnprintf reports the length it wanted: a line longer than the buffer is kept cut at the buffer's end
    size_t n = (size_t)printed < sizeof line ? (size_t)printed : sizeof line - 1;
    plat_mutex_lock(&out_lock);
    if (olen + n + 1 > ocap) {
        size_t c = ocap ? ocap * 2 : (size_t)1 << 20;
        while (c < olen + n + 1) c *= 2;
        char *p = realloc(obuf, c);
        if (!p) { plat_mutex_unlock(&out_lock); return; }
        obuf = p, ocap = c;
    }
    memcpy(obuf + olen, line, n);
    olen += n;
    obuf[olen++] = '\n';
    plat_mutex_unlock(&out_lock);
}

static void out_flush(void) {
    plat_mutex_lock(&out_lock);
    char *b = obuf;
    size_t n = olen;
    obuf = NULL, olen = ocap = 0;
    plat_mutex_unlock(&out_lock);
    if (b && ofile && n) fwrite(b, 1, n, ofile);
    free(b);
    if (ofile) fflush(ofile);
}

// ---- the simulated mouse: one move at a time, its counts sent as packets at the polling rate
static PlatMutex mv_lock = PLAT_MUTEX_INIT;
static struct { int active; uint64_t t0, dur; double rate; int32_t nx, ny, sx, sy; } mv;   // sx, sy: sent so far
static int64_t inj_x, inj_y;   // every count sent (mv_lock)

void mgb_take(uint64_t now, double *dx, double *dy) {
    *dx = *dy = 0;
    plat_mutex_lock(&mv_lock);
    if (mv.active && now >= mv.t0) {
        double frac = 1;
        if (mv.dur) {   // packets at t0 + i / rate, each bringing the total to round(n * i / (rate * dur))
            double i = floor((double)(now - mv.t0) * 1e-9 * mv.rate);
            frac = i / (mv.rate * (double)mv.dur * 1e-9);
            if (frac > 1) frac = 1;
        }
        int32_t tx = (int32_t)lround(mv.nx * frac), ty = (int32_t)lround(mv.ny * frac);
        int32_t ddx = tx - mv.sx, ddy = ty - mv.sy;
        mv.sx = tx, mv.sy = ty;
        if (frac >= 1) mv.active = 0;
        inj_x += ddx, inj_y += ddy;
        *dx = ddx, *dy = ddy;
        if (ddx || ddy) out("T,%llu,%d,%d", (unsigned long long)now, ddx, ddy);
    }
    plat_mutex_unlock(&mv_lock);
}

// ---- samples: numbered as the sensor ring numbers them (every push to the mouse gyro's sensor passes here)
typedef struct { int64_t no; uint64_t t; double yaw, pitch; } Smp;
static PlatMutex s_lock = PLAT_MUTEX_INIT;
static Smp sring[SRING];
static int64_t s_no;
static double s_yaw, s_last;
static int s_have;
// Field tests: 0 the samples as made; 1 the rate only (attitude, gravity and angle held at the mode's first sample);
// 2 the attitude only (rate 0, angle held); 3 delta_ns doubled.
static volatile int32_t field_mode;
static MotionState held;
static int held_set;

static double clamp1(double v) { return v > 1 ? 1 : v < -1 ? -1 : v; }

static volatile int32_t yaw_body_set = -1;   // the yawaxis test's choice of yaw axis, -1: input.c's own
int mgb_yaw_body(int def) {
    int v = __atomic_load_n(&yaw_body_set, __ATOMIC_RELAXED);
    return v >= 0 ? v : def;
}

void mgb_samples(MotionState *s, int n, uint64_t now) {
    if (!mgb_on()) return;
    int mode = __atomic_load_n(&field_mode, __ATOMIC_RELAXED);
    if (!mode) held_set = 0;
    for (int i = 0; i < n; i++) {
        // the controller's y axis (dir[3..5]) in the world, held flat: yaw about the world's up, pitch above level
        double yaw = atan2(-s[i].dir[3], s[i].dir[4]) * 180 / M_PI, pitch = asin(clamp1(s[i].dir[5])) * 180 / M_PI;
        if (!s_have) s_last = s_yaw = yaw, s_have = 1;
        double d = yaw - s_last;
        while (d > 180) d -= 360;
        while (d < -180) d += 360;
        s_yaw += d, s_last = yaw;
        plat_mutex_lock(&s_lock);
        int64_t no = ++s_no;
        sring[no % SRING] = (Smp){no, now, s_yaw, pitch};
        plat_mutex_unlock(&s_lock);
        out("S,%lld,%llu,%.6f,%.6f,%d", (long long)no, (unsigned long long)now, s_yaw, pitch, n);
        if (mode && !held_set) held = s[i], held_set = 1;
        if (mode == 1) {
            memcpy(s[i].dir, held.dir, sizeof held.dir);
            memcpy(s[i].acc, held.acc, sizeof held.acc);
            memcpy(s[i].angle, held.angle, sizeof held.angle);
        } else if (mode == 2) {
            memset(s[i].gyro, 0, sizeof s[i].gyro);
            memcpy(s[i].angle, held.angle, sizeof held.angle);
        } else if (mode == 3) {
            s[i].delta_ns *= 2;
        }
    }
}

// ---- reads and presents
static volatile int64_t reads;              // six-axis reads so far
static volatile uint64_t last_read_ns, period_ns;
static int64_t last_s0;
static uint64_t aim_addr[AIMS];
static volatile int32_t naims;
static volatile int64_t stall_ns;           // the next read holds the game thread that long (hitch test)
static volatile int32_t shots_left;         // the next reads each ask for a shot of the frame presented after them
static char shots_dir[512];
static volatile uint64_t presents;
static volatile int32_t started;

static void bench_main(void *arg);

void mgb_read(const MotionState *s, int n, uint64_t now) {
    if (!mgb_on()) return;
    if (!__atomic_load_n(&started, __ATOMIC_RELAXED) && !plat_xchg32(&started, 1))
        if (!plat_thread_spawn(bench_main, NULL, "mgyro bench", 0)) rt_log("mgyro bench: cannot start its thread");
    int64_t s0 = n ? s[0].sampling : 0;
    double cy = NAN, cp = NAN;
    plat_mutex_lock(&s_lock);
    const Smp *e = &sring[s0 % SRING];
    if (s0 > 0 && e->no == s0) cy = e->yaw, cp = e->pitch;
    plat_mutex_unlock(&s_lock);
    char aims[AIMS * 48] = "";
    int k = 0, na = __atomic_load_n(&naims, __ATOMIC_ACQUIRE);
    for (int i = 0; i < na; i++) {
        const float *v = (const float *)(uintptr_t)aim_addr[i];
        k += snprintf(aims + k, sizeof aims - (size_t)k, ",%.8f,%.8f,%.8f", v[0], v[1], v[2]);
    }
    plat_mutex_lock(&mv_lock);
    int64_t ix = inj_x, iy = inj_y;
    plat_mutex_unlock(&mv_lock);
    int64_t r = __atomic_load_n(&reads, __ATOMIC_RELAXED) + 1;
    out("R,%lld,%llu,%lld,%lld,%llu,%.6f,%.6f,%lld,%lld%s", (long long)r, (unsigned long long)now, (long long)s0,
        (long long)(s0 - last_s0), (unsigned long long)__atomic_load_n(&presents, __ATOMIC_RELAXED), cy, cp,
        (long long)ix, (long long)iy, aims);
    last_s0 = s0;
    uint64_t lr = __atomic_load_n(&last_read_ns, __ATOMIC_RELAXED);
    if (lr && now > lr && now - lr < 100000000ull) {   // the frame period, smoothed over about 16 reads
        uint64_t p = __atomic_load_n(&period_ns, __ATOMIC_RELAXED);
        __atomic_store_n(&period_ns, p ? p + ((int64_t)(now - lr) - (int64_t)p) / 16 : now - lr, __ATOMIC_RELAXED);
    }
    __atomic_store_n(&last_read_ns, now, __ATOMIC_RELAXED);
    __atomic_store_n(&reads, r, __ATOMIC_RELEASE);
    int64_t st = __atomic_exchange_n(&stall_ns, 0, __ATOMIC_RELAXED);
    if (st > 0) {
        out("E,%llu,%lld,stall ns=%lld", (unsigned long long)kern_now_ns(), (long long)r, (long long)st);
        plat_sleep_ns((uint64_t)st);
    }
}

void mgb_present(const void *tex, uint64_t n, uint64_t now, uint64_t slot) {
    if (!mgb_on()) return;
    __atomic_store_n(&presents, n, __ATOMIC_RELAXED);
    int64_t r = __atomic_load_n(&reads, __ATOMIC_RELAXED);
    out("P,%llu,%llu,%llu,%lld", (unsigned long long)n, (unsigned long long)now, (unsigned long long)slot, (long long)r);
    if (__atomic_load_n(&shots_left, __ATOMIC_RELAXED) > 0) {   // after the present: the picture of this frame
        char p[600];
        snprintf(p, sizeof p, "%s/p%06llu-r%06lld.png", shots_dir, (unsigned long long)n, (long long)r);
        rt_shot_small((const tex_t *)tex, p);
        __atomic_sub_fetch(&shots_left, 1, __ATOMIC_RELAXED);
    }
}

void mgb_flip(uint64_t slot, uint64_t now) {
    if (!mgb_on()) return;
    out("F,%llu,%llu", (unsigned long long)slot, (unsigned long long)now);
}

// ---- the bench thread
static void ev(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void ev(const char *fmt, ...) {
    char t[400];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(t, sizeof t, fmt, ap);
    va_end(ap);
    out("E,%llu,%lld,%s", (unsigned long long)kern_now_ns(), (long long)__atomic_load_n(&reads, __ATOMIC_ACQUIRE), t);
}

static int64_t nreads(void) { return __atomic_load_n(&reads, __ATOMIC_ACQUIRE); }

static void wait_reads(int64_t n) {
    int64_t target = nreads() + n;
    uint64_t t0 = kern_now_ns();
    while (nreads() < target) {
        plat_sleep_ns(1000000);
        if (kern_now_ns() - t0 > 30000000000ull) { rt_log("mgyro bench: the game stopped reading the sensor"); return; }
    }
    out_flush();
}

static void move_start(int nx, int ny, double cps, double rate, uint64_t t0) {
    double n = fmax(fabs((double)nx), fabs((double)ny));
    plat_mutex_lock(&mv_lock);
    mv.active = 1, mv.t0 = t0 ? t0 : kern_now_ns(), mv.dur = cps > 0 ? (uint64_t)(n / cps * 1e9) : 0;
    mv.rate = rate, mv.nx = nx, mv.ny = ny, mv.sx = mv.sy = 0;
    plat_mutex_unlock(&mv_lock);
}

static void move_wait(void) {
    for (;;) {
        plat_mutex_lock(&mv_lock);
        int a = mv.active;
        plat_mutex_unlock(&mv_lock);
        if (!a) return;
        plat_sleep_ns(500000);
    }
}

static void move(int nx, int ny, double cps, double rate) {
    move_start(nx, ny, cps, rate, 0);
    move_wait();
}

// ---- finding the aim: unit vectors that turn with the controller about one fixed axis
typedef struct { uint64_t a; float v0[3], v1[3]; int pass; double d1, d2, d12, d3; float dv; } Cand;

// A unit vector that can turn: no component at +-1, and at most one near 0 (a level camera's right vector has one).
static int unit_ok3(float x, float y, float z) {
    if (!(fabsf(x) <= 0.9995f && fabsf(y) <= 0.9995f && fabsf(z) <= 0.9995f)) return 0;
    return (fabsf(x) < 1e-3f) + (fabsf(y) < 1e-3f) + (fabsf(z) < 1e-3f) <= 1;
}

static size_t scan(Cand **out_c) {
    uint64_t base = mem_layout()->heap.base, size = mem_heap_size();
    size_t cap = 1 << 20, n = 0;
    Cand *c = malloc(cap * sizeof *c);
    if (!c) return 0;
    uint64_t chunk = 1ull << 21, skipped = 0;
    for (uint64_t off = 0; off + 12 <= size; off += chunk) {
        // 2 MiB at a time; a chunk with a page that cannot be read is taken page by page
        if (off % (1ull << 21) == 0) chunk = plat_mem_readable(base + off, size - off < (1ull << 21) ? size - off : 1ull << 21) ? 1ull << 21 : 4096;
        uint64_t len = size - off < chunk ? size - off : chunk;
        if (chunk == 4096 && !plat_mem_readable(base + off, len)) { skipped += len; continue; }
        if (len + 8 <= size - off && plat_mem_readable(base + off + len, 8)) len += 8;   // triples across the boundary
        const float *f = (const float *)(uintptr_t)(base + off);
        size_t m = (size_t)(len / 4), stop = (size_t)((size - off < chunk ? size - off : chunk) / 4);
        for (size_t i = 0; i + 2 < m && i < stop; i++) {
            float x = f[i], y = f[i + 1], z = f[i + 2];
            float q = x * x + y * y + z * z;
            if (!(fabsf(q - 1) <= 2e-5f) || !unit_ok3(x, y, z)) continue;
            if (n == cap) {
                if (cap >= (size_t)8 << 20) break;
                Cand *p = realloc(c, cap * 2 * sizeof *c);
                if (!p) break;
                c = p, cap *= 2;
            }
            c[n++] = (Cand){.a = base + off + i * 4, .v0 = {x, y, z}};
        }
    }
    if (skipped) rt_log("mgyro bench: %llu bytes of the heap could not be read", (unsigned long long)skipped);
    *out_c = c;
    return n;
}

// The angle between two vectors, in degrees.
static double chord(const float *a, const float *b) {
    double d = (double)a[0] * b[0] + (double)a[1] * b[1] + (double)a[2] * b[2];
    double n = sqrt(((double)a[0] * a[0] + (double)a[1] * a[1] + (double)a[2] * a[2]) *
                    ((double)b[0] * b[0] + (double)b[1] * b[1] + (double)b[2] * b[2]));
    return n > 0 ? acos(clamp1(d / n)) * 180 / M_PI : 0;
}

static int by_pitch_moved(const void *a, const void *b) {
    float x = ((const Cand *)a)->dv, y = ((const Cand *)b)->dv;
    return x < y ? 1 : x > y ? -1 : 0;
}

static void read3(uint64_t a, float v[3]) {
    const float *p = (const float *)(uintptr_t)a;
    v[0] = p[0], v[1] = p[1], v[2] = p[2];
}

// Unit vectors that follow the controller: a turn of +A moves them by c1 degrees, -2A brings them as far to the other
// side (by c1 from the start, about 2 c1 from where they were), +A back to the start. Whatever the axis they turn about
// (the game may turn its camera in the camera's own frame), so the view's vectors and the game's copies of the sensor
// both pass; the copies turn by exactly the controller's angle. Then how far a pitch moves each.
static int find_aims(void) {
    const int A = 500, P = 250;   // counts: 20 and 10 degrees of controller turn at 0.04
    ev("find pre");
    // The right stick turns the camera and not the controller, so the view's vectors and the game's copies of the
    // sensor no longer hold the same values; then off the start pose, so no component is exactly 0 or 1.
    input_inject("RSTICK 26000 0 20");
    wait_reads(60);
    move(137, -53, 2000, 1000);
    wait_reads(45);
    uint64_t t0 = kern_now_ns();
    Cand *c = NULL;
    size_t n = scan(&c);
    rt_log("mgyro bench: %zu unit vectors in the heap (%.1f GiB scanned in %.2f s)", n,
           mem_heap_size() / (double)(1ull << 30), (kern_now_ns() - t0) / 1e9);
    ev("find scan n=%zu", n);
    move(A, 0, 2000, 1000);
    wait_reads(45);
    size_t m = 0;
    for (size_t i = 0; i < n; i++) {
        float v1[3];
        read3(c[i].a, v1);
        if (!(fabsf(v1[0] * v1[0] + v1[1] * v1[1] + v1[2] * v1[2] - 1) <= 1e-4f)) continue;
        double c1 = chord(c[i].v0, v1);
        if (c1 < 2) continue;
        c[i].d1 = c1;
        memcpy(c[i].v1, v1, sizeof v1);
        c[m++] = c[i];
    }
    ev("find yaw1 n=%zu", m);
    move(-2 * A, 0, 2000, 1000);
    wait_reads(45);
    for (size_t i = 0; i < m; i++) {
        float v2[3];
        read3(c[i].a, v2);
        c[i].d2 = chord(c[i].v0, v2);
        c[i].d12 = chord(c[i].v1, v2);
        // to the other side of the start, by about as much (the camera's turn for a turn need not be the same each time)
        c[i].pass = c[i].d2 > 0.5 * c[i].d1 && c[i].d2 < 2 * c[i].d1 && c[i].d12 > 1.5 * fmin(c[i].d1, c[i].d2);
    }
    move(A, 0, 2000, 1000);
    wait_reads(45);
    size_t m3 = 0;
    for (size_t i = 0; i < m; i++) {
        float v3[3];
        read3(c[i].a, v3);
        c[i].d3 = chord(c[i].v0, v3);
        c[i].pass &= c[i].d3 < 0.3 * c[i].d1;   // the camera need not come back exactly: that is measured later
        m3 += c[i].pass;
    }
    ev("find yaw3 n=%zu", m3);
    move(0, -P, 2000, 1000);   // up
    wait_reads(45);
    ev("find pitch");
    for (size_t i = 0; i < m; i++) {
        float v4[3];
        read3(c[i].a, v4);
        c[i].dv = (float)chord(c[i].v0, v4);
    }
    move(0, P, 2000, 1000);
    wait_reads(45);
    for (size_t i = 0; i < m && i < 1500; i++)
        out("H,cand,heap+%#llx c1=%.4f c2=%.4f c12=%.4f c3=%.4f cp=%.4f pass=%d v0=%.5f %.5f %.5f",
            (unsigned long long)(c[i].a - mem_layout()->heap.base), c[i].d1, c[i].d2, c[i].d12, c[i].d3, c[i].dv,
            c[i].pass, c[i].v0[0], c[i].v0[1], c[i].v0[2]);
    // The view: vectors that turned by another angle than the controller (the game's copies of the sensor turn by
    // exactly A counts, less for a vector off the horizontal), those the pitch moved most first (the forward vector moves
    // by the whole pitch), one per distinct value; then one copy of the sensor, as a reference.
    // Before them all, the camera's forward vector even when its turns did not add up (the camera turned 37, 59 and 15
    // degrees for +20, -40 and +20 with the 200 Hz states of the first version): nearly level, turned far, moved by the
    // pitch about as much as the camera turns.
    qsort(c, m, sizeof *c, by_pitch_moved);
    for (size_t i = 0; i < m; i++)
        c[i].pass |= (c[i].d1 > 25 && c[i].dv > 10 && c[i].dv < 40 && fabsf(c[i].v0[1]) < 0.1f) << 1;
    int chosen = 0;
    for (int pass = 0; pass < 3 && chosen < AIMS; pass++)
        for (size_t i = 0; i < m && chosen < AIMS; i++) {
            int copy = fabs(c[i].d1 - A * 0.04) < 0.02;
            int pitched = c[i].dv > 1;
            if (pass == 0 ? !(c[i].pass & 2) : !(c[i].pass & 1) || (pass == 1 && copy) || (pass == 2 && (!copy || !pitched)))
                continue;
            int dup = 0;
            for (int j = 0; j < chosen; j++) {
                float w[3], v[3];
                read3(aim_addr[j], w);
                read3(c[i].a, v);
                dup |= fabsf(w[0] - v[0]) < 1e-5f && fabsf(w[1] - v[1]) < 1e-5f && fabsf(w[2] - v[2]) < 1e-5f;
            }
            if (dup) continue;
            aim_addr[chosen] = c[i].a;
            out("H,aim%d,heap+%#llx c1=%.4f c3=%.4f cp=%.4f kind=%s v0=%.6f %.6f %.6f", chosen,
                (unsigned long long)(c[i].a - mem_layout()->heap.base), c[i].d1, c[i].d3, c[i].dv,
                copy ? "sensor" : pitched ? "view" : "view-flat", c[i].v0[0], c[i].v0[1], c[i].v0[2]);
            const float *ctx = (const float *)(uintptr_t)(c[i].a - 48);
            char line[400];
            int k = 0;
            for (int j = 0; j < 28; j++) k += snprintf(line + k, sizeof line - (size_t)k, " %.5f", ctx[j]);
            out("H,ctx%d,from -48:%s", chosen, line);
            rt_log("mgyro bench: aim %d at heap+%#llx: turned %.3f degrees for %.1f, %.3f for the pitch (%s)", chosen,
                   (unsigned long long)(c[i].a - mem_layout()->heap.base), c[i].d1, A * 0.04, c[i].dv,
                   copy ? "the game's copy of the sensor" : "view");
            chosen++;
            if (pass == 2) break;   // one sensor copy is enough
        }
    free(c);
    __atomic_store_n(&naims, chosen, __ATOMIC_RELEASE);
    ev("find done aims=%d", chosen);
    out_flush();
    return chosen;
}

// ---- tests
static void settle(void) { wait_reads(40); }

static void t_idle(int secs) {
    ev("idle s=%d", secs);
    wait_reads(60 * secs);
    ev("end");
}

static void t_lin(int axis) {
    static const double v[] = {6, 15, 30, 60, 120, 300, 600, 1200, 3000, 6000, 12000, 30000, 60000, 120000};
    for (size_t i = 0; i < sizeof v / sizeof *v; i++) {
        int n = axis ? (int)fmin(fmax(v[i] * 0.5, 30), 500) : (int)fmin(fmax(v[i], 30), 2400);
        for (int sgn = 1; sgn >= -1; sgn -= 2) {
            ev("lin axis=%c v=%g n=%d", axis ? 'y' : 'x', v[i], sgn * n);
            move(axis ? 0 : sgn * n, axis ? sgn * n : 0, v[i], 1000);
            settle();
            ev("end");
        }
    }
}

static void t_rt(void) {
    static const struct { int axis, n; double v; int cycles; } r[] = {
        {0, 200, 1200, 100}, {0, 600, 30000, 100}, {1, 100, 1200, 100}, {1, 300, 30000, 100}};
    for (size_t i = 0; i < sizeof r / sizeof *r; i++) {
        ev("rt axis=%c n=%d v=%g cycles=%d", r[i].axis ? 'y' : 'x', r[i].n, r[i].v, r[i].cycles);
        for (int k = 0; k < r[i].cycles; k++) {
            move(r[i].axis ? 0 : r[i].n, r[i].axis ? r[i].n : 0, r[i].v, 1000);
            move(r[i].axis ? 0 : -r[i].n, r[i].axis ? -r[i].n : 0, r[i].v, 1000);
        }
        settle();
        ev("end");
    }
    // slow one way, fast the other: any speed dependence of the gain adds up
    ev("rt axis=x n=300 v=300/30000 cycles=50");
    for (int k = 0; k < 50; k++) {
        move(300, 0, 300, 1000);
        move(-300, 0, 30000, 1000);
    }
    settle();
    ev("end");
}

static void t_step(void) {
    static const struct { int axis, n; } s[] = {{0, 10}, {0, 50}, {0, 200}, {0, 800}, {1, 50}, {1, 200}};
    for (size_t i = 0; i < sizeof s / sizeof *s; i++)
        for (int k = 0; k < 6; k++) {
            int n = k & 1 ? -s[i].n : s[i].n;
            ev("step axis=%c n=%d", s[i].axis ? 'y' : 'x', n);
            move(s[i].axis ? 0 : n, s[i].axis ? n : 0, 0, 1000);
            settle();
            ev("end");
        }
}

// A burst a given time before the predicted next read: is it in that read, or the next one?
static void t_phase(void) {
    for (int rep = 0; rep < 3; rep++)
        for (int phi = 0; phi <= 16; phi++) {
            int64_t r0 = nreads();
            while (nreads() == r0) plat_sleep_ns(200000);
            uint64_t p = __atomic_load_n(&period_ns, __ATOMIC_RELAXED), last = __atomic_load_n(&last_read_ns, __ATOMIC_RELAXED);
            if (!p) p = 16666667;
            uint64_t t = last + p - (uint64_t)phi * 1000000 - 500000, now = kern_now_ns();
            while (t < now + 1000000) t += p;
            int n = (rep * 17 + phi) & 1 ? -60 : 60;
            ev("phase phi_us=%d n=%d t0=%llu", phi * 1000 + 500, n, (unsigned long long)t);
            move_start(n, 0, 0, 1000, t);
            move_wait();
            wait_reads(20);
            ev("end");
        }
}

static void t_rate(void) {
    static const double hz[] = {125, 250, 500, 1000, 2000, 4000, 8000};
    static const struct { double v; int n; } sp[] = {{1200, 1200}, {6000, 3000}};
    for (size_t s = 0; s < 2; s++)
        for (size_t i = 0; i < sizeof hz / sizeof *hz; i++)
            for (int sgn = 1; sgn >= -1; sgn -= 2) {
                ev("rate v=%g hz=%g n=%d", sp[s].v, hz[i], sgn * sp[s].n);
                move(sgn * sp[s].n, 0, sp[s].v, hz[i]);
                settle();
                ev("end");
            }
}

static void t_fields(void) {
    for (int axis = 0; axis < 2; axis++)
        for (int mode = 0; mode <= 3; mode++) {
            __atomic_store_n(&field_mode, mode, __ATOMIC_RELAXED);
            wait_reads(3);
            for (int sgn = 1; sgn >= -1; sgn -= 2) {   // a move at 600 counts/s, not a packet: every sample carries counts
                int n = sgn * (axis ? 100 : 200);
                ev("field axis=%c mode=%d n=%d", axis ? 'y' : 'x', mode, n);
                move(axis ? 0 : n, axis ? n : 0, 600, 1000);
                settle();
                ev("end");
            }
            __atomic_store_n(&field_mode, 0, __ATOMIC_RELAXED);
            wait_reads(10);
        }
}

static void t_pitchyaw(void) {
    static const int pc[] = {-500, 500};   // up 20 degrees, down 20 degrees
    for (int i = 0; i < 2; i++) {
        move(0, pc[i], 1200, 1000);
        settle();
        for (int sgn = 1; sgn >= -1; sgn -= 2) {
            ev("pitchyaw pitch_counts=%d n=%d", pc[i], sgn * 500);
            move(sgn * 500, 0, 1200, 1000);
            settle();
            ev("end");
        }
        move(0, -pc[i], 1200, 1000);
        settle();
    }
}

// The yaw gain with the controller pitched 20 degrees up and down, the yaw rate about the world's up then about the
// controller's own vertical axis.
static void t_yawaxis(void) {
    static const int pc[] = {-500, 500};
    for (int i = 0; i < 2; i++) {
        move(0, pc[i], 1200, 1000);
        settle();
        for (int body = 0; body < 3; body++) {
            __atomic_store_n(&yaw_body_set, body, __ATOMIC_RELAXED);
            for (int sgn = 1; sgn >= -1; sgn -= 2) {
                ev("yawaxis body=%d pitch_counts=%d n=%d", body, pc[i], sgn * 500);
                move(sgn * 500, 0, 1200, 1000);
                settle();
                ev("end");
            }
        }
        __atomic_store_n(&yaw_body_set, -1, __ATOMIC_RELAXED);
        move(0, -pc[i], 1200, 1000);
        settle();
    }
}

// Back and forth at 1200 counts/s (240 counts, about 12 frames each way), straight away or after 1, 2, 4 or 8 reads
// without counts: how the game takes a reversal of the turn, with input.c's hold at a reversal (hold=-1) and without.
static volatile int32_t hold_set = -1;
int mgb_reverse_hold(int def) {
    int v = __atomic_load_n(&hold_set, __ATOMIC_RELAXED);
    return v >= 0 ? v : def;
}

static void t_reverse(void) {
    static const int pause[] = {0, 1, 2, 4, 8}, hold[] = {0, -1};
    for (size_t h = 0; h < 2; h++)
        for (size_t p = 0; p < sizeof pause / sizeof *pause; p++) {
            __atomic_store_n(&hold_set, hold[h], __ATOMIC_RELAXED);
            ev("reverse hold=%d pause=%d cycles=5 n=240 v=1200", hold[h], pause[p]);
            for (int k = 0; k < 5; k++)
                for (int sgn = 1; sgn >= -1; sgn -= 2) {
                    ev("leg hold=%d pause=%d sign=%d", hold[h], pause[p], sgn);
                    move(sgn * 240, 0, 1200, 1000);
                    if (pause[p]) wait_reads(pause[p]);
                }
            settle();
            ev("end");
        }
    __atomic_store_n(&hold_set, -1, __ATOMIC_RELAXED);
}

// Packets of more and more counts at once: the fastest turn the game takes in one frame.
static void t_cap(void) {
    static const int n[] = {100, 200, 400, 800, 1200, 1600, 2400, 3200, 4800};
    for (size_t i = 0; i < sizeof n / sizeof *n; i++)
        for (int sgn = 1; sgn >= -1; sgn -= 2) {
            ev("cap n=%d", sgn * n[i]);
            move(sgn * n[i], 0, 0, 1000);
            settle();
            ev("end");
        }
}

// The game's recentre (Y) with the controller pitched 10 degrees down or up, with the virtual controller levelled at
// the same time (as Y does) or not: where the camera's pitch ends, and how it moves when the mouse comes back.
static volatile int32_t level_set = -1;   // -1: input.c's own choice
int mgb_level_on_y(int def) {
    int v = __atomic_load_n(&level_set, __ATOMIC_RELAXED);
    return v >= 0 ? v : def;
}

static void t_recenter(void) {
    static const struct { const char *what; int dy, y; } st[] = {
        {"down10", 250, 0}, {"Y", 0, 1}, {"up10", -250, 0}, {"up10b", -250, 0}, {"down10b", 250, 0}};
    for (int lv = 1; lv >= 0; lv--) {
        // from level: the controller levelled with a Y, a small move each way (the game takes the levelling's turn
        // then), a Y that does not level: controller and camera level
        __atomic_store_n(&level_set, 1, __ATOMIC_RELAXED);
        input_inject("Y 6");
        wait_reads(90);
        move(0, 25, 1200, 1000);
        move(0, -25, 1200, 1000);
        settle();
        __atomic_store_n(&level_set, 0, __ATOMIC_RELAXED);
        input_inject("Y 6");
        wait_reads(90);
        __atomic_store_n(&level_set, lv, __ATOMIC_RELAXED);
        for (size_t i = 0; i < sizeof st / sizeof *st; i++) {
            ev("recenter level=%d step=%s", lv, st[i].what);
            if (st[i].y) {
                input_inject("Y 6");
                wait_reads(90);
            } else {
                move(0, st[i].dy, 1200, 1000);
                settle();
            }
            ev("end");
        }
    }
    __atomic_store_n(&level_set, -1, __ATOMIC_RELAXED);
}

// From level, up past every limit in 125-count steps (5 degrees of controller pitch at 0.04), back down 8 steps, down
// past every limit (the controller stops at mouse_gyro_pitch), back up 8 steps, then level again.
static void t_limits(void) {
    static const struct { int steps, dy; } seq[] = {{24, -125}, {8, 125}, {32, 125}, {8, -125}};
    for (size_t s = 0; s < sizeof seq / sizeof *seq; s++)
        for (int k = 1; k <= seq[s].steps; k++) {
            move(0, seq[s].dy, 2500, 1000);
            wait_reads(20);
            ev("lim leg=%zu step=%d dy=%d", s, k, seq[s].dy);
        }
    plat_mutex_lock(&s_lock);
    double pitch = sring[s_no % SRING].pitch;   // the controller's, degrees above level
    plat_mutex_unlock(&s_lock);
    move(0, (int)lround(pitch / cfg_double("TOTK_MOUSE_GYRO_SENS")), 2500, 1000);
    settle();
    ev("lim level");
}

static void t_hitch(void) {
    static const int64_t ms[] = {50, 100, 150};
    for (int sgn = 1; sgn >= -1; sgn -= 2) {
        ev("hitch v=1200 n=%d stalls=%s", sgn * 2400, sgn > 0 ? "50,100,150" : "none");
        move_start(sgn * 2400, 0, 1200, 1000, 0);
        for (int i = 0; i < 3; i++) {
            wait_reads(25);
            if (sgn > 0) __atomic_store_n(&stall_ns, ms[i] * 1000000, __ATOMIC_RELAXED);
        }
        move_wait();
        settle();
        ev("end");
    }
}

static void t_fps(void) {
    static const char *lim[] = {"30", "45"};
    for (int i = 0; i < 2; i++) {
        cfg_set("TOTK_FPS_LIMIT", lim[i]);
        wait_reads(60);
        for (int sgn = 1; sgn >= -1; sgn -= 2) {
            ev("fps limit=%s lin v=1200 n=%d", lim[i], sgn * 1200);
            move(sgn * 1200, 0, 1200, 1000);
            settle();
            ev("end");
        }
        for (int sgn = 1; sgn >= -1; sgn -= 2) {
            ev("fps limit=%s step n=%d", lim[i], sgn * 200);
            move(sgn * 200, 0, 0, 1000);
            settle();
            ev("end");
        }
    }
    cfg_set("TOTK_FPS_LIMIT", "60");
    wait_reads(60);
}

static void shots_setup(void) {
    const char *o = cfg_get("TOTK_MGYRO_BENCH_OUT");
    snprintf(shots_dir, sizeof shots_dir, "%s-shots", o && *o ? o : "mgyro-bench.csv");
    plat_file_mkdir_p(shots_dir);
}

// A picture at rest and after each turn: right 10 degrees and back, up 5 and back, up 40, right 20 there and back,
// down to level again. Shows what turns on screen and how the view's vectors move with it.
static void t_look(void) {
    shots_setup();
    static const struct { int dx, dy; } m[] = {{0, 0}, {250, 0}, {-250, 0}, {0, -125}, {0, 125}, {0, -1000}, {500, 0},
                                               {-500, 0}, {0, 1000}};
    for (size_t i = 0; i < sizeof m / sizeof *m; i++) {
        if (m[i].dx || m[i].dy) move(m[i].dx, m[i].dy, 1200, 1000);
        settle();
        ev("look dx=%d dy=%d", m[i].dx, m[i].dy);
        __atomic_store_n(&shots_left, 1, __ATOMIC_RELAXED);
        wait_reads(5);
    }
}

static void t_image(void) {
    shots_setup();
    for (int sgn = 1; sgn >= -1; sgn -= 2) {
        ev("image n=%d", sgn * 400);
        __atomic_store_n(&shots_left, 16, __ATOMIC_RELAXED);
        wait_reads(3);
        move(sgn * 400, 0, 0, 1000);
        wait_reads(16);
        settle();
        ev("end");
    }
}

static int wants(const char *list, const char *name) {
    if (!strcmp(list, "all") || !strcmp(list, "1")) return 1;
    size_t n = strlen(name);
    for (const char *p = list; (p = strstr(p, name)); p += n)
        if ((p == list || p[-1] == ',' || p[-1] == ' ') && (p[n] == 0 || p[n] == ',' || p[n] == ' ')) return 1;
    return 0;
}

static void bench_main(void *arg) {
    (void)arg;
    const char *list = cfg_get("TOTK_MGYRO_BENCH"), *o = cfg_get("TOTK_MGYRO_BENCH_OUT"), *d = cfg_get("TOTK_MGYRO_BENCH_DELAY");
    const char *path = o && *o ? o : "mgyro-bench.csv";
    ofile = fopen(path, "w");
    if (!ofile) rt_log("mgyro bench: cannot write %s", path);
    double delay = d && *d ? atof(d) : 40;
    rt_log("mgyro bench: tests '%s', starting %.0f s after the first live six-axis read, CSV %s", list, delay, path);
    // the first read with samples, then the delay
    while (!last_s0) plat_sleep_ns(10000000);
    uint64_t t0 = kern_now_ns();
    while (kern_now_ns() - t0 < (uint64_t)(delay * 1e9)) {
        plat_sleep_ns(50000000);
        out_flush();
    }
    input_human(0);   // the player's gamepad, keys and mouse stay out of the measurement
    out("H,deg_per_count,%.8f", input_mouse_gyro_deg_per_count());   // mouse_gyro_sens, or what cm360 makes
    out("H,mouse_gyro_cm360,%s", cfg_value("TOTK_MOUSE_GYRO_CM360"));
    out("H,mouse_gyro_dpi,%s", cfg_value("TOTK_MOUSE_GYRO_DPI"));
    out("H,mouse_gyro_x,%s", cfg_value("TOTK_MOUSE_GYRO_X"));
    out("H,mouse_gyro_y,%s", cfg_value("TOTK_MOUSE_GYRO_Y"));
    out("H,mouse_gyro_pitch,%s", cfg_value("TOTK_MOUSE_GYRO_PITCH"));
    out("H,heap,%#llx+%#llx", (unsigned long long)mem_layout()->heap.base, (unsigned long long)mem_heap_size());
    // Y: the game recentres its camera behind the player, level (and the virtual controller levels with it)
    shots_setup();
    __atomic_store_n(&shots_left, 1, __ATOMIC_RELAXED);
    wait_reads(5);
    ev("start Y");
    input_inject("Y 6");
    wait_reads(60);
    __atomic_store_n(&shots_left, 1, __ATOMIC_RELAXED);
    wait_reads(5);
    int found = 0;
    for (int tries = 0; tries < 6 && !found; tries++) {
        found = find_aims();
        if (!found) {
            rt_log("mgyro bench: no aim vector found (try %d); again in 10 s", tries + 1);
            plat_sleep_ns(10000000000ull);
        }
    }
    if (!found) rt_log("mgyro bench: no aim vector: the tests log the controller only");
    if (wants(list, "look")) t_look();
    if (wants(list, "idle")) t_idle(10);
    if (wants(list, "lin")) t_lin(0), t_lin(1);
    if (wants(list, "step")) t_step();
    if (wants(list, "phase")) t_phase();
    if (wants(list, "rate")) t_rate();
    if (wants(list, "rt")) t_rt();
    if (wants(list, "fields")) t_fields();
    if (wants(list, "pitchyaw")) t_pitchyaw();
    if (wants(list, "yawaxis")) t_yawaxis();
    if (wants(list, "reverse")) t_reverse();
    if (wants(list, "cap")) t_cap();
    if (wants(list, "recenter")) t_recenter();
    if (wants(list, "limits")) t_limits();
    if (wants(list, "hitch")) t_hitch();
    if (wants(list, "fps")) t_fps();
    if (wants(list, "image")) t_image();
    if (wants(list, "idle")) t_idle(10);
    ev("done");
    wait_reads(10);
    out_flush();
    rt_log("mgyro bench: done, %lld reads", (long long)nreads());
    plat_xchg32(&closed, 1);
    out_flush();
    if (ofile) fclose(ofile), ofile = NULL;
    const char *ex = cfg_get("TOTK_MGYRO_BENCH_EXIT");
    if (ex && atoi(ex)) {
        prof_write();
        plat_exit(0);
    }
}
