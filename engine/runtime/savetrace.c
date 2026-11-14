// Save-data trace (debug tool), TOTK_SAVE_TRACE=<dir>. Splatoon 3 encrypts its save.dat itself, through the nn::crypto
// functions main imports from the SDK. Observers on those imports (JUMP_SLOT, so only main's calls are seen, not the
// SDK's calls inside itself) and hooks in fs_host.c's file calls log every call with its arguments, its thread and its
// guest call stack, and dump the buffers it reads and writes, so the format can be read from what the game does
// instead of from its code. Without the variable every observer only forwards to the SDK's definition.
//
// <dir>/trace.log: one line per call, "<seq> <ms> t<thread> <function> <arguments> | <call stack>". A buffer of up to
// 64 bytes is written inline as hex; a longer one goes to <dir>/<seq>-<function>-<role>.bin and the line names the file
// with its first bytes. The dumps hold key material and the player's save in clear: the folder belongs to the
// player's own user data, never to the repository.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cfg.h"
#include "fs.h"
#include "host.h"
#include "savetrace.h"
#include "platform/platform.h"

#define INLINE_MAX 64
#define LOG_CAP (512ull << 20)   // the log stops growing past this (a run that never ends)
#define MAX_FILES 64

static int state = -1;   // -1 not read yet, 0 off, 1 on
static char dir[1024];
static FILE *lg;
static PlatMutex mu = PLAT_MUTEX_INIT;
static uint64_t seq, logged, t0;
static PlatOnce once = PLAT_ONCE_INIT;

static void st_init(void *arg) {
    (void)arg;
    const char *e = cfg_get("TOTK_SAVE_TRACE");
    if (!e || !*e || !strcmp(e, "0")) {
        __atomic_store_n(&state, 0, __ATOMIC_RELEASE);
        return;
    }
    snprintf(dir, sizeof dir, "%s", strcmp(e, "1") ? e : "savetrace");
    plat_fs_mkdir(dir);
    char p[1100];
    snprintf(p, sizeof p, "%s/trace.log", dir);
    lg = fopen(p, "ab");
    if (!lg) {
        rt_log("savetrace: cannot write %s; trace off", p);
        __atomic_store_n(&state, 0, __ATOMIC_RELEASE);
        return;
    }
    t0 = plat_now_ns();
    rt_log("savetrace: on, log %s", p);
    fprintf(lg, "# savetrace start\n");
    fflush(lg);
    __atomic_store_n(&state, 1, __ATOMIC_RELEASE);
}

int savetrace_on(void) {
    int s = __atomic_load_n(&state, __ATOMIC_ACQUIRE);
    if (s >= 0) return s;
    plat_once(&once, st_init, NULL);
    return __atomic_load_n(&state, __ATOMIC_ACQUIRE);
}

// ---- a record: built in a buffer, written under the lock in one piece
typedef struct { char b[8192]; size_t n; uint64_t seq; } Rec;

static void rec_add(Rec *r, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static void rec_add(Rec *r, const char *fmt, ...) {
    if (r->n >= sizeof r->b - 1) return;
    va_list ap;
    va_start(ap, fmt);
    int k = vsnprintf(r->b + r->n, sizeof r->b - r->n, fmt, ap);
    va_end(ap);
    if (k > 0) r->n += (size_t)k < sizeof r->b - r->n ? (size_t)k : sizeof r->b - 1 - r->n;
}

static void rec_begin(Rec *r, const char *fn) {
    r->n = 0;
    r->seq = __atomic_fetch_add(&seq, 1, __ATOMIC_RELAXED);
    rec_add(r, "%06llu %9.3f t%u %s", (unsigned long long)r->seq, (double)(plat_now_ns() - t0) / 1e6, plat_thread_id(),
            fn);
}

static int readable(uint64_t a, uint64_t n) { return a && n && n < (1ull << 31) && mem_guest_readable(a, n); }

// A guest buffer: inline hex up to INLINE_MAX bytes, else a file and its first 16 bytes.
static void rec_buf(Rec *r, const char *role, uint64_t a, uint64_t n) {
    if (!n) {
        rec_add(r, " %s=<0>", role);
        return;
    }
    if (!readable(a, n)) {
        rec_add(r, " %s=<%#llx unreadable %llu>", role, (unsigned long long)a, (unsigned long long)n);
        return;
    }
    const uint8_t *p = (const uint8_t *)(uintptr_t)a;
    if (n <= INLINE_MAX) {
        rec_add(r, " %s=", role);
        for (uint64_t i = 0; i < n; i++) rec_add(r, "%02x", p[i]);
        return;
    }
    char name[1200];
    snprintf(name, sizeof name, "%s/%06llu-%s.bin", dir, (unsigned long long)r->seq, role);
    FILE *f = fopen(name, "wb");
    if (f) {
        fwrite(p, 1, (size_t)n, f);
        fclose(f);
    }
    rec_add(r, " %s=[%06llu-%s.bin %llu:", role, (unsigned long long)r->seq, role, (unsigned long long)n);
    for (uint64_t i = 0; i < 16; i++) rec_add(r, "%02x", p[i]);
    rec_add(r, "..]");
}

// The guest call stack: the return address, then the frame records from x29 (AAPCS64: [fp] the caller's fp, [fp+8]
// its return address).
static void rec_stack(Rec *r, const Cpu *c, int depth) {
    char d[96];
    rec_add(r, " | %s", ldr_describe(c->x[30] - 4, d, sizeof d));
    uint64_t fp = c->x[29];
    for (int i = 0; i < depth && readable(fp, 16) && !(fp & 7); i++) {
        uint64_t next = LD64(fp), lr = LD64(fp + 8);
        if (!lr) break;
        rec_add(r, " < %s", ldr_describe(lr - 4, d, sizeof d));
        if (next <= fp) break;
        fp = next;
    }
}

static void rec_end(Rec *r) {
    rec_add(r, "\n");
    plat_mutex_lock(&mu);
    if (logged < LOG_CAP) {
        fwrite(r->b, 1, r->n, lg);
        fflush(lg);
        logged += r->n;
    }
    plat_mutex_unlock(&mu);
}

// ---- nn::crypto observers
#define SYM_DEC_CBC "_ZN2nn6crypto16DecryptAes128CbcEPvmPKvmS3_mS3_m"
#define SYM_AES16_INIT "_ZN2nn6crypto6detail7AesImplILm16EE10InitializeEPKvmb"
#define SYM_AES16_ENC "_ZNK2nn6crypto6detail7AesImplILm16EE12EncryptBlockEPvmPKvm"
#define SYM_AES32_INIT "_ZN2nn6crypto6detail7AesImplILm32EE10InitializeEPKvmb"
#define SYM_AES32_ENC "_ZNK2nn6crypto6detail7AesImplILm32EE12EncryptBlockEPvmPKvm"
#define SYM_MAC_UPDATE "_ZN2nn6crypto6detail10CbcMacImpl6UpdateINS0_12AesEncryptorILm16EEEEEvPKvm"
#define SYM_MAC_MASK "_ZN2nn6crypto6detail10CbcMacImpl16MaskBufferedDataEPKvm"
#define SYM_MAC_GET "_ZN2nn6crypto6detail10CbcMacImpl6GetMacEPvm"
#define SYM_CBC_DEC "_ZN2nn6crypto6detail11CbcModeImplINS0_12AesDecryptorILm16EEEE13DecryptBlocksEPhS6_PKhiPKS4_"
#define SYM_SHA256_INIT "_ZN2nn6crypto6detail10Sha256Impl10InitializeEv"
#define SYM_SHA256_UPDATE "_ZN2nn6crypto6detail10Sha256Impl6UpdateEPKvm"
#define SYM_SHA256_GET "_ZN2nn6crypto6detail10Sha256Impl7GetHashEPvm"
#define SYM_SHA1_INIT "_ZN2nn6crypto6detail8Sha1Impl10InitializeEv"
#define SYM_SHA1_UPDATE "_ZN2nn6crypto6detail8Sha1Impl6UpdateEPKvm"
#define SYM_SHA1_GET "_ZN2nn6crypto6detail8Sha1Impl7GetHashEPvm"
#define SYM_RANDOM "_ZN2nn6crypto36GenerateCryptographicallyRandomBytesEPvm"

#define STACK_DEPTH 16

// size_t DecryptAes128Cbc(void *dst, size_t dst_size, const void *key, size_t key_size, const void *iv,
//                         size_t iv_size, const void *src, size_t src_size)
static void obs_dec_cbc(Cpu *c) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    uint64_t dst = c->x[0], dsz = c->x[1], src = c->x[6], ssz = c->x[7];
    Rec r;
    rec_begin(&r, "DecryptAes128Cbc");
    rec_add(&r, " dst=%#llx dst_size=%llu", (unsigned long long)dst, (unsigned long long)dsz);
    rec_buf(&r, "key", c->x[2], c->x[3]);
    rec_buf(&r, "iv", c->x[4], c->x[5]);
    rec_buf(&r, "src", src, ssz);
    rec_stack(&r, c, STACK_DEPTH);
    uint64_t ret = rt_original(c);
    rec_add(&r, " -> %llu", (unsigned long long)ret);
    rec_buf(&r, "out", dst, ret < dsz ? ret : dsz);
    rec_end(&r);
}

// void AesImpl<N>::Initialize(const void *key, size_t key_size, bool is_encrypt)
static void aes_init(Cpu *c, const char *fn) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    Rec r;
    rec_begin(&r, fn);
    rec_add(&r, " this=%#llx encrypt=%d", (unsigned long long)c->x[0], (int)(c->x[3] & 1));
    rec_buf(&r, "key", c->x[1], c->x[2]);
    rec_stack(&r, c, STACK_DEPTH);
    rt_original(c);
    rec_end(&r);
}
static void obs_aes16_init(Cpu *c) { aes_init(c, "Aes128.Initialize"); }
static void obs_aes32_init(Cpu *c) { aes_init(c, "Aes256.Initialize"); }

// void AesImpl<N>::EncryptBlock(void *dst, size_t dst_size, const void *src, size_t src_size) const
static void aes_enc(Cpu *c, const char *fn) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    uint64_t dst = c->x[1], dsz = c->x[2], src = c->x[3], n = c->x[4];
    Rec r;
    rec_begin(&r, fn);
    rec_add(&r, " this=%#llx", (unsigned long long)c->x[0]);
    rec_buf(&r, "in", src, n);
    rec_stack(&r, c, 2);
    rt_original(c);
    rec_buf(&r, "out", dst, dsz < n ? dsz : n);
    rec_end(&r);
}
static void obs_aes16_enc(Cpu *c) { aes_enc(c, "Aes128.EncryptBlock"); }
static void obs_aes32_enc(Cpu *c) { aes_enc(c, "Aes256.EncryptBlock"); }

// CbcMacImpl::Update<AesEncryptor<16>>(const void *data, size_t size), MaskBufferedData(const void *subkey, size_t
// size), GetMac(void *mac, size_t size)
static void mac_in(Cpu *c, const char *fn) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    Rec r;
    rec_begin(&r, fn);
    rec_add(&r, " this=%#llx", (unsigned long long)c->x[0]);
    rec_buf(&r, "data", c->x[1], c->x[2]);
    rec_stack(&r, c, STACK_DEPTH);
    rt_original(c);
    rec_end(&r);
}
static void obs_mac_update(Cpu *c) { mac_in(c, "CbcMac.Update"); }
static void obs_mac_mask(Cpu *c) { mac_in(c, "CbcMac.MaskBufferedData"); }
static void obs_sha256_update(Cpu *c) { mac_in(c, "Sha256.Update"); }
static void obs_sha1_update(Cpu *c) { mac_in(c, "Sha1.Update"); }

static void hash_out(Cpu *c, const char *fn) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    uint64_t out = c->x[1], n = c->x[2];
    Rec r;
    rec_begin(&r, fn);
    rec_add(&r, " this=%#llx", (unsigned long long)c->x[0]);
    rec_stack(&r, c, STACK_DEPTH);
    rt_original(c);
    rec_buf(&r, "out", out, n);
    rec_end(&r);
}
static void obs_mac_get(Cpu *c) { hash_out(c, "CbcMac.GetMac"); }
static void obs_sha256_get(Cpu *c) { hash_out(c, "Sha256.GetHash"); }
static void obs_sha1_get(Cpu *c) { hash_out(c, "Sha1.GetHash"); }

static void hash_init(Cpu *c, const char *fn) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    Rec r;
    rec_begin(&r, fn);
    rec_add(&r, " this=%#llx", (unsigned long long)c->x[0]);
    rec_stack(&r, c, STACK_DEPTH);
    rt_original(c);
    rec_end(&r);
}
static void obs_sha256_init(Cpu *c) { hash_init(c, "Sha256.Initialize"); }
static void obs_sha1_init(Cpu *c) { hash_init(c, "Sha1.Initialize"); }

// CbcModeImpl<AesDecryptor<16>>::DecryptBlocks(u8 *, u8 *, const u8 *, int, const AesDecryptor<16> *): whether it
// takes a this pointer is not known, so the six argument registers are logged and the block count is the first of
// x3 / x4 that looks like one; the buffers around it are dumped with that count.
static void obs_cbc_dec(Cpu *c) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    Rec r;
    rec_begin(&r, "CbcDec.DecryptBlocks");
    for (int i = 0; i < 6; i++) rec_add(&r, " x%d=%#llx", i, (unsigned long long)c->x[i]);
    int k = c->x[3] < 0x100000 ? 3 : c->x[4] < 0x100000 ? 4 : -1;
    uint64_t blocks = k > 0 ? (uint32_t)c->x[k] : 0, dst = k > 0 ? c->x[k - 3] : 0, iv = k > 0 ? c->x[k - 2] : 0,
             src = k > 0 ? c->x[k - 1] : 0;
    if (k > 0) {
        rec_buf(&r, "iv", iv, 16);
        rec_buf(&r, "src", src, blocks * 16);
    }
    rec_stack(&r, c, STACK_DEPTH);
    rt_original(c);
    if (k > 0) {
        rec_buf(&r, "out", dst, blocks * 16);
        rec_buf(&r, "iv_after", iv, 16);
    }
    rec_end(&r);
}

// void GenerateCryptographicallyRandomBytes(void *buf, size_t size)
static void obs_random(Cpu *c) {
    if (!savetrace_on()) {
        rt_original(c);
        return;
    }
    uint64_t buf = c->x[0], n = c->x[1];
    Rec r;
    rec_begin(&r, "RandomBytes");
    rec_stack(&r, c, STACK_DEPTH);
    rt_original(c);
    rec_buf(&r, "out", buf, n);
    rec_end(&r);
}

HOST_OBSERVER(SYM_DEC_CBC, obs_dec_cbc);
HOST_OBSERVER(SYM_AES16_INIT, obs_aes16_init);
HOST_OBSERVER(SYM_AES16_ENC, obs_aes16_enc);
HOST_OBSERVER(SYM_AES32_INIT, obs_aes32_init);
HOST_OBSERVER(SYM_AES32_ENC, obs_aes32_enc);
HOST_OBSERVER(SYM_MAC_UPDATE, obs_mac_update);
HOST_OBSERVER(SYM_MAC_MASK, obs_mac_mask);
HOST_OBSERVER(SYM_MAC_GET, obs_mac_get);
HOST_OBSERVER(SYM_CBC_DEC, obs_cbc_dec);
HOST_OBSERVER(SYM_SHA256_INIT, obs_sha256_init);
HOST_OBSERVER(SYM_SHA256_UPDATE, obs_sha256_update);
HOST_OBSERVER(SYM_SHA256_GET, obs_sha256_get);
HOST_OBSERVER(SYM_SHA1_INIT, obs_sha1_init);
HOST_OBSERVER(SYM_SHA1_UPDATE, obs_sha1_update);
HOST_OBSERVER(SYM_SHA1_GET, obs_sha1_get);
HOST_OBSERVER(SYM_RANDOM, obs_random);

// ---- file calls (fs_host.c): the host files the game opens, by handle
static struct { uint64_t h; char path[256]; } files[MAX_FILES];

static const char *file_path(uint64_t h) {
    for (int i = 0; i < MAX_FILES; i++)
        if (files[i].h == h) return files[i].path;
    return "?";
}

static void rec_guest_stack(Rec *r) {
    Cpu *c = kern_cpu();
    if (c) rec_stack(r, c, STACK_DEPTH);
}

void savetrace_fs_open(const char *path, uint64_t h, int mode, uint32_t result) {
    if (!savetrace_on()) return;
    if (!result && !romfs_handle(h)) {   // RomFS files are logged at open only
        plat_mutex_lock(&mu);
        for (int i = 0; i < MAX_FILES; i++)
            if (!files[i].h || files[i].h == h) {
                files[i].h = h;
                snprintf(files[i].path, sizeof files[i].path, "%s", path ? path : "(null)");
                break;
            }
        plat_mutex_unlock(&mu);
    }
    Rec r;
    rec_begin(&r, "fs.OpenFile");
    rec_add(&r, " %s mode=%d h=%#llx result=%#x", path ? path : "(null)", mode, (unsigned long long)(result ? 0 : h),
            result);
    rec_guest_stack(&r);
    rec_end(&r);
}

void savetrace_fs_io(uint64_t h, int write, int64_t offset, const void *buf, uint64_t size) {
    if (!savetrace_on()) return;
    Rec r;
    rec_begin(&r, write ? "fs.WriteFile" : "fs.ReadFile");
    rec_add(&r, " %s off=%lld size=%llu", file_path(h), (long long)offset, (unsigned long long)size);
    rec_buf(&r, write ? "data" : "got", (uint64_t)(uintptr_t)buf, size);
    rec_guest_stack(&r);
    rec_end(&r);
}

void savetrace_fs_close(uint64_t h) {
    if (!savetrace_on()) return;
    Rec r;
    rec_begin(&r, "fs.CloseFile");
    plat_mutex_lock(&mu);
    rec_add(&r, " %s", file_path(h));
    for (int i = 0; i < MAX_FILES; i++)
        if (files[i].h == h) files[i].h = 0;
    plat_mutex_unlock(&mu);
    rec_end(&r);
}

void savetrace_fs_op(const char *op, const char *path, int64_t arg, uint32_t result) {
    if (!savetrace_on()) return;
    Rec r;
    rec_begin(&r, op);
    rec_add(&r, " %s arg=%lld result=%#x", path ? path : "(null)", (long long)arg, result);
    rec_guest_stack(&r);
    rec_end(&r);
}
