// TOTK_SHDBG: what a fragment shader's discards decide, measured in the running game. RenderDoc cannot capture this
// back end (it hides VK_EXT_external_memory_host), so the shader reports its own values.
//
// TOTK_SHDBG=<hash>[:nokill][,<hash>...] names up to 16 fragment shaders by the FNV-1a hash of their SPIR-V (the "fs"
// names of TOTK_DUMP's passes.txt, vk_dump.c shader_hash). Their SPIR-V is instrumented when a pipeline or a fragment
// stage library is made from it (vk_pipeline.c create_graphics, lib_create): before each discard (OpKill or
// OpTerminateInvocation behind a conditional branch), a fragment inside TOTK_SHDBG_RECT on the TOTK_SHDBG_STEP grid
// appends a 16-word record to a host-visible buffer through its device address (SPV_KHR_physical_storage_buffer):
//   tag (shader index | discard index << 4 | branch condition << 8), window x, y, depth,
//   the two operands of the comparison the condition comes from, then up to ten leaves of the expressions feeding them
//   (texture channels, interpolated inputs, uniform words, each traced through the translator's local variables).
// The log names every recorded value once per shader ("vk_shdbg: ..."), and fs_<hash>.spv is the instrumented module
// (spirv-val checks it). ":nokill" also lets that shader's discards fall through: the fragment is shaded as if the
// condition were false.
// TOTK_SHDBG_RECT=x0,y0,x1,y1   the window pixels recorded (default all); TOTK_SHDBG_STEP=n every n-th pixel in x and y
// (default 4); TOTK_SHDBG_CAP=n records kept per frame (default 262144).
// TOTK_SHDBG_AT=<present> | w<seconds>   recording starts after that present (with w: the first present at or after
// that world-clock time, frame_step_world_us); TOTK_SHDBG_FRAMES=n presents are recorded (default 1). At each of them
// the GPU is drained and the frame's records go to TOTK_SHDBG_DIR (default "shdbg")/frame_<present>.txt: per shader and
// discard the fragments seen, how many the condition discards and the range of every value, then one line per record.
// Only values computed in the discard's own block are recorded (the translator puts a discard's test in the block that
// ends with its branch); the log says when one is left out.
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cfg.h"
#include "host.h"
#include "vk.h"
#include "platform/platform.h"

#define MAX_SH 16
#define MAX_GUARDS 4
#define NV 12          // values after the tag and the position: comparison operands a, b and ten leaves
#define REC_WORDS 16
#define HDR_WORDS 16   // word 0: records appended (an atomic counter), word 1: capacity

enum { K_NONE, K_FLOAT, K_UINT, K_INT, K_BOOL };
typedef struct { int n, inverted; uint8_t kind[NV]; char what[NV][112]; } Guard;
typedef struct { uint64_t hash; int nokill, ng, described; Guard g[MAX_GUARDS]; } Sh;

static struct {
    int n;
    Sh sh[MAX_SH];
    uint32_t x0, y0, x1, y1, step, cap;
    uint64_t at, frames;
    int at_world;
    char dir[400];
    PlatMutex lock;
    VkBuffer buf;
    VmaAllocation alloc;
    uint32_t *map;
    VkDeviceAddress addr;
    int buf_failed, state;   // state 0: waiting for TOTK_SHDBG_AT, 1: recording, 2: done
    uint64_t left;
} D = {.lock = PLAT_MUTEX_INIT};
static PlatOnce parse_once = PLAT_ONCE_INIT;

static void parse(void *arg) {
    (void)arg;
    const char *e = cfg_get("TOTK_SHDBG");
    for (const char *p = e; p && *p && D.n < MAX_SH;) {
        char *q;
        uint64_t h = strtoull(p, &q, 16);
        if (q == p) break;
        Sh *s = &D.sh[D.n++];
        s->hash = h;
        if (!strncmp(q, ":nokill", 7)) s->nokill = 1, q += 7;
        while (*q && *q != ',') q++;
        p = *q ? q + 1 : q;
    }
    if (!D.n) return;
    D.x1 = D.y1 = 65535;
    const char *r = cfg_get("TOTK_SHDBG_RECT");
    if (r && sscanf(r, "%u,%u,%u,%u", &D.x0, &D.y0, &D.x1, &D.y1) != 4) D.x0 = D.y0 = 0, D.x1 = D.y1 = 65535;
    if (D.x1 <= D.x0) D.x1 = D.x0 + 1;
    if (D.y1 <= D.y0) D.y1 = D.y0 + 1;
    const char *st = cfg_get("TOTK_SHDBG_STEP");
    D.step = st && atoi(st) > 0 ? (uint32_t)atoi(st) : 4;
    const char *cp = cfg_get("TOTK_SHDBG_CAP");
    D.cap = cp && atoi(cp) > 0 ? (uint32_t)atoi(cp) : 1u << 18;
    const char *at = cfg_get("TOTK_SHDBG_AT");
    if (at && *at == 'w') D.at_world = 1, D.at = (uint64_t)(atof(at + 1) * 1e6);
    else D.at = at && *at ? strtoull(at, NULL, 10) : 1;
    const char *fr = cfg_get("TOTK_SHDBG_FRAMES");
    D.frames = fr && atoi(fr) > 0 ? (uint64_t)atoi(fr) : 1;
    const char *d = cfg_get("TOTK_SHDBG_DIR");
    snprintf(D.dir, sizeof D.dir, "%s", d && *d ? d : "shdbg");
    plat_file_mkdir_p(D.dir);
    rt_log("vk_shdbg: %d fragment shader(s) instrumented; window %u,%u..%u,%u every %u px, %u records a frame, from %s%llu "
           "for %llu present(s), into %s", D.n, D.x0, D.y0, D.x1, D.y1, D.step, D.cap,
           D.at_world ? "world-clock us " : "present ", (unsigned long long)D.at, (unsigned long long)D.frames, D.dir);
}

// Under D.lock.
static int buffer_ready(void) {
    if (D.addr) return 1;
    if (D.buf_failed) return 0;
    VkCtx *c = vk_ctx();
    D.buf_failed = 1;
    if (!c || !c->has_buffer_device_address) {
        rt_log("vk_shdbg: no bufferDeviceAddress on this device: shaders are not instrumented");
        return 0;
    }
    VkDeviceSize size = ((VkDeviceSize)HDR_WORDS + (VkDeviceSize)D.cap * REC_WORDS) * 4;
    VkBufferCreateInfo bi = {.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO, .size = size,
                             .usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT | VK_BUFFER_USAGE_SHADER_DEVICE_ADDRESS_BIT};
    VmaAllocationCreateInfo ai = {.usage = VMA_MEMORY_USAGE_AUTO,
                                  .flags = VMA_ALLOCATION_CREATE_HOST_ACCESS_RANDOM_BIT | VMA_ALLOCATION_CREATE_MAPPED_BIT};
    VmaAllocationInfo info;
    if (vmaCreateBuffer(c->vma, &bi, &ai, &D.buf, &D.alloc, &info) != VK_SUCCESS || !info.pMappedData) {
        rt_log("vk_shdbg: record buffer allocation failed");
        return 0;
    }
    D.map = info.pMappedData;
    memset(D.map, 0, (size_t)size);
    D.map[1] = D.cap;
    vmaFlushAllocation(c->vma, D.alloc, 0, VK_WHOLE_SIZE);
    VkBufferDeviceAddressInfo ba = {.sType = VK_STRUCTURE_TYPE_BUFFER_DEVICE_ADDRESS_INFO, .buffer = D.buf};
    __atomic_store_n(&D.addr, vkGetBufferDeviceAddress(c->device, &ba), __ATOMIC_RELEASE);
    D.buf_failed = 0;
    return 1;
}

// ---- SPIR-V (specification 1.6, section 3)

enum {
    OP_NAME = 5, OP_EXTENSION = 10, OP_EXT_INST = 12, OP_MEMORY_MODEL = 14, OP_ENTRY_POINT = 15, OP_CAPABILITY = 17,
    OP_TYPE_BOOL = 20, OP_TYPE_INT = 21, OP_TYPE_FLOAT = 22, OP_TYPE_VECTOR = 23, OP_TYPE_RUNTIME_ARRAY = 29,
    OP_TYPE_STRUCT = 30, OP_TYPE_POINTER = 32, OP_CONSTANT = 43, OP_FUNCTION = 54, OP_VARIABLE = 59, OP_LOAD = 61,
    OP_STORE = 62, OP_ACCESS_CHAIN = 65, OP_IN_BOUNDS_ACCESS_CHAIN = 66, OP_DECORATE = 71, OP_MEMBER_DECORATE = 72,
    OP_VECTOR_SHUFFLE = 79, OP_COMPOSITE_EXTRACT = 81, OP_SAMPLE_FIRST = 87, OP_SAMPLE_LAST = 98, OP_CONVERT_F_TO_U = 109,
    OP_CONVERT_U_TO_PTR = 120, OP_BITCAST = 124, OP_I_ADD = 128, OP_I_SUB = 130, OP_I_MUL = 132, OP_U_MOD = 137,
    OP_LOGICAL_AND = 167, OP_SELECT = 169, OP_I_EQUAL = 170, OP_U_LESS_THAN = 176, OP_BITWISE_OR = 197,
    OP_ATOMIC_I_ADD = 234, OP_PHI = 245, OP_LOOP_MERGE = 246, OP_SELECTION_MERGE = 247, OP_LABEL = 248, OP_BRANCH = 249,
    OP_BRANCH_CONDITIONAL = 250, OP_KILL = 252, OP_TERMINATE_INVOCATION = 4416,
    SC_INPUT = 1, SC_UNIFORM = 2, SC_FUNCTION = 7, SC_PSB = 5349, DEC_BLOCK = 2, DEC_ARRAY_STRIDE = 6, DEC_BUILTIN = 11,
    DEC_LOCATION = 30, DEC_OFFSET = 35, BUILTIN_FRAG_COORD = 15, CAP_INT64 = 11, CAP_PSB = 5347, ADDR_PSB64 = 5348,
};

// Instructions with neither a result type nor a result id inside a function (the ones a fragment shader can hold).
static int no_result(uint32_t op) {
    switch (op) {
    case 0: case 8: case 62: case 63: case 64: case 99: case 218: case 219: case 224: case 225: case 228: case 246: case 247:
    case 249: case 250: case 251: case 252: case 253: case 254: case 255: case 56: case 317: case 4416: case 5364: case 5365:
    case 5380:
        return 1;
    }
    return 0;
}

// comparisons with two operands (OpLogicalEqual .. OpFUnordGreaterThanEqual, not the logical and, or and not)
static int is_compare(uint32_t op) { return op >= 164 && op <= 191 && (op < 166 || op > 168); }
// value operands from word 3 of the instruction to its end (result type and id before them)
static int is_arith(uint32_t op) {
    return (op >= 109 && op <= 115) || op == 124 || (op >= 126 && op <= 142) || op == 148 || op == 156 || op == 157 ||
           (op >= 164 && op <= 191) || op == OP_SELECT;
}

typedef struct {
    const uint32_t *w;
    uint64_t n;
    uint32_t bound, m;           // m: instructions
    uint64_t *at;                // word index of instruction k
    uint32_t *def;               // instruction ordinal defining an id + 1 (0: none)
    uint32_t *rtype;             // result type of an id
    uint32_t *sc;                // storage class of a variable or pointer type, + 1
    uint32_t *pointee;           // pointer type -> pointee
    uint32_t *loc;               // Location + 1 of a variable
    uint32_t *builtin;           // BuiltIn + 1 of a variable
    uint32_t *name;              // ordinal of the OpName of an id + 1
    uint8_t *tkind;              // K_* of a scalar type
    uint8_t *is_const;
    uint32_t *cval;              // value of a 32-bit OpConstant
} Mod;

static uint32_t op_of(const Mod *M, uint32_t k) { return M->w[M->at[k]] & 0xffff; }
static uint32_t len_of(const Mod *M, uint32_t k) { return M->w[M->at[k]] >> 16; }
static const uint32_t *args(const Mod *M, uint32_t k) { return M->w + M->at[k] + 1; }

static void mod_free(Mod *M) {
    free(M->at), free(M->def), free(M->rtype), free(M->sc), free(M->pointee), free(M->loc), free(M->builtin), free(M->name);
    free(M->tkind), free(M->is_const), free(M->cval);
}

static int mod_parse(Mod *M, const uint32_t *w, uint64_t n) {
    memset(M, 0, sizeof *M);
    M->w = w, M->n = n;
    M->bound = w[3] < (1u << 22) ? w[3] : 0;
    if (!M->bound) return 0;
    uint32_t b = M->bound;
    M->at = malloc(n * sizeof *M->at);
    M->def = calloc(b, 4), M->rtype = calloc(b, 4), M->sc = calloc(b, 4), M->pointee = calloc(b, 4), M->loc = calloc(b, 4);
    M->builtin = calloc(b, 4), M->name = calloc(b, 4), M->tkind = calloc(b, 1), M->is_const = calloc(b, 1);
    M->cval = calloc(b, 4);
    if (!M->at || !M->def || !M->rtype || !M->sc || !M->pointee || !M->loc || !M->builtin || !M->name || !M->tkind ||
        !M->is_const || !M->cval)
        return 0;
    int in_fn = 0;
    for (uint64_t i = 5; i < n;) {
        uint32_t op = w[i] & 0xffff, len = w[i] >> 16;
        if (!len || i + len > n) return 0;
        const uint32_t *a = w + i + 1;
        uint32_t k = M->m++;
        M->at[k] = i;
#define ID_OK(x) ((x) < b)
        if (op == OP_FUNCTION) in_fn = 1;
        if (op == OP_NAME && len >= 2 && ID_OK(a[0])) M->name[a[0]] = k + 1;
        else if (op == OP_DECORATE && len >= 4 && ID_OK(a[0]) && a[1] == DEC_LOCATION) M->loc[a[0]] = a[2] + 1;
        else if (op == OP_DECORATE && len >= 4 && ID_OK(a[0]) && a[1] == DEC_BUILTIN) M->builtin[a[0]] = a[2] + 1;
        else if (op == OP_TYPE_INT && len >= 4 && ID_OK(a[0]) && a[1] == 32)
            M->tkind[a[0]] = a[2] ? K_INT : K_UINT, M->def[a[0]] = k + 1;
        else if (op == OP_TYPE_FLOAT && len >= 3 && ID_OK(a[0]) && a[1] == 32) M->tkind[a[0]] = K_FLOAT, M->def[a[0]] = k + 1;
        else if (op == OP_TYPE_BOOL && len >= 2 && ID_OK(a[0])) M->tkind[a[0]] = K_BOOL, M->def[a[0]] = k + 1;
        else if (op == OP_TYPE_POINTER && len >= 4 && ID_OK(a[0]))
            M->sc[a[0]] = a[1] + 1, M->pointee[a[0]] = a[2], M->def[a[0]] = k + 1;
        else if (op >= 19 && op <= 39 && len >= 2 && ID_OK(a[0])) M->def[a[0]] = k + 1;   // other types
        else if (op >= 41 && op <= 52 && len >= 3 && ID_OK(a[1])) {   // constants and spec constants
            M->is_const[a[1]] = 1, M->rtype[a[1]] = a[0], M->def[a[1]] = k + 1;
            if (op == OP_CONSTANT && len == 4) M->cval[a[1]] = a[2];
        } else if (op == OP_VARIABLE && len >= 4 && ID_OK(a[1])) {
            M->rtype[a[1]] = a[0], M->def[a[1]] = k + 1, M->sc[a[1]] = a[2] + 1;
        } else if (in_fn && op == OP_LABEL && len >= 2 && ID_OK(a[0])) {
            M->def[a[0]] = k + 1;
        } else if (in_fn && !no_result(op) && op != OP_LABEL && len >= 3 && ID_OK(a[1])) {
            M->def[a[1]] = k + 1, M->rtype[a[1]] = a[0];
        }
#undef ID_OK
        i += len;
    }
    return 1;
}

// Ordinal of the instruction defining id, or -1.
static int64_t def_of(const Mod *M, uint32_t id) { return id < M->bound && M->def[id] ? (int64_t)M->def[id] - 1 : -1; }

// The variable an access chain (or the pointer itself) starts from.
static uint32_t base_var(const Mod *M, uint32_t ptr) {
    for (int g = 0; g < 8; g++) {
        int64_t d = def_of(M, ptr);
        if (d < 0) return 0;
        uint32_t op = op_of(M, (uint32_t)d);
        if (op == OP_VARIABLE) return ptr;
        if (op != OP_ACCESS_CHAIN && op != OP_IN_BOUNDS_ACCESS_CHAIN) return 0;
        ptr = args(M, (uint32_t)d)[2];
    }
    return 0;
}

// A load from a function-local variable: the value of the last whole store to it before the load, in the same
// function (the translator keeps its registers in such variables). Otherwise id itself.
static uint32_t through_store(const Mod *M, uint32_t id) {
    for (int g = 0; g < 16; g++) {
        int64_t d = def_of(M, id);
        if (d < 0 || op_of(M, (uint32_t)d) != OP_LOAD) return id;
        uint32_t ptr = args(M, (uint32_t)d)[2];
        int64_t pd = def_of(M, ptr);
        if (ptr >= M->bound || M->sc[ptr] != SC_FUNCTION + 1 || pd < 0 || op_of(M, (uint32_t)pd) != OP_VARIABLE) return id;
        uint32_t found = 0;
        for (int64_t k = d - 1; k >= 0; k--) {
            uint32_t op = op_of(M, (uint32_t)k);
            if (op == OP_FUNCTION) break;
            if (op == OP_STORE && args(M, (uint32_t)k)[0] == ptr) { found = args(M, (uint32_t)k)[1]; break; }
        }
        if (!found) return id;
        id = found;
    }
    return id;
}

typedef struct { uint32_t id[NV]; int n; } Vals;

static void add_val(Vals *v, uint32_t id) {
    for (int i = 0; i < v->n; i++)
        if (v->id[i] == id) return;
    if (v->n < NV) v->id[v->n++] = id;
}

static void leaves(const Mod *M, uint32_t id, Vals *v, int depth) {
    if (depth > 24 || v->n >= NV || id >= M->bound || M->is_const[id]) return;
    int64_t d = def_of(M, id);
    if (d < 0) return;
    uint32_t op = op_of(M, (uint32_t)d), len = len_of(M, (uint32_t)d);
    const uint32_t *a = args(M, (uint32_t)d);
    if (op == OP_VARIABLE || op == OP_LABEL) return;
    if (op == OP_LOAD) {
        uint32_t s = through_store(M, id);
        if (s != id) leaves(M, s, v, depth + 1);
        else add_val(v, id);
    } else if (op == OP_EXT_INST) {
        for (uint32_t k = 4; k < len - 1; k++) leaves(M, a[k], v, depth + 1);
    } else if (is_arith(op)) {
        for (uint32_t k = 2; k < len - 1; k++) leaves(M, a[k], v, depth + 1);
    } else {
        add_val(v, id);
    }
}

static const char *name_of(const Mod *M, uint32_t id, char *buf, size_t n) {
    if (id < M->bound && M->name[id]) {
        uint32_t k = M->name[id] - 1, len = len_of(M, k);
        snprintf(buf, n, "%.*s", (int)((len - 2) * 4), (const char *)(args(M, k) + 1));
    } else {
        snprintf(buf, n, "%%%u", id);
    }
    return buf;
}

static void describe(const Mod *M, uint32_t id, char *o, size_t n) {
    static const char xyzw[] = "xyzw";
    char nm[64];
    int64_t d = def_of(M, id);
    uint32_t op = d >= 0 ? op_of(M, (uint32_t)d) : 0;
    const uint32_t *a = d >= 0 ? args(M, (uint32_t)d) : NULL;
    if (op == OP_LOAD) {
        uint32_t ptr = a[2], var = base_var(M, ptr);
        size_t k = (size_t)snprintf(o, n, "%%%u = ", id);
        if (var && M->builtin[var]) k += (size_t)snprintf(o + k, n - k, "builtin%u", M->builtin[var] - 1);
        else if (var && M->sc[var] == SC_INPUT + 1 && M->loc[var]) k += (size_t)snprintf(o + k, n - k, "in%u", M->loc[var] - 1);
        else if (var) k += (size_t)snprintf(o + k, n - k, "%s", name_of(M, var, nm, sizeof nm));
        else k += (size_t)snprintf(o + k, n - k, "*%%%u", ptr);
        int64_t pd = def_of(M, ptr);
        if (pd >= 0 && ptr != var && k < n) {   // the access chain's indices
            uint32_t pl = len_of(M, (uint32_t)pd);
            const uint32_t *pa = args(M, (uint32_t)pd);
            for (uint32_t j = 3; j < pl - 1 && k < n; j++) {
                uint32_t ix = pa[j];
                if (ix < M->bound && M->is_const[ix]) k += (size_t)snprintf(o + k, n - k, "[%u]", M->cval[ix]);
                else k += (size_t)snprintf(o + k, n - k, "[%%%u]", ix);
            }
        }
        return;
    }
    if (op == OP_COMPOSITE_EXTRACT && len_of(M, (uint32_t)d) >= 5) {
        uint32_t comp = a[3], base = a[2];
        for (int g = 0; g < 8; g++) {   // through local variables and shuffles to the instruction that made the vector
            base = through_store(M, base);
            int64_t bd = def_of(M, base);
            if (bd < 0 || op_of(M, (uint32_t)bd) != OP_VECTOR_SHUFFLE) break;
            const uint32_t *sa = args(M, (uint32_t)bd);
            uint32_t sl = len_of(M, (uint32_t)bd);
            if (comp + 5 >= sl) break;
            uint32_t pick = sa[4 + comp];
            if (pick >= 4) break;   // from the second vector: not followed
            comp = pick, base = sa[2];
        }
        int64_t bd = def_of(M, base);
        uint32_t bop = bd >= 0 ? op_of(M, (uint32_t)bd) : 0;
        if (bop >= OP_SAMPLE_FIRST && bop <= OP_SAMPLE_LAST) {
            const uint32_t *sa = args(M, (uint32_t)bd);
            uint32_t img = through_store(M, sa[2]), var = 0;
            int64_t id2 = def_of(M, img);
            if (id2 >= 0 && op_of(M, (uint32_t)id2) == OP_LOAD) var = base_var(M, args(M, (uint32_t)id2)[2]);
            snprintf(o, n, "%%%u = tex(%s).%c, op %u, coordinate %%%u", id, var ? name_of(M, var, nm, sizeof nm) : "?",
                     comp < 4 ? xyzw[comp] : '?', bop, sa[3]);
            return;
        }
        snprintf(o, n, "%%%u = %%%u.%c (op %u)", id, base, comp < 4 ? xyzw[comp] : '?', bop);
        return;
    }
    snprintf(o, n, "%%%u = op %u", id, op);
}

typedef struct {
    uint32_t ip, gb_label, after, kill, merge, cond, tag;
    int inverted;
    Vals v;
} GuardAt;

// Output buffer
typedef struct { uint32_t *w; uint64_t n, cap; } Out;
static void put(Out *o, uint32_t x) {
    if (o->n == o->cap) {
        o->cap = o->cap ? o->cap * 2 : 4096;
        o->w = realloc(o->w, o->cap * 4);
        if (!o->w) rt_fatal("vk_shdbg: out of memory");
    }
    o->w[o->n++] = x;
}
static void ins(Out *o, uint32_t op, int nargs, const uint32_t *a) {
    put(o, (uint32_t)(nargs + 1) << 16 | op);
    for (int i = 0; i < nargs; i++) put(o, a[i]);
}
#define INS(o, op, ...) do { const uint32_t a_[] = {__VA_ARGS__}; ins(o, op, (int)(sizeof a_ / 4), a_); } while (0)

static uint32_t *instrument(const uint32_t *w, uint64_t n, int si, uint64_t *out_n) {
    Mod M;
    Out o = {0};
    uint32_t *res = NULL;
    Sh *sh = &D.sh[si];
    if (!mod_parse(&M, w, n)) goto out;
    uint32_t next = M.bound;
    // sections and reusable types
    uint32_t cap_end = 0, mm = 0, ep = 0, annot_end = 0, fn0 = 0, t_uint = 0, t_bool = 0, t_float = 0, t_ulong = 0, t_v4f = 0;
    uint32_t fc_var = 0, have_int64 = 0;
    for (uint32_t k = 0; k < M.m; k++) {
        uint32_t op = op_of(&M, k), len = len_of(&M, k);
        const uint32_t *a = args(&M, k);
        if (op == OP_CAPABILITY) cap_end = k + 1, have_int64 |= a[0] == CAP_INT64;
        else if (op == OP_MEMORY_MODEL) mm = k;
        else if (op == OP_ENTRY_POINT) ep = k;
        else if (!annot_end && ((op >= 19 && op <= 52) || op == OP_VARIABLE || op == 1)) annot_end = k;
        if (op == OP_FUNCTION && !fn0) fn0 = k;
        if (op == OP_TYPE_INT && len >= 4 && a[1] == 32 && !a[2] && !t_uint) t_uint = a[0];
        if (op == OP_TYPE_INT && len >= 4 && a[1] == 64 && !a[2] && !t_ulong) t_ulong = a[0];
        if (op == OP_TYPE_FLOAT && len >= 3 && a[1] == 32 && !t_float) t_float = a[0];
        if (op == OP_TYPE_BOOL && !t_bool) t_bool = a[0];
        if (op == OP_TYPE_VECTOR && len >= 4 && a[1] == t_float && t_float && a[2] == 4 && !t_v4f) t_v4f = a[0];
        if (op == OP_VARIABLE && len >= 4 && a[1] < M.bound && M.builtin[a[1]] == BUILTIN_FRAG_COORD + 1) fc_var = a[1];
    }
    if (!cap_end || !mm || !ep || !annot_end || !fn0) goto out;
    // the discards and the blocks that decide them (described in lg, copied into sh once)
    GuardAt G[MAX_GUARDS];
    Guard lg[MAX_GUARDS];
    int ng = 0;
    for (uint32_t k = fn0; k < M.m && ng < MAX_GUARDS; k++) {
        uint32_t op = op_of(&M, k);
        if (op != OP_KILL && op != OP_TERMINATE_INVOCATION) continue;
        int64_t kl = k;
        while (kl > 0 && op_of(&M, (uint32_t)kl) != OP_LABEL) kl--;
        uint32_t label = args(&M, (uint32_t)kl)[0];
        int64_t bc = -1;
        int inverted = 0;
        for (uint32_t j = fn0; j < M.m; j++)
            if (op_of(&M, j) == OP_BRANCH_CONDITIONAL && (args(&M, j)[1] == label || args(&M, j)[2] == label)) {
                bc = j, inverted = args(&M, j)[1] != label;
                break;
            }
        unsigned long long hh = (unsigned long long)sh->hash;
        if (bc < 0) {
            rt_log("vk_shdbg: %016llx: discard %d has no conditional branch to it; left alone", hh, ng);
            continue;
        }
        GuardAt *g = &G[ng];
        memset(g, 0, sizeof *g);
        g->kill = k, g->inverted = inverted, g->cond = args(&M, (uint32_t)bc)[0];
        g->ip = (uint32_t)bc;
        if (bc > 0 && op_of(&M, (uint32_t)bc - 1) == OP_SELECTION_MERGE)
            g->ip = (uint32_t)bc - 1, g->merge = args(&M, (uint32_t)bc - 1)[0];
        else
            g->merge = args(&M, (uint32_t)bc)[inverted ? 1 : 2];
        int64_t gb = g->ip;
        while (gb > 0 && op_of(&M, (uint32_t)gb) != OP_LABEL) gb--;
        int loop = 0;
        for (int64_t j = gb; j < g->ip; j++) loop |= op_of(&M, (uint32_t)j) == OP_LOOP_MERGE;
        if (loop) {
            rt_log("vk_shdbg: %016llx: discard %d is decided in a loop header; left alone", hh, ng);
            continue;
        }
        g->gb_label = args(&M, (uint32_t)gb)[0];
        int64_t cdef = def_of(&M, g->cond);
        if (cdef <= gb || cdef >= g->ip) {
            rt_log("vk_shdbg: %016llx: discard %d tests a value from another block; left alone", hh, ng);
            continue;
        }
        // the comparison the condition comes from, its operands, the leaves behind them
        uint32_t c = through_store(&M, g->cond);
        int64_t cd = def_of(&M, c);
        Vals v = {0};
        if (cd >= 0 && is_compare(op_of(&M, (uint32_t)cd))) {
            v.id[0] = args(&M, (uint32_t)cd)[2], v.id[1] = args(&M, (uint32_t)cd)[3], v.n = 2;
        } else {
            v.id[0] = c, v.id[1] = 0, v.n = 2;
        }
        Vals lf = {0};
        for (int j = 0; j < 2; j++)
            if (v.id[j]) add_val(&lf, v.id[j]);
        int nlf0 = lf.n;
        for (int j = 0; j < 2; j++)
            if (v.id[j]) leaves(&M, v.id[j], &lf, 0);
        for (int j = nlf0; j < lf.n && v.n < NV; j++) v.id[v.n++] = lf.id[j];
        // keep what the guard block computes; constants are described, not recorded
        Guard *dg = &lg[ng];
        memset(dg, 0, sizeof *dg);
        dg->n = v.n, dg->inverted = inverted;
        for (int j = 0; j < v.n; j++) {
            uint32_t id = v.id[j];
            dg->kind[j] = K_NONE;
            if (!id) { snprintf(dg->what[j], sizeof dg->what[j], "-"); continue; }
            int64_t d = def_of(&M, id);
            uint32_t t = id < M.bound ? M.rtype[id] : 0;
            uint8_t kind = t < M.bound ? M.tkind[t] : K_NONE;
            describe(&M, id, dg->what[j], sizeof dg->what[j]);
            if (id < M.bound && M.is_const[id]) {
                char tmp[48];
                float f;
                memcpy(&f, &M.cval[id], 4);
                snprintf(tmp, sizeof tmp, "constant %#x (%g)", M.cval[id], kind == K_FLOAT ? (double)f : (double)M.cval[id]);
                snprintf(dg->what[j], sizeof dg->what[j], "%s", tmp);
                v.id[j] = 0;
                continue;
            }
            if (d <= gb || d >= g->ip || kind == K_NONE) {
                size_t l = strlen(dg->what[j]);
                snprintf(dg->what[j] + l, sizeof dg->what[j] - l, kind == K_NONE ? " (not a 32-bit scalar: left out)"
                                                                                 : " (outside the discard's block: left out)");
                v.id[j] = 0;
                continue;
            }
            dg->kind[j] = kind;
        }
        g->v = v;
        g->tag = (uint32_t)si | (uint32_t)ng << 4;
        ng++;
    }
    if (!ng) { rt_log("vk_shdbg: %016llx: no discard to instrument", (unsigned long long)sh->hash); goto out; }
    // ids
    uint32_t n_t_uint = t_uint ? 0 : (t_uint = next++), n_t_bool = t_bool ? 0 : (t_bool = next++);
    uint32_t n_t_float = t_float ? 0 : (t_float = next++), n_t_ulong = t_ulong ? 0 : (t_ulong = next++);
    uint32_t n_t_v4f = t_v4f ? 0 : (t_v4f = next++);
    uint32_t t_rta = next++, t_S = next++, t_pS = next++, t_pu = next++, t_pin = 0, n_fc = 0;
    if (!fc_var) t_pin = next++, fc_var = n_fc = next++;
    uint32_t c_addr = next++, c_k[16], c_cap = next++, c_x0 = next++, c_xw = next++, c_y0 = next++, c_yh = next++;
    uint32_t c_step = next++, c_16 = next++, c_256 = next++, c_tag[MAX_GUARDS];
    for (int k = 0; k < 16; k++) c_k[k] = next++;
    for (int k = 0; k < ng; k++) c_tag[k] = next++, G[k].after = next++;
    // header, capabilities, the extension, the memory model
    for (int i = 0; i < 5; i++) put(&o, w[i]);
    uint32_t bound_at = 3;
    for (uint32_t k = 0; k < M.m; k++) {
        uint32_t op = op_of(&M, k), len = len_of(&M, k);
        const uint32_t *src = w + M.at[k];
        if (k == cap_end) {
            if (!have_int64) INS(&o, OP_CAPABILITY, CAP_INT64);
            INS(&o, OP_CAPABILITY, CAP_PSB);
            static const char ext[] = "SPV_KHR_physical_storage_buffer";   // 31 bytes and the NUL: 8 words
            uint32_t ew[8] = {0};
            memcpy(ew, ext, sizeof ext);
            ins(&o, OP_EXTENSION, 8, ew);
        }
        if (k == annot_end) {
            INS(&o, OP_DECORATE, t_rta, DEC_ARRAY_STRIDE, 4);
            INS(&o, OP_DECORATE, t_S, DEC_BLOCK);
            INS(&o, OP_MEMBER_DECORATE, t_S, 0, DEC_OFFSET, 0);
            if (n_fc) INS(&o, OP_DECORATE, n_fc, DEC_BUILTIN, BUILTIN_FRAG_COORD);
        }
        if (k == fn0) {
            if (n_t_uint) INS(&o, OP_TYPE_INT, n_t_uint, 32, 0);
            if (n_t_bool) INS(&o, OP_TYPE_BOOL, n_t_bool);
            if (n_t_float) INS(&o, OP_TYPE_FLOAT, n_t_float, 32);
            if (n_t_ulong) INS(&o, OP_TYPE_INT, n_t_ulong, 64, 0);
            if (n_t_v4f) INS(&o, OP_TYPE_VECTOR, n_t_v4f, t_float, 4);
            INS(&o, OP_TYPE_RUNTIME_ARRAY, t_rta, t_uint);
            INS(&o, OP_TYPE_STRUCT, t_S, t_rta);
            INS(&o, OP_TYPE_POINTER, t_pS, SC_PSB, t_S);
            INS(&o, OP_TYPE_POINTER, t_pu, SC_PSB, t_uint);
            if (n_fc) {
                INS(&o, OP_TYPE_POINTER, t_pin, SC_INPUT, t_v4f);
                INS(&o, OP_VARIABLE, t_pin, n_fc, SC_INPUT);
            }
            uint64_t addr = __atomic_load_n(&D.addr, __ATOMIC_ACQUIRE);
            INS(&o, OP_CONSTANT, t_ulong, c_addr, (uint32_t)addr, (uint32_t)(addr >> 32));
            for (int j = 0; j < 16; j++) INS(&o, OP_CONSTANT, t_uint, c_k[j], (uint32_t)j);
            INS(&o, OP_CONSTANT, t_uint, c_cap, D.cap);
            INS(&o, OP_CONSTANT, t_uint, c_x0, D.x0);
            INS(&o, OP_CONSTANT, t_uint, c_xw, D.x1 - D.x0);
            INS(&o, OP_CONSTANT, t_uint, c_y0, D.y0);
            INS(&o, OP_CONSTANT, t_uint, c_yh, D.y1 - D.y0);
            INS(&o, OP_CONSTANT, t_uint, c_step, D.step);
            INS(&o, OP_CONSTANT, t_uint, c_16, 16);
            INS(&o, OP_CONSTANT, t_uint, c_256, 256);
            for (int j = 0; j < ng; j++) INS(&o, OP_CONSTANT, t_uint, c_tag[j], G[j].tag);
        }
        for (int j = 0; j < ng; j++) {
            if (k != G[j].ip) continue;
            GuardAt *g = &G[j];
            uint32_t fc = next++, fx = next++, fy = next++, fz = next++, ux = next++, uy = next++, dx = next++, ix = next++;
            uint32_t dy = next++, iy = next++, mx = next++, zx = next++, my = next++, zy = next++, a1 = next++, a2 = next++;
            uint32_t cc = next++, l_rec = next++, l_wr = next++, l_after2 = next++;
            INS(&o, OP_LOAD, t_v4f, fc, fc_var);
            INS(&o, OP_COMPOSITE_EXTRACT, t_float, fx, fc, 0);
            INS(&o, OP_COMPOSITE_EXTRACT, t_float, fy, fc, 1);
            INS(&o, OP_COMPOSITE_EXTRACT, t_float, fz, fc, 2);
            INS(&o, OP_CONVERT_F_TO_U, t_uint, ux, fx);
            INS(&o, OP_CONVERT_F_TO_U, t_uint, uy, fy);
            INS(&o, OP_I_SUB, t_uint, dx, ux, c_x0);
            INS(&o, OP_U_LESS_THAN, t_bool, ix, dx, c_xw);
            INS(&o, OP_I_SUB, t_uint, dy, uy, c_y0);
            INS(&o, OP_U_LESS_THAN, t_bool, iy, dy, c_yh);
            INS(&o, OP_U_MOD, t_uint, mx, ux, c_step);
            INS(&o, OP_I_EQUAL, t_bool, zx, mx, c_k[0]);
            INS(&o, OP_U_MOD, t_uint, my, uy, c_step);
            INS(&o, OP_I_EQUAL, t_bool, zy, my, c_k[0]);
            INS(&o, OP_LOGICAL_AND, t_bool, a1, ix, iy);
            INS(&o, OP_LOGICAL_AND, t_bool, a2, zx, zy);
            INS(&o, OP_LOGICAL_AND, t_bool, cc, a1, a2);
            INS(&o, OP_SELECTION_MERGE, g->after, 0);
            INS(&o, OP_BRANCH_CONDITIONAL, cc, l_rec, g->after);
            INS(&o, OP_LABEL, l_rec);
            uint32_t base = next++, pc = next++, slot = next++, ok = next++;
            INS(&o, OP_CONVERT_U_TO_PTR, t_pS, base, c_addr);
            INS(&o, OP_ACCESS_CHAIN, t_pu, pc, base, c_k[0], c_k[0]);
            INS(&o, OP_ATOMIC_I_ADD, t_uint, slot, pc, c_k[1], c_k[0], c_k[1]);   // scope Device, relaxed
            INS(&o, OP_U_LESS_THAN, t_bool, ok, slot, c_cap);
            INS(&o, OP_SELECTION_MERGE, l_after2, 0);
            INS(&o, OP_BRANCH_CONDITIONAL, ok, l_wr, l_after2);
            INS(&o, OP_LABEL, l_wr);
            uint32_t r16 = next++, rb = next++;
            INS(&o, OP_I_MUL, t_uint, r16, slot, c_16);
            INS(&o, OP_I_ADD, t_uint, rb, r16, c_16);
            for (int f = 0; f < REC_WORDS; f++) {
                uint32_t val;
                if (f == 0) {
                    uint32_t cb = next++;
                    val = next++;
                    INS(&o, OP_SELECT, t_uint, cb, g->cond, c_256, c_k[0]);
                    INS(&o, OP_BITWISE_OR, t_uint, val, c_tag[j], cb);
                } else if (f <= 3) {
                    val = next++;
                    INS(&o, OP_BITCAST, t_uint, val, f == 1 ? fx : f == 2 ? fy : fz);
                } else {
                    int vj = f - 4;
                    uint32_t id = vj < g->v.n ? g->v.id[vj] : 0;
                    uint8_t kind = id ? lg[j].kind[vj] : K_NONE;
                    if (!id || kind == K_NONE) {
                        val = c_k[0];
                    } else if (kind == K_UINT) {
                        val = id;
                    } else if (kind == K_BOOL) {
                        val = next++;
                        INS(&o, OP_SELECT, t_uint, val, id, c_k[1], c_k[0]);
                    } else {
                        val = next++;
                        INS(&o, OP_BITCAST, t_uint, val, id);
                    }
                }
                uint32_t ixk = next++, pk = next++;
                INS(&o, OP_I_ADD, t_uint, ixk, rb, c_k[f]);
                INS(&o, OP_ACCESS_CHAIN, t_pu, pk, base, c_k[0], ixk);
                INS(&o, OP_STORE, pk, val, 2, 4);   // Aligned 4
            }
            INS(&o, OP_BRANCH, l_after2);
            INS(&o, OP_LABEL, l_after2);
            INS(&o, OP_BRANCH, g->after);
            INS(&o, OP_LABEL, g->after);
        }
        // the instruction itself, changed where needed
        if (k == mm) {
            INS(&o, OP_MEMORY_MODEL, ADDR_PSB64, src[2]);
            continue;
        }
        if (k == ep && n_fc) {
            put(&o, (len + 1) << 16 | op);
            for (uint32_t i = 1; i < len; i++) put(&o, src[i]);
            put(&o, n_fc);
            continue;
        }
        if ((op == OP_KILL || op == OP_TERMINATE_INVOCATION) && sh->nokill) {
            int j = 0;
            while (j < ng && G[j].kill != k) j++;
            if (j < ng) {
                INS(&o, OP_BRANCH, G[j].merge);
                continue;
            }
        }
        size_t at = (size_t)o.n;
        for (uint32_t i = 0; i < len; i++) put(&o, src[i]);
        if (op == OP_PHI)   // a guard block's successors now come from its second half
            for (uint32_t i = 4; i < len; i += 2)
                for (int j = 0; j < ng; j++)
                    if (o.w[at + i] == G[j].gb_label) o.w[at + i] = G[j].after;
    }
    o.w[bound_at] = next;
    // describe once per shader
    plat_mutex_lock(&D.lock);
    if (!sh->described) {
        sh->described = 1, sh->ng = ng;
        memcpy(sh->g, lg, sizeof lg);
        for (int j = 0; j < ng; j++) {
            const Guard *dg = &sh->g[j];
            rt_log("vk_shdbg: fs %016llx discard %d (%s when the condition is %s)%s:", (unsigned long long)sh->hash, j,
                   "kills", dg->inverted ? "false" : "true", sh->nokill ? ", disabled (nokill)" : "");
            for (int v = 0; v < dg->n; v++)
                rt_log("vk_shdbg:   v%d %s %s", v, v == 0 ? "a" : v == 1 ? "b" : "leaf", dg->what[v]);
        }
        char path[600];
        snprintf(path, sizeof path, "%s/fs_%016llx.spv", D.dir, (unsigned long long)sh->hash);
        FILE *f = fopen(path, "wb");
        if (f) fwrite(o.w, 4, (size_t)o.n, f), fclose(f);
    }
    plat_mutex_unlock(&D.lock);
    res = o.w, o.w = NULL;
    *out_n = o.n;
out:
    free(o.w);
    mod_free(&M);
    return res;
}

static uint64_t fnv(const uint32_t *w, uint64_t words) {
    uint64_t h = 0xcbf29ce484222325ull;
    const uint8_t *b = (const uint8_t *)w;
    for (uint64_t i = 0; i < words * 4; i++) h = (h ^ b[i]) * 0x100000001b3ull;
    return h;
}

uint32_t *vk_shdbg_fs(const uint32_t *w, uint64_t n, uint64_t *out_n) {
    plat_once(&parse_once, parse, NULL);
    if (!D.n || !w || n < 5) return NULL;
    uint64_t h = fnv(w, n);
    int si = -1;
    for (int i = 0; i < D.n; i++)
        if (D.sh[i].hash == h) si = i;
    if (si < 0) return NULL;
    plat_mutex_lock(&D.lock);
    int ok = buffer_ready();
    plat_mutex_unlock(&D.lock);
    return ok ? instrument(w, n, si, out_n) : NULL;
}

// ---- reading the records back

static void clear_records(void) {
    D.map[0] = 0;
    vmaFlushAllocation(vk_ctx()->vma, D.alloc, 0, 64);
}

static double as_num(uint32_t u, uint8_t kind) {
    float f;
    switch (kind) {
    case K_FLOAT: memcpy(&f, &u, 4); return f;
    case K_INT: return (double)(int32_t)u;
    default: return (double)u;
    }
}

static void write_frame(uint64_t frame) {
    vmaInvalidateAllocation(vk_ctx()->vma, D.alloc, 0, VK_WHOLE_SIZE);
    uint32_t asked = D.map[0], n = asked < D.cap ? asked : D.cap;
    char path[600];
    snprintf(path, sizeof path, "%s/frame_%llu.txt", D.dir, (unsigned long long)frame);
    FILE *f = fopen(path, "w");
    if (!f) { rt_log("vk_shdbg: cannot write %s", path); return; }
    fprintf(f, "frame %llu (world clock %.3f s): %u records of %u fragments (cap %u); window %u,%u..%u,%u every %u px\n",
            (unsigned long long)frame, frame_step_world_us() / 1e6, n, asked, D.cap, D.x0, D.y0, D.x1, D.y1, D.step);
    const uint32_t *rec = D.map + HDR_WORDS;
    for (int s = 0; s < D.n; s++) {
        const Sh *sh = &D.sh[s];
        fprintf(f, "\nshader s%d fs %016llx%s\n", s, (unsigned long long)sh->hash, sh->nokill ? " (nokill)" : "");
        for (int g = 0; g < sh->ng; g++) {
            const Guard *dg = &sh->g[g];
            uint64_t cnt = 0, killed = 0, nan[NV] = {0};
            double mn[NV], mx[NV];
            for (int v = 0; v < NV; v++) mn[v] = INFINITY, mx[v] = -INFINITY;
            for (uint32_t r = 0; r < n; r++) {
                const uint32_t *q = rec + (uint64_t)r * REC_WORDS;
                if ((q[0] & 0xff) != (uint32_t)(s | g << 4)) continue;
                cnt++;
                killed += ((q[0] >> 8 & 1) ^ (uint32_t)dg->inverted) != 0;
                for (int v = 0; v < dg->n; v++) {
                    if (dg->kind[v] == K_NONE) continue;
                    double x = as_num(q[4 + v], dg->kind[v]);
                    if (isnan(x)) { nan[v]++; continue; }
                    if (x < mn[v]) mn[v] = x;
                    if (x > mx[v]) mx[v] = x;
                }
            }
            fprintf(f, "  discard %d: %llu fragments, %llu discarded by the condition\n", g, (unsigned long long)cnt,
                    (unsigned long long)killed);
            for (int v = 0; v < dg->n; v++) {
                if (dg->kind[v] == K_NONE) fprintf(f, "    v%d %s\n", v, dg->what[v]);
                else fprintf(f, "    v%d min %.9g max %.9g nan %llu   %s\n", v, cnt ? mn[v] : 0, cnt ? mx[v] : 0,
                             (unsigned long long)nan[v], dg->what[v]);
            }
        }
    }
    fprintf(f, "\nrecords: s<shader> g<discard> k<1: the condition discards> x y depth | v0 v1 ...\n");
    for (uint32_t r = 0; r < n; r++) {
        const uint32_t *q = rec + (uint64_t)r * REC_WORDS;
        int s = (int)(q[0] & 15), g = (int)(q[0] >> 4 & 15);
        if (s >= D.n || g >= MAX_GUARDS) continue;
        const Guard *dg = &D.sh[s].g[g];
        float x, y, z;
        memcpy(&x, &q[1], 4), memcpy(&y, &q[2], 4), memcpy(&z, &q[3], 4);
        fprintf(f, "s%d g%d k%u %.1f %.1f %.9g |", s, g, (q[0] >> 8 & 1) ^ (uint32_t)dg->inverted, (double)x, (double)y,
                (double)z);
        for (int v = 0; v < dg->n; v++)
            if (dg->kind[v] == K_NONE) fprintf(f, " -");
            else fprintf(f, " %.9g", as_num(q[4 + v], dg->kind[v]));
        fprintf(f, "\n");
    }
    fclose(f);
    rt_log("vk_shdbg: present %llu: %u records (%u fragments) -> %s", (unsigned long long)frame, n, asked, path);
}

void vk_shdbg_present(uint64_t frame) {
    if (!__atomic_load_n(&D.addr, __ATOMIC_ACQUIRE) || D.state == 2) return;
    if (D.state == 0) {
        int go = D.at_world ? frame_step_world_us() >= (int64_t)D.at : frame >= D.at;
        if (!go) return;
        vk_sync_finish();
        clear_records();
        D.state = 1, D.left = D.frames;
        rt_log("vk_shdbg: recording from present %llu", (unsigned long long)frame);
        return;
    }
    vk_sync_finish();
    write_frame(frame);
    clear_records();
    if (!--D.left) D.state = 2;
}
