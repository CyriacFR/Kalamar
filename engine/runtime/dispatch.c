// Indirect-branch dispatch: guest address -> lifted function, O(1).
//
// Layout of the table: one bit per 4-byte guest instruction over [lo, hi), where lo is the lowest function start of
// any module and hi is past the highest, set where a function starts, in groups of 64 slots; each group also holds how
// many functions start before it, its rank. The host function of slot i is off[rank + set bits of its group below i],
// stored as the function's address minus `anchor` (an address just below the lowest host function), so 4 bytes are
// enough as long as all lifted code lies within 4 GiB, which a single PE image guarantees. Memory: 16 bytes per 64
// guest instructions and 4 bytes per function: 8 MiB for Splatoon 3's 27 million slots and 314,000 functions, where a
// flat table of one 32-bit slot per instruction took 104 MiB of commit and, since nearly every page of it holds a
// function start, 80 MiB of RAM. A lookup is one more load and a popcount; the groups and offsets fit the caches far
// better than the flat table did.
//
// The table is filled once by rt_dispatch_init and then made read-only, so any number of threads read it without
// locks or per-thread caches. Host (HLE) functions at addresses outside the modules live in a small hash table that
// is also fixed at init; they are looked up only after the table misses.
#include "brand.h"
#include "dispatch.h"
#include "host.h"
#include "platform/platform.h"

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>


#define TABLE_SPAN_LIMIT (1ull << 30)   // guest bytes covered by the table, and its size in bytes
#define HOST_CAP 8192                   // host function registry, power of two
#define MAX_MODULES 16

typedef struct { uint64_t bits; uint32_t rank, pad; } Group;   // 64 slots: function starts, functions before them

// Hot data read by every rt_dispatch: one cache line.
static struct __attribute__((aligned(64))) {
    const Group *grp;
    const uint32_t *off;
    uintptr_t anchor;
    uint64_t lo;
    uint64_t slots;
} D;

static const RtModule *g_mods[MAX_MODULES];   // sorted by base
static int g_nmods;
static bool g_ready;
static uint64_t g_table_bytes, g_functions;

static struct { uint64_t addr; GuestFn fn; } g_host[HOST_CAP];
static uint64_t g_nhost;

void rt_fatal(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    rt_log_raw("FATAL: ", 7);
    char msg[2048], shown[2048];   // the text a player reads names the brand's settings file and variables
    vsnprintf(msg, sizeof msg, fmt, ap);
    rt_brand_text(msg, shown, sizeof shown);
    rt_log_raw(shown, strlen(shown));
    rt_log_raw("\n", 1);
    va_end(ap);
    // A failed allocation (malloc, realloc, VirtualAlloc) means the commit limit was reached: this line tells a leak of
    // ours (process private bytes near the limit) from a machine short of page file.
    double proc, used, limit;
    plat_commit_info(&proc, &used, &limit);
    rt_log("fatal: memory commit: this process %.0f MB, the system %.0f of %.0f MB", proc, used, limit);
    kern_free_cores_note();
    fflush(stdout);
    fflush(stderr);
    _Exit(70);
}

static const RtModule *module_at(uint64_t pc) {
    for (int i = 0; i < g_nmods; i++)
        if (pc - g_mods[i]->base < g_mods[i]->size) return g_mods[i];
    return NULL;
}

const char *rt_guest_name(uint64_t pc, char *buf, int size) {
    const RtModule *m = module_at(pc);
    if (m) snprintf(buf, size, "%#llx (%s+%#llx)", (unsigned long long)pc, m->name, (unsigned long long)(pc - m->base));
    else snprintf(buf, size, "%#llx", (unsigned long long)pc);
    return buf;
}

static uint64_t host_hash(uint64_t addr) { return (addr >> 2) * 0x9E3779B97F4A7C15ull >> 51; }   // 13 bits

static GuestFn host_lookup(uint64_t addr) {
    for (uint64_t h = host_hash(addr), n = 0; n < HOST_CAP; h = (h + 1) & (HOST_CAP - 1), n++) {
        if (g_host[h].addr == addr) return g_host[h].fn;
        if (!g_host[h].addr) return NULL;
    }
    return NULL;
}

void rt_register_host(uint64_t addr, GuestFn fn) {
    if (g_ready) rt_fatal("rt_register_host(%#llx) after rt_dispatch_init", (unsigned long long)addr);
    if (!addr || (addr & 3) || !fn) rt_fatal("rt_register_host: bad entry %#llx", (unsigned long long)addr);
    if (g_nhost >= HOST_CAP / 2) rt_fatal("rt_register_host: more than %d host functions", HOST_CAP / 2);
    uint64_t h = host_hash(addr);
    while (g_host[h].addr && g_host[h].addr != addr) h = (h + 1) & (HOST_CAP - 1);
    if (g_host[h].addr) rt_fatal("rt_register_host: %#llx registered twice", (unsigned long long)addr);
    g_host[h].addr = addr;
    g_host[h].fn = fn;
    g_nhost++;
}

static void *table_alloc(uint64_t bytes) {
    return plat_mem_alloc(bytes);   // zero pages, backed on first write
}

static void table_seal(void *p, uint64_t bytes) {
    if (!plat_mem_protect((uint64_t)(uintptr_t)p, bytes, PLAT_PROT_READ)) rt_fatal("dispatch: sealing the table failed");
}

void rt_dispatch_init(const RtModule *mods, int count) {
    if (g_ready) rt_fatal("rt_dispatch_init called twice");
    if (count <= 0 || count > MAX_MODULES) rt_fatal("rt_dispatch_init: %d modules", count);

    // Modules by base, checked for overlap.
    for (int i = 0; i < count; i++) {
        int j = g_nmods++;
        while (j > 0 && g_mods[j - 1]->base > mods[i].base) { g_mods[j] = g_mods[j - 1]; j--; }
        g_mods[j] = &mods[i];
    }
    for (int i = 1; i < g_nmods; i++)
        if (g_mods[i - 1]->base + g_mods[i - 1]->size > g_mods[i]->base)
            rt_fatal("dispatch: modules %s and %s overlap", g_mods[i - 1]->name, g_mods[i]->name);

    // Function tables: sorted, aligned, inside their module. Range of guest starts and of host functions.
    uint64_t lo = UINT64_MAX, hi = 0;
    uintptr_t host_lo = UINTPTR_MAX, host_hi = 0;
    for (int i = 0; i < g_nmods; i++) {
        const RtModule *m = g_mods[i];
        for (uint64_t k = 0; k < m->count; k++) {
            uint64_t a = m->addrs[k];
            uintptr_t f = (uintptr_t)m->fns[k];
            if ((a & 3) || a - m->base >= m->size)
                rt_fatal("dispatch: %s_fn_addrs[%llu] = %#llx is misaligned or outside the module", m->name,
                         (unsigned long long)k, (unsigned long long)a);
            if (k && a <= m->addrs[k - 1])
                rt_fatal("dispatch: %s_fn_addrs is not strictly ascending at [%llu] = %#llx", m->name,
                         (unsigned long long)k, (unsigned long long)a);
            if (!f) rt_fatal("dispatch: %s_fn_ptrs[%llu] is NULL", m->name, (unsigned long long)k);
            if (f < host_lo) host_lo = f;
            if (f > host_hi) host_hi = f;
        }
        if (m->count) {
            if (m->addrs[0] < lo) lo = m->addrs[0];
            if (m->addrs[m->count - 1] + 4 > hi) hi = m->addrs[m->count - 1] + 4;
        }
    }
    if (hi <= lo) rt_fatal("dispatch: no functions in any module");
    if (hi - lo > TABLE_SPAN_LIMIT)
        rt_fatal("dispatch: modules span %#llx guest bytes, over the %#llx the flat table allows",
                 (unsigned long long)(hi - lo), TABLE_SPAN_LIMIT);
    uintptr_t anchor = (host_lo & ~(uintptr_t)15) - 16;
    if (host_hi - anchor > UINT32_MAX)
        rt_fatal("dispatch: lifted code spans %#llx host bytes; slots hold 32-bit offsets (4 GiB)",
                 (unsigned long long)(host_hi - anchor));

    for (uint64_t h = 0; h < HOST_CAP; h++)
        if (g_host[h].addr && module_at(g_host[h].addr))
            rt_fatal("dispatch: host function at %#llx lies inside module %s", (unsigned long long)g_host[h].addr,
                     module_at(g_host[h].addr)->name);

    // The modules are sorted and do not overlap, and each one's starts ascend: in that order the functions come in slot
    // order, which is the order of off[].
    uint64_t slots = (hi - lo) >> 2, ngrp = (slots + 63) >> 6, total = 0;
    for (int i = 0; i < g_nmods; i++) total += g_mods[i]->count;
    uint64_t gbytes = ngrp * sizeof(Group), bytes = (gbytes + total * 4 + 0xFFFF) & ~0xFFFFull;
    uint8_t *mem = table_alloc(bytes);
    if (!mem) rt_fatal("dispatch: cannot allocate a %llu-byte table", (unsigned long long)bytes);
    Group *grp = (Group *)mem;
    uint32_t *off = (uint32_t *)(mem + gbytes);
    uint64_t n = 0;
    for (int i = 0; i < g_nmods; i++) {
        const RtModule *m = g_mods[i];
        for (uint64_t k = 0; k < m->count; k++) {
            uint64_t s = (m->addrs[k] - lo) >> 2;
            grp[s >> 6].bits |= 1ull << (s & 63);
            off[n++] = (uint32_t)((uintptr_t)m->fns[k] - anchor);
        }
        g_functions += m->count;
    }
    uint32_t rank = 0;
    for (uint64_t g = 0; g < ngrp; g++) grp[g].rank = rank, rank += (uint32_t)__builtin_popcountll(grp[g].bits);
    table_seal(mem, bytes);

    g_table_bytes = bytes;
    D.grp = grp;
    D.off = off;
    D.anchor = anchor;
    D.lo = lo;
    D.slots = slots;
    __atomic_store_n(&g_ready, true, __ATOMIC_RELEASE);
}

// The table's entry for slot i (i < D.slots): the host function's offset from D.anchor, 0 when no function starts there.
static inline uint32_t slot_off(uint64_t i) {
    const Group *g = &D.grp[i >> 6];
    uint64_t bits = g->bits, below = (1ull << (i & 63)) - 1;
    if (!(bits >> (i & 63) & 1)) return 0;
    return D.off[g->rank + (uint32_t)__builtin_popcountll(bits & below)];
}

GuestFn rt_dispatch_lookup(uint64_t pc) {
    uint64_t i = __builtin_rotateright64(pc - D.lo, 2);   // a misaligned pc lands far above D.slots
    uint32_t off = i < D.slots ? slot_off(i) : 0;
    if (off) return (GuestFn)(D.anchor + off);
    return g_ready ? host_lookup(pc) : NULL;
}

__attribute__((noinline, noreturn, cold)) static void dispatch_fail(Cpu *c) {
    char at[64], lr[64];
    uint64_t pc = c->pc;
    if (!g_ready) rt_fatal("rt_dispatch to %#llx before rt_dispatch_init", (unsigned long long)pc);
    const char *why = (pc & 3) ? "misaligned address"
                    : module_at(pc) ? "not a function start"
                    : "outside every module, no host function registered";
    rt_fatal("rt_dispatch: no function at guest pc %s: %s (lr %s)", rt_guest_name(pc, at, sizeof at), why,
             rt_guest_name(c->x[30], lr, sizeof lr));
}

// Host functions, and the fatal error. A separate tail call keeps the fast path below free of a stack frame.
__attribute__((noinline)) static void dispatch_slow(Cpu *c) {
    GuestFn f = g_ready ? host_lookup(c->pc) : NULL;
    if (!f) dispatch_fail(c);
    __attribute__((musttail)) return f(c);
}

// A pending preempt request at an indirect call (a safe point: the guest state is all in Cpu). Out of line, so
// rt_dispatch keeps no stack frame.
static __attribute__((noinline)) void dispatch_preempt(Cpu *c) {
    kern_checkpoint();
    __attribute__((musttail)) return rt_dispatch(c);
}

// Jump to the guest address in c->pc. Always a tail call, so chains of indirect jumps keep the host stack flat. A
// pending preempt request costs two loads and a branch to check.
void rt_dispatch(Cpu *c) {
    if (__builtin_expect(c->thread && kern_preempt_pending(c->thread), 0))
        __attribute__((musttail)) return dispatch_preempt(c);
    uint64_t i = __builtin_rotateright64(c->pc - D.lo, 2);
    uint32_t off = i < D.slots ? slot_off(i) : 0;
    if (__builtin_expect(!off, 0)) __attribute__((musttail)) return dispatch_slow(c);
    GuestFn f = (GuestFn)(D.anchor + off);
    __attribute__((musttail)) return f(c);
}

RtDispatchStats rt_dispatch_stats(void) {
    return (RtDispatchStats){D.lo, D.lo + D.slots * 4, D.slots, g_table_bytes, g_functions, g_nhost};
}
