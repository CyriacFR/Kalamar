// The console offline: the network APIs a game imports answer as a Switch with no network connection, local wireless
// off and no linked Nintendo Account would, so the game takes its offline paths. Splatoon 3 11.2.0's main imports
// every name here (nn::nifm, nn::socket and its C shims, nn::ssl and nnssl*, nn::ldn, nn::friends, nn::bcat, nn::nsd,
// nn::ngc, nn::ec, nn::pctl, nn::account's network service account, nn::web, nn::album, nn::crypto's random source);
// The publisher's online libraries, gRPC, libcurl and the session library are linked into main and reach the network only through these.
//
// The host has no network code at all: nothing in the runtime opens a socket, resolves a name or speaks TLS or HTTP,
// and a service request no override answers stops with a named fatal in ipc.c (sm: serves audren:u and nfp:user
// only). So this file decides how the game fails, never whether it can connect:
//   nifm      no network: IsNetworkAvailable false, requests end unavailable, no IP configuration
//   socket    Initialize and Finalize succeed (they do offline too); creating a socket or an eventfd fails with
//             ENETDOWN, every call on a descriptor with EBADF, every name lookup with EAI_AGAIN; byte-order and
//             address-text helpers compute their result
//   ssl       initialisation and contexts succeed with local ids; built-in certificates and every connection fail
//   ldn       local wireless unavailable (Initialize fails, state None)
//   friends, bcat, ec, web, album, account's network service account, time's network clock: unavailable
//   nsd       the production environment ("lp1"); ResolveEx copies the name (no lookup happens)
//   ngc       the profanity filter masks nothing (its word list is system data this host does not have)
//   pctl      no parental controls: free communication available
// Play reports (nn::prepo) are dropped in ov_system.c. Each kind of call logs its first use ("net: ...").
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "host.h"
#include "platform/platform.h"

// Failure results in the module concerned (module | description << 9). The descriptions are this port's choice, not
// read from the SDK: what matters is that the game sees a failure of that module.
#define R_NIFM_UNAVAILABLE RT_RESULT(110, 300)      // nifm: the request found no network
#define R_SSL_UNAVAILABLE RT_RESULT(123, 300)       // ssl: no connection can be made
#define R_LDN_UNAVAILABLE RT_RESULT(203, 32)        // ldn: local wireless is off
#define R_FRIENDS_UNAVAILABLE RT_RESULT(121, 300)   // friends: no network service account
#define R_BCAT_UNAVAILABLE RT_RESULT(122, 1)        // bcat: no delivery cache
#define R_ACCOUNT_NSA_UNAVAILABLE RT_RESULT(124, 300)   // account: no network service account
#define R_WEB_UNAVAILABLE RT_RESULT(166, 1)         // web applet: unavailable
#define R_ALBUM_UNAVAILABLE RT_RESULT(206, 1)       // capsrv: no album on this host

// nn::socket's errno numbers, Linux's (runtime/lan.h, the LE_* values)
enum { E_BADF = 9, E_INVAL = 22, E_AFNOSUPPORT = 97, E_NETDOWN = 100 };
enum { EAI_AGAIN_ = 2 };
enum { AF_INET_ = 2 };

static void note(volatile int32_t *seen, const char *what, Cpu *c) {
    if (plat_xchg32(seen, 1)) return;
    char lr[96];
    rt_log("net: %s (offline) from %s", what, ldr_describe(c->x[30] - 4, lr, sizeof lr));
}
#define NOTE(c, what) do { static volatile int32_t seen_; note(&seen_, what, c); } while (0)

// ==== nn::nifm
static void ov_nifm_initialize(Cpu *c) { NOTE(c, "nifm::Initialize"); ret_x(c, 0); }
static void ov_nifm_available(Cpu *c) { NOTE(c, "nifm::IsNetworkAvailable -> false"); ret_x(c, 0); }
static void ov_nifm_on_hold(Cpu *c) { ret_x(c, 0); }
static void ov_nifm_void(Cpu *c) { NOTE(c, "nifm request (submit, cancel or local mode): no network"); }
// Result GetCurrentIpConfigInfo(in_addr *ip, *mask, *gateway, *dns1, *dns2), GetCurrentProxySetting(ProxySetting *)
static void ov_nifm_unavailable(Cpu *c) { NOTE(c, "nifm configuration query -> unavailable"); ret_x(c, R_NIFM_UNAVAILABLE); }
// Result HandleNetworkRequestErrorResult(): the system's error dialog for the failed request; it stays failed
static void ov_nifm_handle_error(Cpu *c) {
    NOTE(c, "nifm::HandleNetworkRequestErrorResult -> unavailable");
    ret_x(c, R_NIFM_UNAVAILABLE);
}

// ==== nn::socket
static _Thread_local int32_t sock_errno;
static void sock_fail(Cpu *c, int32_t e) {
    sock_errno = e;
    ret_x(c, (uint64_t)(int64_t)-1);
}
static void ov_sock_initialize(Cpu *c) { NOTE(c, "socket::Initialize"); ret_x(c, 0); }
static void ov_sock_finalize(Cpu *c) { ret_x(c, 0); }
static void ov_sock_create(Cpu *c) { NOTE(c, "socket creation -> ENETDOWN"); sock_fail(c, E_NETDOWN); }
static void ov_sock_bad_fd(Cpu *c) { NOTE(c, "socket call on a descriptor -> EBADF"); sock_fail(c, E_BADF); }
static void ov_sock_errno(Cpu *c) { ret_x(c, (uint64_t)(int64_t)sock_errno); }
static void ov_sock_set_errno(Cpu *c) { sock_errno = (int32_t)arg_x(c, 0); }
// AiErrno GetAddrInfo(const char *node, const char *service, const addrinfo *hints, addrinfo **out)
static void ov_sock_getaddrinfo(Cpu *c) {
    uint64_t *out = arg_p(c, 3);
    if (out) *out = 0;
    NOTE(c, "socket name lookup -> EAI_AGAIN");
    ret_x(c, EAI_AGAIN_);
}
// hostent *GetHostByNameWithOptions(const char *name, ...)
static void ov_sock_gethostbyname(Cpu *c) { NOTE(c, "socket name lookup -> none"); ret_x(c, 0); }
static void ov_sock_freeaddrinfo(Cpu *c) { (void)c; }   // nothing is ever handed out
// const char *GAIStrError(AiErrno): one text, in guest-visible memory
static void ov_sock_gai_strerror(Cpu *c) {
    static char *text;
    if (!text) {
        static const char msg[] = "Temporary failure in name resolution (no network)";
        char *p = mem_kernel_alloc(sizeof msg, 16);
        memcpy(p, msg, sizeof msg);
        text = p;
    }
    ret_x(c, (uint64_t)(uintptr_t)text);
}
// int ResolverSetOption(...), ResolverGetOption(...): the resolver never runs
static void ov_sock_resolver_set(Cpu *c) { ret_x(c, 0); }
static void ov_sock_resolver_get(Cpu *c) { sock_fail(c, E_INVAL); }
static void ov_htonl(Cpu *c) { ret_x(c, __builtin_bswap32((uint32_t)arg_x(c, 0))); }
static void ov_htons(Cpu *c) { ret_x(c, __builtin_bswap16((uint16_t)arg_x(c, 0))); }
// int InetPton(int af, const char *text, void *out): IPv4 dotted quads
static void ov_inet_pton(Cpu *c) {
    int af = (int)arg_x(c, 0);
    const char *s = arg_p(c, 1);
    uint8_t *out = arg_p(c, 2);
    if (af != AF_INET_) { sock_fail(c, E_AFNOSUPPORT); return; }
    unsigned v[4];
    char tail;
    if (!s || !out || sscanf(s, "%u.%u.%u.%u%c", &v[0], &v[1], &v[2], &v[3], &tail) != 4 || v[0] > 255 || v[1] > 255 ||
        v[2] > 255 || v[3] > 255) {
        ret_x(c, 0);
        return;
    }
    for (int i = 0; i < 4; i++) out[i] = (uint8_t)v[i];
    ret_x(c, 1);
}
// const char *InetNtop(int af, const void *addr, char *out, socklen_t size): IPv4
static void ov_inet_ntop(Cpu *c) {
    int af = (int)arg_x(c, 0);
    const uint8_t *a = arg_p(c, 1);
    char *out = arg_p(c, 2);
    uint32_t n = (uint32_t)arg_x(c, 3);
    char buf[16];
    if (af != AF_INET_ || !a || !out) { sock_fail(c, af != AF_INET_ ? E_AFNOSUPPORT : E_INVAL); ret_x(c, 0); return; }
    int k = snprintf(buf, sizeof buf, "%u.%u.%u.%u", a[0], a[1], a[2], a[3]);
    if ((uint32_t)k + 1 > n) { sock_fail(c, E_INVAL); ret_x(c, 0); return; }
    memcpy(out, buf, (size_t)k + 1);
    ret_x(c, (uint64_t)(uintptr_t)out);
}

// ==== nn::ssl and the nnssl* C functions
static volatile int64_t ssl_ids;
static void ov_ssl_ok(Cpu *c) { NOTE(c, "ssl initialisation"); ret_x(c, 0); }
// Result Context::Create(SslVersion), ImportServerPki(uint64_t *id, ...), ImportClientPki(uint64_t *id, ...),
// ImportCrl(uint64_t *id, ...): local objects, given ids
static void ov_ssl_context_create(Cpu *c) {
    uint64_t *self = arg_p(c, 0);
    if (self) *self = (uint64_t)plat_inc64(&ssl_ids);
    NOTE(c, "ssl::Context::Create");
    ret_x(c, 0);
}
static void ov_ssl_import(Cpu *c) {
    uint64_t *id = arg_p(c, 1);
    if (id) *id = (uint64_t)plat_inc64(&ssl_ids);
    ret_x(c, 0);
}
static void ov_ssl_unavailable(Cpu *c) { NOTE(c, "ssl connection or certificate store -> unavailable"); ret_x(c, R_SSL_UNAVAILABLE); }
// nnResult nnsslContextGetContextId(const nnsslContext *, uint64_t *id)
static void ov_ssl_context_id(Cpu *c) {
    const uint64_t *ctx = arg_p(c, 0);
    uint64_t *id = arg_p(c, 1);
    if (id) *id = ctx ? *ctx : 0;
    ret_x(c, 0);
}

// ==== nn::ldn: local wireless off
static void ov_ldn_unavailable(Cpu *c) { NOTE(c, "ldn (local wireless) -> unavailable"); ret_x(c, R_LDN_UNAVAILABLE); }
static void ov_ldn_state(Cpu *c) { ret_x(c, 0); }          // State GetState(): None
static void ov_ldn_void(Cpu *c) { (void)c; }               // Finalize, CloseStation, ... after a failed Initialize
static void ov_ldn_reason(Cpu *c) { ret_x(c, 0); }         // DisconnectReason GetDisconnectReason(): None
// Ipv4Address MakeBroadcastAddress(Ipv4Address, SubnetMask)
static void ov_ldn_broadcast(Cpu *c) { ret_x(c, (uint32_t)arg_x(c, 0) | ~(uint32_t)arg_x(c, 1)); }
// void AttachStateChangeEvent(os::SystemEventType *): bound, never signalled
static void ov_ldn_state_event(Cpu *c) {
    uint64_t ev = arg_x(c, 0);
    if (ev) kern_bind_system_event(c, ev, 0);
}

// ==== nn::friends
static void ov_friends_unavailable(Cpu *c) { NOTE(c, "friends -> unavailable"); ret_x(c, R_FRIENDS_UNAVAILABLE); }
// Result GetFriendList(int *count, NetworkServiceAccountId *, int offset, int max, const FriendFilter &): none cached
static void ov_friends_list(Cpu *c) {
    int32_t *count = arg_p(c, 0);
    if (count) *count = 0;
    NOTE(c, "friends::GetFriendList -> empty");
    ret_x(c, 0);
}
// Result AsyncContext::HasDone(bool *): the friends calls above never start one
static void ov_friends_done(Cpu *c) {
    uint8_t *out = arg_p(c, 1);
    if (out) *out = 1;
    ret_x(c, 0);
}
static void ov_friends_result(Cpu *c) { ret_x(c, R_FRIENDS_UNAVAILABLE); }   // AsyncContext::GetResult() const
static void ov_friends_ok(Cpu *c) { ret_x(c, 0); }   // AddPlayHistory: kept nowhere

// ==== nn::bcat: no delivery cache
static void ov_bcat_initialize(Cpu *c) { NOTE(c, "bcat::Initialize"); ret_x(c, 0); }
static void ov_bcat_unavailable(Cpu *c) { NOTE(c, "bcat delivery cache -> unavailable"); ret_x(c, R_BCAT_UNAVAILABLE); }
static void ov_bcat_void(Cpu *c) { (void)c; }
static void ov_bcat_zero(Cpu *c) { ret_x(c, 0); }   // DeliveryCacheFile::GetSize, Progress::GetWholeDownloaded
static void ov_bcat_status(Cpu *c) { ret_x(c, 9); }   // DeliveryCacheProgress::GetStatus(): Done
static void ov_bcat_result(Cpu *c) { ret_x(c, R_BCAT_UNAVAILABLE); }

// ==== nn::nsd
// Result GetEnvironmentIdentifier(EnvironmentIdentifier *): char[8]
static void ov_nsd_environment(Cpu *c) {
    char *out = arg_p(c, 0);
    if (out) {
        memset(out, 0, 8);
        memcpy(out, "lp1", 3);
    }
    ret_x(c, 0);
}
// Result ResolveEx(Fqdn *out, const Fqdn &in): char[256]
static void ov_nsd_resolve(Cpu *c) {
    char *out = arg_p(c, 0);
    const char *in = arg_p(c, 1);
    if (out && in) memmove(out, in, 256), out[255] = 0;
    NOTE(c, "nsd::ResolveEx (name kept, nothing is looked up)");
    ret_x(c, 0);
}

// ==== nn::ngc::ProfanityFilter
static void ov_ngc_ok(Cpu *c) { NOTE(c, "ngc::ProfanityFilter (masks nothing)"); ret_x(c, 0); }
static void ov_ngc_void(Cpu *c) { (void)c; }
// Result MaskProfanityWordsInText(int *count, char *text, uint32_t languages)
static void ov_ngc_mask(Cpu *c) {
    int32_t *count = arg_p(c, 0);
    if (count) *count = 0;
    ret_x(c, 0);
}

// ==== nn::ec, nn::web, nn::album
static void ov_ec_shop(Cpu *c) { NOTE(c, "ec::ShowShopAddOnContentList: no shop"); }
static void ov_web_unavailable(Cpu *c) { NOTE(c, "web::ShowLobbyPage -> unavailable"); ret_x(c, R_WEB_UNAVAILABLE); }
static void ov_album_initialize(Cpu *c) { ret_x(c, 0); }
static void ov_album_unavailable(Cpu *c) { NOTE(c, "album -> unavailable"); ret_x(c, R_ALBUM_UNAVAILABLE); }
static void ov_album_void(Cpu *c) { (void)c; }

// ==== nn::pctl: no restrictions
static void ov_pctl_true(Cpu *c) { ret_x(c, 1); }   // IsFreeCommunicationAvailable, TryBeginFreeCommunication
static void ov_pctl_void(Cpu *c) { (void)c; }

// ==== nn::account: the local user has no network service account
// Result IsNetworkServiceAccountAvailable(bool *, const UserHandle &)
static void ov_nsa_available(Cpu *c) {
    uint8_t *out = arg_p(c, 0);
    if (out) *out = 0;
    NOTE(c, "account::IsNetworkServiceAccountAvailable -> false");
    ret_x(c, 0);
}
static void ov_nsa_unavailable(Cpu *c) { NOTE(c, "account network service account -> unavailable"); ret_x(c, R_ACCOUNT_NSA_UNAVAILABLE); }

// ==== nn::time: the network clock is never corrected
static void ov_time_accuracy(Cpu *c) { ret_x(c, 0); }   // StandardNetworkSystemClock::IsAccuracySufficient()

// ==== nn::crypto: the random source (csrng)
// void GenerateCryptographicallyRandomBytes(void *, size_t)
static void ov_random_bytes(Cpu *c) {
    void *p = arg_p(c, 0);
    uint64_t n = arg_x(c, 1);
    if (n && (!p || !plat_random(p, (size_t)n))) rt_fatal("nn::crypto::GenerateCryptographicallyRandomBytes: %llu bytes",
                                                          (unsigned long long)n);
}

// ==== registration
HOST_OVERRIDE("_ZN2nn4nifm10InitializeEv", ov_nifm_initialize);
HOST_OVERRIDE("_ZN2nn4nifm18IsNetworkAvailableEv", ov_nifm_available);
HOST_OVERRIDE("_ZN2nn4nifm19SetLocalNetworkModeEb", ov_nifm_void);
HOST_OVERRIDE("_ZN2nn4nifm20CancelNetworkRequestEv", ov_nifm_void);
HOST_OVERRIDE("_ZN2nn4nifm22GetCurrentIpConfigInfoEP7in_addrS2_S2_S2_S2_", ov_nifm_unavailable);
HOST_OVERRIDE("_ZN2nn4nifm22GetCurrentProxySettingEPNS0_12ProxySettingE", ov_nifm_unavailable);
HOST_OVERRIDE("_ZN2nn4nifm22IsNetworkRequestOnHoldEv", ov_nifm_on_hold);
HOST_OVERRIDE("_ZN2nn4nifm27SubmitNetworkRequestAndWaitEv", ov_nifm_void);
HOST_OVERRIDE("_ZN2nn4nifm31HandleNetworkRequestErrorResultEv", ov_nifm_handle_error);

HOST_OVERRIDE("_ZN2nn6socket10InitializeEPvmmi", ov_sock_initialize);
HOST_OVERRIDE("_ZN2nn6socket8FinalizeEv", ov_sock_finalize);
HOST_OVERRIDE("_ZN2nn6socket6SocketENS0_6FamilyENS0_4TypeENS0_8ProtocolE", ov_sock_create);
HOST_OVERRIDE("_ZN2nn6socket6SocketEiii", ov_sock_create);
HOST_OVERRIDE("_ZN2nn6socket7EventFdEmNS0_12EventFdFlagsE", ov_sock_create);
HOST_OVERRIDE("_ZN2nn6socket10SetSockOptEiNS0_5LevelENS0_6OptionEPKvj", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket11EventFdReadEiPm", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket12EventFdWriteEim", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket4BindEiPK8sockaddrj", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket4BindEiPKNS0_8SockAddrEj", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket4PollEP6pollfdmi", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket5CloseEi", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket6SendToEiPKvmNS0_7MsgFlagEPKNS0_8SockAddrEj", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket6SendToEiPKvmiPK8sockaddrj", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket8RecvFromEiPvmNS0_7MsgFlagEPNS0_8SockAddrEPj", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket8RecvFromEiPvmiP8sockaddrPj", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket8ShutdownEii", ov_sock_bad_fd);
HOST_OVERRIDE("_ZN2nn6socket11GetAddrInfoEPKcS2_PK8addrinfoPPS3_", ov_sock_getaddrinfo);
HOST_OVERRIDE("_ZN2nn6socket11GetAddrInfoEPKcS2_PKNS0_8AddrInfoEPPS3_", ov_sock_getaddrinfo);
HOST_OVERRIDE("_ZN2nn6socket12FreeAddrInfoEP8addrinfo", ov_sock_freeaddrinfo);
HOST_OVERRIDE("_ZN2nn6socket12FreeAddrInfoEPNS0_8AddrInfoE", ov_sock_freeaddrinfo);
HOST_OVERRIDE("_ZN2nn6socket11GAIStrErrorENS0_7AiErrnoE", ov_sock_gai_strerror);
HOST_OVERRIDE("_ZN2nn6socket12GetLastErrnoEv", ov_sock_errno);
HOST_OVERRIDE("_ZN2nn6socket12GetLastErrorEv", ov_sock_errno);
HOST_OVERRIDE("_ZN2nn6socket8InetNtopEiPKvPcj", ov_inet_ntop);
HOST_OVERRIDE("_ZN2nn6socket8InetPtonEiPKcPv", ov_inet_pton);
HOST_OVERRIDE("_ZN2nn6socket9InetHtonlEj", ov_htonl);
HOST_OVERRIDE("_ZN2nn6socket9InetHtonsEt", ov_htons);
HOST_OVERRIDE("_ZN2nn6socket9InetNtohlEj", ov_htonl);
HOST_OVERRIDE("_ZN2nn6socket9InetNtohsEt", ov_htons);
HOST_OVERRIDE("nnsocketSocket", ov_sock_create);
HOST_OVERRIDE("nnsocketAccept", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketBind", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketClose", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketConnect", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketFcntl", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketGetPeerName", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketGetSockName", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketGetSockOpt", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketListen", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketPoll", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketRecv", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketRecvMsg", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketSend", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketSendMsg", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketSetSockOpt", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketShutdown", ov_sock_bad_fd);
HOST_OVERRIDE("nnsocketGetAddrInfo", ov_sock_getaddrinfo);
HOST_OVERRIDE("nnsocketFreeAddrInfo", ov_sock_freeaddrinfo);
HOST_OVERRIDE("nnsocketGAIStrError", ov_sock_gai_strerror);
HOST_OVERRIDE("nnsocketGetHostByNameWithOptions", ov_sock_gethostbyname);
HOST_OVERRIDE("nnsocketGetLastErrno", ov_sock_errno);
HOST_OVERRIDE("nnsocketSetLastErrno", ov_sock_set_errno);
HOST_OVERRIDE("nnsocketResolverSetOption", ov_sock_resolver_set);
HOST_OVERRIDE("nnsocketResolverGetOption", ov_sock_resolver_get);
HOST_OVERRIDE("nnsocketInetHtonl", ov_htonl);
HOST_OVERRIDE("nnsocketInetHtons", ov_htons);
HOST_OVERRIDE("nnsocketInetNtohl", ov_htonl);
HOST_OVERRIDE("nnsocketInetNtohs", ov_htons);

HOST_OVERRIDE("_ZN2nn3ssl10InitializeEv", ov_ssl_ok);
HOST_OVERRIDE("_ZN2nn3ssl8FinalizeEv", ov_ssl_ok);
HOST_OVERRIDE("_ZN2nn3ssl7Context6CreateENS1_10SslVersionE", ov_ssl_context_create);
HOST_OVERRIDE("_ZN2nn3ssl7Context15ImportServerPkiEPmPKcjNS0_17CertificateFormatE", ov_ssl_import);
HOST_OVERRIDE("_ZN2nn3ssl7Context15ImportClientPkiEPmPKcS4_jj", ov_ssl_import);
HOST_OVERRIDE("_ZN2nn3ssl7Context9ImportCrlEPmPKcj", ov_ssl_import);
HOST_OVERRIDE("_ZN2nn3ssl10Connection19SetSessionCacheModeENS1_16SessionCacheModeE", ov_ssl_unavailable);
HOST_OVERRIDE("_ZN2nn3ssl14BuiltInManager22GetBuiltInCertificatesEPPNS0_26BuiltInCertificateInfoBaseEPhjPNS0_15CaCertificateIdEj",
              ov_ssl_unavailable);
HOST_OVERRIDE("_ZN2nn3ssl14BuiltInManager28GetBuiltInCertificateBufSizeEPjPNS0_15CaCertificateIdEj", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslInitialize", ov_ssl_ok);
HOST_OVERRIDE("nnsslFinalize", ov_ssl_ok);
HOST_OVERRIDE("nnsslContextDestroy", ov_ssl_ok);
HOST_OVERRIDE("nnsslContextGetContextId", ov_ssl_context_id);
HOST_OVERRIDE("nnsslConnectionCreate", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionDestroy", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionDoHandshakeWithCertBuffer", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionGetConnectionId", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionGetNextAlpnProto", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionGetServerCertDetail", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionGetVerifyCertError", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionPeek", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionPending", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionRead", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetHostName", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetIoMode", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetNextAlpnProto", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetOption", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetServerCertBuffer", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetSessionCacheMode", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetSocketDescriptor", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionSetVerifyOption", ov_ssl_unavailable);
HOST_OVERRIDE("nnsslConnectionWrite", ov_ssl_unavailable);

HOST_OVERRIDE("_ZN2nn3ldn10InitializeEv", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn8FinalizeEv", ov_ldn_void);
HOST_OVERRIDE("_ZN2nn3ldn8GetStateEv", ov_ldn_state);
HOST_OVERRIDE("_ZN2nn3ldn19GetDisconnectReasonEv", ov_ldn_reason);
HOST_OVERRIDE("_ZN2nn3ldn20MakeBroadcastAddressENS0_11Ipv4AddressENS0_10SubnetMaskE", ov_ldn_broadcast);
HOST_OVERRIDE("_ZN2nn3ldn22AttachStateChangeEventEPNS_2os15SystemEventTypeE", ov_ldn_state_event);
HOST_OVERRIDE("_ZN2nn3ldn10DisconnectEv", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn11OpenStationEv", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn11ScanPrivateEPNS0_11NetworkInfoEPiiRKNS0_10ScanFilterEi", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn11SetProtocolENS0_8ProtocolE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn12CloseStationEv", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn13CreateNetworkERKNS0_13NetworkConfigERKNS0_14SecurityConfigERKNS0_10UserConfigE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn14ConnectPrivateERKNS0_13NetworkConfigERKNS0_14SecurityConfigERKNS0_17SecurityParameterERKNS0_"
              "10UserConfigEiNS0_13ConnectOptionE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn14DestroyNetworkEv", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn14GetIpv4AddressEPNS0_11Ipv4AddressEPNS0_10SubnetMaskE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn14GetNetworkInfoEPNS0_11NetworkInfoE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn15OpenAccessPointEv", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn16CloseAccessPointEv", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn16SetAdvertiseDataEPKvm", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn20AddAcceptFilterEntryENS0_10MacAddressE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn20CreateNetworkPrivateERKNS0_13NetworkConfigERKNS0_14SecurityConfigERKNS0_17SecurityParameterE"
              "RKNS0_10UserConfigEiPKNS0_12AddressEntryE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn20GetSecurityParameterEPNS0_17SecurityParameterE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn22SetStationAcceptPolicyENS0_12AcceptPolicyE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn4ScanEPNS0_11NetworkInfoEPiiRKNS0_10ScanFilterEi", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn6RejectENS0_11Ipv4AddressE", ov_ldn_unavailable);
HOST_OVERRIDE("_ZN2nn3ldn7ConnectERKNS0_11NetworkInfoERKNS0_14SecurityConfigERKNS0_10UserConfigEiNS0_13ConnectOptionE",
              ov_ldn_unavailable);

HOST_OVERRIDE("_ZN2nn7friends13GetFriendListEPiPNS_7account23NetworkServiceAccountIdEiiRKNS0_12FriendFilterE",
              ov_friends_list);
HOST_OVERRIDE("_ZN2nn7friends14AddPlayHistoryERKNS0_26PlayHistoryRegistrationKeyERKNS0_15InAppScreenNameES6_",
              ov_friends_ok);
HOST_OVERRIDE("_ZN2nn7friends14GetProfileListEPNS0_12AsyncContextEPNS0_7ProfileEPKNS_7account23NetworkServiceAccountIdEi",
              ov_friends_unavailable);
HOST_OVERRIDE("_ZN2nn7friends16UpdateFriendInfoEPNS0_6FriendEPKNS_7account23NetworkServiceAccountIdEi",
              ov_friends_unavailable);
HOST_OVERRIDE("_ZN2nn7friends25EnsureFriendListAvailableEPNS0_12AsyncContextE", ov_friends_unavailable);
HOST_OVERRIDE("_ZN2nn7friends29GetPlayHistoryRegistrationKeyEPNS0_26PlayHistoryRegistrationKeyEb", ov_friends_unavailable);
HOST_OVERRIDE("_ZN2nn7friends12AsyncContext7HasDoneEPb", ov_friends_done);
HOST_OVERRIDE("_ZNK2nn7friends12AsyncContext9GetResultEv", ov_friends_result);

HOST_OVERRIDE("_ZN2nn4bcat10InitializeEv", ov_bcat_initialize);
HOST_OVERRIDE("_ZN2nn4bcat25MountDeliveryCacheStorageEv", ov_bcat_unavailable);
HOST_OVERRIDE("_ZN2nn4bcat27UnmountDeliveryCacheStorageEv", ov_bcat_void);
HOST_OVERRIDE("_ZN2nn4bcat24RequestSyncDeliveryCacheEPNS0_21DeliveryCacheProgressEPKc", ov_bcat_unavailable);
HOST_OVERRIDE("_ZN2nn4bcat31EnumerateDeliveryCacheDirectoryEPiPNS0_13DirectoryNameEi", ov_bcat_unavailable);
HOST_OVERRIDE("_ZN2nn4bcat17DeliveryCacheFile4OpenEPKc", ov_bcat_unavailable);
HOST_OVERRIDE("_ZN2nn4bcat17DeliveryCacheFile4ReadEPmlPvm", ov_bcat_unavailable);
HOST_OVERRIDE("_ZN2nn4bcat17DeliveryCacheFile5CloseEv", ov_bcat_void);
HOST_OVERRIDE("_ZN2nn4bcat17DeliveryCacheFile7GetSizeEv", ov_bcat_zero);
HOST_OVERRIDE("_ZN2nn4bcat22DeliveryCacheDirectory4OpenEPKc", ov_bcat_unavailable);
HOST_OVERRIDE("_ZN2nn4bcat22DeliveryCacheDirectory4ReadEPiPNS0_27DeliveryCacheDirectoryEntryEi", ov_bcat_unavailable);
HOST_OVERRIDE("_ZN2nn4bcat22DeliveryCacheDirectory5CloseEv", ov_bcat_void);
HOST_OVERRIDE("_ZN2nn4bcat21DeliveryCacheProgress6UpdateEv", ov_bcat_void);
HOST_OVERRIDE("_ZNK2nn4bcat21DeliveryCacheProgress18GetWholeDownloadedEv", ov_bcat_zero);
HOST_OVERRIDE("_ZNK2nn4bcat21DeliveryCacheProgress9GetResultEv", ov_bcat_result);
HOST_OVERRIDE("_ZNK2nn4bcat21DeliveryCacheProgress9GetStatusEv", ov_bcat_status);

HOST_OVERRIDE("_ZN2nn3nsd24GetEnvironmentIdentifierEPNS0_21EnvironmentIdentifierE", ov_nsd_environment);
HOST_OVERRIDE("_ZN2nn3nsd9ResolveExEPNS0_4FqdnERKS1_", ov_nsd_resolve);

HOST_OVERRIDE("_ZN2nn3ngc15ProfanityFilter10InitializeEPvm", ov_ngc_ok);
HOST_OVERRIDE("_ZN2nn3ngc15ProfanityFilter8FinalizeEv", ov_ngc_ok);
HOST_OVERRIDE("_ZN2nn3ngc15ProfanityFilter15SkipAtSignCheckENS1_8SkipModeE", ov_ngc_void);
HOST_OVERRIDE("_ZN2nn3ngc15ProfanityFilter24MaskProfanityWordsInTextEPiPcj", ov_ngc_mask);

HOST_OVERRIDE("_ZN2nn2ec24ShowShopAddOnContentListEv", ov_ec_shop);
HOST_OVERRIDE("_ZN2nn3web13ShowLobbyPageEPNS0_20LobbyPageReturnValueERKNS0_16ShowLobbyPageArgE", ov_web_unavailable);
HOST_OVERRIDE("_ZN2nn5album10InitializeEv", ov_album_initialize);
HOST_OVERRIDE("_ZN2nn5album8FinalizeEv", ov_album_void);
HOST_OVERRIDE("_ZN2nn5album14SaveScreenshotEPKvmNS0_9ImageSizeENS0_16ImageOrientationENS0_17AlbumReportOptionE",
              ov_album_unavailable);
HOST_OVERRIDE("_ZN2nn5album22SaveAndShareScreenshotEPNS0_7ShareIdEPKvmNS0_9ImageSizeENS0_16ImageOrientationENS0_"
              "17AlbumReportOptionERKNS_7account3UidE", ov_album_unavailable);
HOST_OVERRIDE("_ZN2nn5album21GetSharePostedEntryIdEPcmRKNS0_7ShareIdE", ov_album_unavailable);
HOST_OVERRIDE("_ZN2nn5album25GetSharePostedServiceNameEPcmRKNS0_7ShareIdE", ov_album_unavailable);

HOST_OVERRIDE("_ZN2nn4pctl28IsFreeCommunicationAvailableEv", ov_pctl_true);
HOST_OVERRIDE("_ZN2nn4pctl25TryBeginFreeCommunicationEb", ov_pctl_true);
HOST_OVERRIDE("_ZN2nn4pctl20EndFreeCommunicationEv", ov_pctl_void);

HOST_OVERRIDE("_ZN2nn7account32IsNetworkServiceAccountAvailableEPbRKNS0_10UserHandleE", ov_nsa_available);
HOST_OVERRIDE("_ZN2nn7account36EnsureNetworkServiceAccountAvailableERKNS0_10UserHandleE", ov_nsa_unavailable);
HOST_OVERRIDE("_ZN2nn7account26GetNetworkServiceAccountIdEPNS0_23NetworkServiceAccountIdERKNS0_10UserHandleE",
              ov_nsa_unavailable);
HOST_OVERRIDE("_ZN2nn7account37LoadNetworkServiceAccountIdTokenCacheEPmPcmRKNS0_10UserHandleE", ov_nsa_unavailable);
HOST_OVERRIDE("_ZN2nn7account44EnsureNetworkServiceAccountIdTokenCacheAsyncEPNS0_12AsyncContextERKNS0_10UserHandleE",
              ov_nsa_unavailable);
HOST_OVERRIDE("_ZN2nn7account29LoadCachedNintendoAccountInfoEPNS0_25CachedNintendoAccountInfoERKNS0_10UserHandleEPvm",
              ov_nsa_unavailable);

HOST_OVERRIDE("_ZN2nn4time26StandardNetworkSystemClock20IsAccuracySufficientEv", ov_time_accuracy);

HOST_OVERRIDE("_ZN2nn6crypto36GenerateCryptographicallyRandomBytesEPvm", ov_random_bytes);
