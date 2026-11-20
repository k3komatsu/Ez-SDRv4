/* sync_check: bench program for 8b's D-1/D-4 checks (2026-11-27). UHD C API only. RX and time only: nothing here transmits. */
#define _POSIX_C_SOURCE 200809L
#include <complex.h>
#include <math.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <uhd.h>

#define TICK_HZ 200000000LL /* master clock 200 MHz: the tick of the Module's time model */
#define MAXDEV 8

typedef struct { uhd_usrp_handle h; size_t mb; } dev_t_;

static struct timespec g_t0;
static const char *g_stage = "S?";
static int g_run = 0;
static atomic_int g_claim, g_done, g_code; /* the first of main and the watchdog to claim prints the run's only verdict line */
static void claim_line(int code) { atomic_store(&g_code, code); if (atomic_exchange(&g_claim, 1)) for (;;) pause(); }
static void line_done(void) { fflush(stdout); atomic_store(&g_done, 1); }
#define LINE(code, ...) do { claim_line(code); printf(__VA_ARGS__); line_done(); } while (0)
static uhd_usrp_handle g_h = NULL; /* handle whose last_error explains a failure; NULL = global error */

static double now_ms(void) {
    struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t);
    return (t.tv_sec - g_t0.tv_sec) * 1e3 + (t.tv_nsec - g_t0.tv_nsec) / 1e6;
}
static void sleep_ms(int ms) { struct timespec t = {ms / 1000, (ms % 1000) * 1000000L}; nanosleep(&t, NULL); }

static void die_msg(const char *call, const char *msg) {
    char m[600]; snprintf(m, sizeof m, "%s: %s", call, msg);
    for (char *p = m; *p; p++) if (*p == '"' || *p == '\n' || *p == '\r') *p = '\'';
    LINE(2, "%s run=%d verdict=ERROR error=\"%s\"\n", g_stage, g_run, m);
    exit(2);
}
static void die(const char *call, uhd_error e) {
    char msg[400] = "";
    if (g_h) uhd_usrp_last_error(g_h, msg, sizeof msg); else uhd_get_last_error(msg, sizeof msg);
    if (!msg[0]) snprintf(msg, sizeof msg, "uhd_error %d", (int)e);
    char c[80]; snprintf(c, sizeof c, "%s", call); char *p = strchr(c, '('); if (p) *p = 0;
    die_msg(c, msg);
}
#define U(call) do { uhd_error e_ = (call); if (e_) die(#call, e_); } while (0)

static int64_t ticks(int64_t full, double frac) { return full * TICK_HZ + llround(frac * (double)TICK_HZ); }
static int64_t lpps(dev_t_ d) { int64_t s; double f; g_h = d.h; U(uhd_usrp_get_time_last_pps(d.h, d.mb, &s, &f)); return ticks(s, f); }
static int64_t tnow(dev_t_ d) { int64_t s; double f; g_h = d.h; U(uhd_usrp_get_time_now(d.h, d.mb, &s, &f)); return ticks(s, f); }

static int lock_state(dev_t_ d) { /* 1 locked, 0 not, -1 read failed (no die: the monitor thread uses it). UHD fills a handle made by sensor_value_make. */
    uhd_sensor_value_handle sv = NULL; bool b = false; int r = -1;
    if (uhd_sensor_value_make(&sv)) return -1;
    if (!uhd_usrp_get_mboard_sensor(d.h, "ref_locked", d.mb, &sv) && !uhd_sensor_value_to_bool(sv, &b)) r = (int)b;
    uhd_sensor_value_free(&sv); return r;
}
static int ref_locked(dev_t_ d) { int r = lock_state(d); g_h = d.h; if (r < 0) die_msg("get_mboard_sensor", "ref_locked unreadable"); return r; }

static double open_dev(const char *args, uhd_usrp_handle *h, int dieonfail, char *err, size_t el) {
    double t = now_ms(); g_h = NULL;
    uhd_error e = uhd_usrp_make(h, args);
    if (e) { if (dieonfail) die("uhd_usrp_make", e); uhd_get_last_error(err, el); *h = NULL; }
    return now_ms() - t;
}

static int wait_change(dev_t_ d, int64_t old, int timeout_ms) { /* 1 when last_pps no longer equals old */
    for (int i = 0; i <= timeout_ms; i++) { if (lpps(d) != old) return 1; sleep_ms(1); }
    return 0;
}
static int wait_edge(dev_t_ d, int timeout_ms) { return wait_change(d, lpps(d), timeout_ms); }

/* Wait for the follower to lock after switching to external; ms to lock, -1 if not within 30 s. */
static double switch_follower(dev_t_ d) {
    double t = now_ms(); g_h = d.h;
    U(uhd_usrp_set_clock_source(d.h, "external", d.mb));
    U(uhd_usrp_set_time_source(d.h, "external", d.mb));
    while (now_ms() - t < 30000) { if (ref_locked(d)) return now_ms() - t; sleep_ms(50); }
    return -1;
}

#define BOUND (TICK_HZ / 100)               /* 10 ms */
#define NEWTIME_MAX (TICK_HZ + TICK_HZ / 10) /* a time set to T reads in [T, T + 1.1 s) at the edge after the next one */
#define NONCE_S 1000                          /* the set value lies this many seconds beyond every handle's current time */

/* The value to set: far beyond every handle's current time (tn, ticks), so that reading it back proves the command took effect. */
static int64_t nonce_s(const int64_t *tn, int n) { int64_t m = 0; for (int i = 0; i < n; i++) if (tn[i] > m) m = tn[i]; return m / TICK_HZ + NONCE_S; }

/* One reading of draft 3.7 step 4. t = the set value in ticks; a, b = last_pps(h0) before and after; ta, tb = time_now(h0) before and after the others are read;
 * L, tn = the others' last_pps and time_now (index 0 unused). 1 pass, 0 refuse (why), -1 read again (the bracket crossed an edge or is wider than the bound). */
static int bracket_ok(int64_t t, int64_t a, int64_t b, const int64_t *L, const int64_t *tn, int64_t ta, int64_t tb, int n, int64_t *delta, char *why, size_t wl) {
    *delta = 0;
    for (int i = 1; i < n; i++) { int64_t d = tn[i] < ta ? ta - tn[i] : tn[i] > tb ? tn[i] - tb : 0; if (d > *delta) *delta = d; }
    if (a != b || tb - ta > BOUND) return -1;
    if (ta < t || tb >= t + NEWTIME_MAX) { snprintf(why, wl, "time_now(h0) is not the set time"); return 0; }
    for (int i = 1; i < n; i++) if (tn[i] < t || tn[i] >= t + NEWTIME_MAX) { snprintf(why, wl, "time_now(h%d)=%lld is not the set time", i, (long long)tn[i]); return 0; }
    if (a % TICK_HZ != 0 || a < t || a >= t + NEWTIME_MAX) { snprintf(why, wl, "last_pps(h0)=%lld is not the edge that took the set time", (long long)a); return 0; }
    for (int i = 1; i < n; i++) if (L[i] != a) { snprintf(why, wl, "last_pps(h%d)=%lld != last_pps(h0)=%lld", i, (long long)L[i], (long long)a); return 0; }
    if (*delta > BOUND) { snprintf(why, wl, "time_now differs from the h0 bracket by more than 10 ms"); return 0; }
    return 1;
}

typedef struct { int attempts, rereads, voids; int64_t set_s, lp[MAXDEV]; int64_t tdelta; char why[200]; } settime_t;

/* Refused unless every attempt was void (no valid comparison was made): then inconclusive. */
static int settime_result(int attempts, int voids) { return voids == attempts ? 2 : 0; }

/* draft 3.7: arm inside the second of an edge, check after the next one; <= 3 attempts. 1 ok, 0 refused, 2 inconclusive. An attempt is void when the arming
 * straddled an edge or every verification bracket was discarded; a missing PPS edge on any handle is a valid failure. */
static int set_time(dev_t_ *hs, int n, settime_t *r) {
    memset(r, 0, sizeof *r);
    for (int at = 1; at <= 3; at++) {
        r->attempts = at;
        if (!wait_edge(hs[0], 1100)) { snprintf(r->why, sizeof r->why, "no PPS edge at handle 0"); continue; }
        int64_t cur[MAXDEV]; for (int i = 0; i < n; i++) cur[i] = tnow(hs[i]);
        r->set_s = nonce_s(cur, n);
        int64_t p0 = lpps(hs[0]);
        for (int i = 0; i < n; i++) { g_h = hs[i].h; U(uhd_usrp_set_time_next_pps(hs[i].h, r->set_s, 0.0, hs[i].mb)); }
        if (lpps(hs[0]) != p0) { r->voids++; snprintf(r->why, sizeof r->why, "an edge passed while the commands were issued"); continue; }
        int64_t pre[MAXDEV]; for (int i = 1; i < n; i++) pre[i] = lpps(hs[i]);
        if (!wait_change(hs[0], p0, 1100)) { snprintf(r->why, sizeof r->why, "no second PPS edge at handle 0"); continue; }
        int missing = 0; /* each follower's own edge must have come before it is read */
        for (int i = 1; i < n && !missing; i++) if (!wait_change(hs[i], pre[i], 1200)) { snprintf(r->why, sizeof r->why, "no PPS edge on h%d", i); missing = 1; }
        if (missing) continue;
        sleep_ms(10);
        int fin = 0;
        for (int rd = 0; rd < 3 && !fin; rd++) {
            int64_t a = lpps(hs[0]), ta = tnow(hs[0]), tn[MAXDEV];
            r->lp[0] = a;
            for (int i = 1; i < n; i++) { r->lp[i] = lpps(hs[i]); tn[i] = tnow(hs[i]); }
            int64_t tb = tnow(hs[0]), b = lpps(hs[0]);
            int v = bracket_ok(r->set_s * TICK_HZ, a, b, r->lp, tn, ta, tb, n, &r->tdelta, r->why, sizeof r->why);
            if (v < 0) { r->rereads++; continue; }
            if (v) { r->why[0] = 0; return 1; }
            fin = 1;
        }
        if (!fin) { r->voids++; snprintf(r->why, sizeof r->why, "every verification bracket crossed an edge or was wider than 10 ms"); }
    }
    return settime_result(r->attempts, r->voids);
}

/* Each handle's distinct last_pps values, in the order its own edges brought them (the first is the value at the start). */
typedef struct { int64_t v[MAXDEV][16]; int cnt[MAXDEV]; } edges_t;
static void edge_obs(edges_t *e, int i, int64_t val) { if (e->cnt[i] < 16 && (e->cnt[i] == 0 || e->v[i][e->cnt[i] - 1] != val)) e->v[i][e->cnt[i]++] = val; }
static int edge_diffs(const edges_t *e, int i, int64_t *out) { int m = e->cnt[i] > 0 ? e->cnt[i] - 1 : 0; if (m > 8) m = 8; for (int j = 0; j < m; j++) out[j] = e->v[i][j + 1] - e->v[i][j]; return m; }
static int missed_of(const edges_t *e, int i, int k) { return k - (e->cnt[i] > 0 ? e->cnt[i] - 1 : 0); }

/* Poll every handle's last_pps every ~1 ms until handle 0 has shown k new edges and 0.5 s more has passed; each handle keeps its own edges. */
static void incr(dev_t_ *hs, int n, int k, edges_t *e) {
    memset(e, 0, sizeof *e);
    double t0 = now_ms(), stop = -1;
    for (;;) {
        for (int i = 0; i < n; i++) edge_obs(e, i, lpps(hs[i]));
        double t = now_ms();
        if (stop < 0 && e->cnt[0] > k) stop = t + 500;
        if (stop >= 0 && t >= stop) return;
        if (t - t0 > 1000.0 * (k + 2)) die_msg("incr", "handle 0 did not show the PPS edges");
        sleep_ms(1);
    }
}

static char *lst(char *b, size_t bl, const int64_t *v, int n) {
    size_t o = 0; b[0] = 0;
    for (int i = 0; i < n && o < bl; i++) o += snprintf(b + o, bl - o, "%s%lld", i ? "," : "", (long long)v[i]);
    return b;
}
static void rst(char *s) { for (; *s; s++) if (*s == '"' || *s == '\n') *s = '\''; }

/* ---- separate: the Module's procedure (open internal, switch the follower, one edge for time) ---- */
typedef struct {
    double open_ms[2], lock1_ms; int lk[2], order10; settime_t st; edges_t ed; int res; /* 1 PASS, 0 REFUSED, 2 INCONCLUSIVE */ char why[220];
    dev_t_ d[2]; int pps_out; /* -1 none, 0 off, 1 on */
} sep_t;

static void sep_close(sep_t *r) { for (int i = 0; i < 2; i++) if (r->d[i].h) uhd_usrp_free(&r->d[i].h); }

static void separate(const char *a0, const char *a1, int order10, int pps_out, sep_t *r) {
    memset(r, 0, sizeof *r); r->pps_out = pps_out; r->order10 = order10;
    const char *ar[2] = {a0, a1}; int ord[2] = {order10 ? 1 : 0, order10 ? 0 : 1};
    for (int k = 0; k < 2; k++) { int i = ord[k]; r->open_ms[i] = open_dev(ar[i], &r->d[i].h, 1, NULL, 0); }
    if (pps_out >= 0) { g_h = r->d[0].h; U(uhd_usrp_set_time_source_out(r->d[0].h, pps_out == 1, 0)); }
    r->lock1_ms = switch_follower(r->d[1]);
    r->lk[0] = ref_locked(r->d[0]);
    if (r->lock1_ms < 0) { r->lk[1] = ref_locked(r->d[1]); snprintf(r->why, sizeof r->why, "handle 1 did not lock to the external reference in 30 s"); return; }
    int ok = set_time(r->d, 2, &r->st);
    if (ok != 1) { r->res = ok; snprintf(r->why, sizeof r->why, "%s (after %d attempts)", r->st.why, r->st.attempts); r->lk[1] = ref_locked(r->d[1]); return; }
    incr(r->d, 2, 3, &r->ed);
    r->lk[1] = ref_locked(r->d[1]); r->res = 1;
}

static int print_sep(const char *stage, sep_t *r) {
    char lp[80], i0[80], i1[80], l2[40], ms[40];
    int64_t lkv[2] = {r->lk[0], r->lk[1]}, lpv[2] = {r->st.lp[0], r->st.lp[1]}, mv[2] = {missed_of(&r->ed, 0, 3), missed_of(&r->ed, 1, 3)}, a[8], b[8];
    int na = edge_diffs(&r->ed, 0, a), nb = edge_diffs(&r->ed, 1, b);
    rst(r->why);
    int code = r->res == 1 ? 0 : r->res == 2 ? 3 : 1;
    LINE(code, "%s run=%d order=%s pps_out=%s open_ms=%.0f,%.0f lock1_ms=%.0f ref_locked=%s attempts=%d void_attempts=%d set_s=%lld bracket_rereads=%d last_pps=%s tnow_delta_ns=%lld incr0=%s incr1=%s missed=%s verdict=%s reason=\"%s\"\n",
           stage, g_run, r->order10 ? "10" : "01", r->pps_out < 0 ? "none" : r->pps_out ? "on" : "off", r->open_ms[0], r->open_ms[1], r->lock1_ms, lst(l2, sizeof l2, lkv, 2),
           r->st.attempts, r->st.voids, (long long)r->st.set_s, r->st.rereads, lst(lp, sizeof lp, lpv, 2), (long long)(r->st.tdelta * 5), lst(i0, sizeof i0, a, na), lst(i1, sizeof i1, b, nb),
           lst(ms, sizeof ms, mv, 2), r->res == 1 ? "PASS" : r->res == 2 ? "INCONCLUSIVE" : "REFUSED", r->why);
    return code;
}

/* ---- align: cross-correlation lag of two RX captures, +-maxlag samples. lag > 0: x1 is the later copy. ---- */
static double nrm(const double _Complex *x, int n) { double s = 0; for (int i = 0; i < n; i++) s += creal(x[i] * conj(x[i])); return s; }
static int xcorr_lag(const double _Complex *x0, const double _Complex *x1, int n, int maxlag, double *peak_db, double *second_db) {
    double best = -1, mag[2 * 64 + 1 + 2 * 64]; int bl = 0; double den = sqrt(nrm(x0, n) * nrm(x1, n));
    if (maxlag > 64) maxlag = 64;
    for (int l = -maxlag; l <= maxlag; l++) {
        double _Complex s = 0;
        for (int i = (l < 0 ? -l : 0); i < n && i + l < n; i++) s += x1[i + l] * conj(x0[i]);
        mag[l + maxlag] = cabs(s);
        if (mag[l + maxlag] > best) { best = mag[l + maxlag]; bl = l; }
    }
    double sec = 0;
    for (int l = -maxlag; l <= maxlag; l++) if (abs(l - bl) > 1 && mag[l + maxlag] > sec) sec = mag[l + maxlag];
    *peak_db = 20 * log10(best / den + 1e-30); *second_db = 20 * log10(sec / den + 1e-30);
    return bl;
}

static void rx_setup(uhd_usrp_handle h, double f, double r, double g, uhd_rx_streamer_handle *s) {
    uhd_tune_request_t tr; memset(&tr, 0, sizeof tr);
    tr.target_freq = f; tr.rf_freq_policy = UHD_TUNE_REQUEST_POLICY_AUTO; tr.dsp_freq_policy = UHD_TUNE_REQUEST_POLICY_AUTO;
    uhd_tune_result_t res; memset(&res, 0, sizeof res);
    size_t ch = 0; char cpu[] = "fc32", otw[] = "sc16", none[] = "";
    uhd_stream_args_t sa = {cpu, otw, none, &ch, 1};
    g_h = h;
    U(uhd_usrp_set_rx_rate(h, r, 0)); U(uhd_usrp_set_rx_freq(h, &tr, 0, &res)); U(uhd_usrp_set_rx_gain(h, g, 0, ""));
    U(uhd_usrp_set_rx_antenna(h, "RX2", 0));
    U(uhd_rx_streamer_make(s)); U(uhd_usrp_get_rx_stream(h, &sa, *s));
}
static void rx_grab(uhd_rx_streamer_handle s, uhd_rx_metadata_handle md, float _Complex *buf, size_t k) {
    size_t tot = 0; double t0 = now_ms();
    while (tot < k) {
        void *b[1] = {buf + tot}; size_t got = 0; uhd_rx_metadata_error_code_t ec;
        if (now_ms() - t0 > 10000) die_msg("rx_streamer_recv", "no samples in 10 s");
        if (uhd_rx_streamer_recv(s, b, k - tot, &md, 3.0, false, &got)) die_msg("rx_streamer_recv", "failed");
        uhd_rx_metadata_error_code(md, &ec);
        if (ec != UHD_RX_METADATA_ERROR_CODE_NONE) { char m[40]; snprintf(m, sizeof m, "rx metadata error code %d", (int)ec); die_msg("rx_streamer_recv", m); }
        tot += got;
    }
}

/* ---- reopen-source: monitor h1's ref_locked while h0 is opened (all counters atomic: main reads them while the thread runs) ---- */
typedef struct { dev_t_ d; double t_ref; atomic_int stop, samples, falses, first_false, relock; } mon_t;
static void *monitor(void *p) {
    mon_t *m = p;
    while (!atomic_load(&m->stop)) {
        int s = lock_state(m->d), t = (int)(now_ms() - m->t_ref); atomic_fetch_add(&m->samples, 1); /* an unreadable sensor counts as not locked */
        if (s != 1) { atomic_fetch_add(&m->falses, 1); if (atomic_load(&m->first_false) < 0) atomic_store(&m->first_false, t); }
        else if (atomic_load(&m->first_false) >= 0 && atomic_load(&m->relock) < 0) atomic_store(&m->relock, t - atomic_load(&m->first_false));
        sleep_ms(10);
    }
    return NULL;
}

static int selftest(void) {
    int bad = 0;
#define T(c) do { if (!(c)) { printf("selftest FAIL line %d: %s\n", __LINE__, #c); bad++; } } while (0)
    T(ticks(3, 0.5) == 700000000LL); T(ticks(0, 1.0 / 200e6 * 1.4) == 1); T(ticks(2, 0.0) == 400000000LL);
    char w[200]; int64_t dl, S = TICK_HZ, B = BOUND, T = 1000 * S, Ls[2] = {T, T};
    /* nonce: far beyond every handle's current time */
    T(nonce_s((int64_t[]){5 * S + 3, 7 * S}, 2) == 7 + NONCE_S); T(nonce_s((int64_t[]){0}, 1) == NONCE_S);
    /* bracket_ok(t, a, b, L, tn, ta, tb, n): 1 pass, 0 refuse, -1 re-read. After the edge every handle reads in [T, T + 1.1 s) and took the same edge. */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, T + 550}, T + 500, T + 600, 2, &dl, w, sizeof w) == 1 && dl == 0);       /* h1 inside the bracket */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, T + 30}, T + 5, T + 20, 2, &dl, w, sizeof w) == 1 && dl == 10);          /* 10 ticks after the bracket */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, T + 100 + B}, T + 100, T + 100, 2, &dl, w, sizeof w) == 1);              /* exactly 10 ms */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, T + 101 + B}, T + 100, T + 100, 2, &dl, w, sizeof w) == 0);              /* 1 tick over */
    T(bracket_ok(T, T, T, (int64_t[]){T, T + S}, (int64_t[]){0, T + 5}, T + 5, T + 5, 2, &dl, w, sizeof w) == 0);     /* h1 took another edge */
    T(bracket_ok(T, T + 123, T + 123, (int64_t[]){0, T + 123}, (int64_t[]){0, T + 5}, T + 5, T + 5, 2, &dl, w, sizeof w) == 0); /* not an edge */
    T(bracket_ok(T, 100 * S, 100 * S, (int64_t[]){0, 100 * S}, (int64_t[]){0, 100 * S + 7}, 100 * S + 5, 100 * S + 8, 2, &dl, w, sizeof w) == 0); /* both still at the old second: the command did not take */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, 100 * S}, T + 5, T + 5, 2, &dl, w, sizeof w) == 0);                      /* h1 kept its old time */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, T - 1}, T + 5, T + 5, 2, &dl, w, sizeof w) == 0);                         /* just before the set value */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, T + NEWTIME_MAX}, T + 5, T + 5, 2, &dl, w, sizeof w) == 0);              /* too long after it */
    T(bracket_ok(T, T, T + S, Ls, (int64_t[]){0, T + 5}, T + 5, T + 5, 2, &dl, w, sizeof w) == -1);                    /* the bracket crossed an edge: re-read */
    T(bracket_ok(T, T, T, Ls, (int64_t[]){0, T + 5}, T + 5, T + 5 + B + 1, 2, &dl, w, sizeof w) == -1);               /* the bracket is wider than the bound: re-read */
    T(bracket_ok(T, T, T, (int64_t[]){T}, (int64_t[]){0}, T + 5, T + 6, 1, &dl, w, sizeof w) == 1);                   /* N = 1 */
    T(settime_result(3, 3) == 2); T(settime_result(3, 2) == 0); T(settime_result(3, 0) == 0); T(settime_result(1, 1) == 2); /* all void: inconclusive; any valid attempt: refused */
    /* edges seen by each handle on its own: h1 misses the 2nd edge, h0 sees all */
    { edges_t e; memset(&e, 0, sizeof e); int64_t d[8], h0[] = {0, 0, S, S, 2 * S, 2 * S, 3 * S}, h1[] = {0, 0, 0, S, S, S, 3 * S};
      for (int i = 0; i < 7; i++) { edge_obs(&e, 0, h0[i]); edge_obs(&e, 1, h1[i]); }
      T(edge_diffs(&e, 0, d) == 3 && d[0] == S && d[1] == S && d[2] == S && missed_of(&e, 0, 3) == 0);
      T(edge_diffs(&e, 1, d) == 2 && d[0] == S && d[1] == 2 * S && missed_of(&e, 1, 3) == 1); }
    enum { N = 4096 }; static double _Complex x0[N], x1[N]; unsigned s = 12345;
    for (int i = 0; i < N; i++) { s = s * 1103515245u + 12345u; double re = (s >> 16 & 0x7fff) / 16384.0 - 1; s = s * 1103515245u + 12345u; x0[i] = re + I * ((s >> 16 & 0x7fff) / 16384.0 - 1); }
    int lags[3] = {5, -3, 0};
    for (int k = 0; k < 3; k++) {
        for (int i = 0; i < N; i++) x1[i] = (i - lags[k] >= 0 && i - lags[k] < N) ? x0[i - lags[k]] : 0; /* x1[n] = x0[n - lag] */
        double p, sp; T(xcorr_lag(x0, x1, N, 64, &p, &sp) == lags[k]); T(p > -1.0 && sp < p - 10);
    }
    printf("selftest %s\n", bad ? "FAIL" : "ok"); return bad ? 1 : 0;
}

static void usage(void) {
    fprintf(stderr, "usage: sync_check MODE [--run N] [options] ARGS...\n"
        "  probe A | separate [--order 01|10] [--pps-out on|off] A0 A1 | reopen-source A0 A1 | outputs-off A0\n"
        "  follower-first A1 | compound ARGS | align [--freq F] [--rate R] [--samps K] [--gain G] A0 A1 | selftest\n");
    exit(2);
}

static int g_limit = 300;
static void *watchdog(void *p) { /* a blocked UHD call must not leave a run without its line */
    (void)p; sleep((unsigned)g_limit);
    if (atomic_exchange(&g_claim, 1)) { for (int i = 0; i < 5000 && !atomic_load(&g_done); i++) sleep_ms(1); _exit(atomic_load(&g_code)); } /* main has its line: leave with its status */
    printf("%s run=%d verdict=ERROR error=\"timeout\"\n", g_stage, g_run); fflush(stdout); _exit(2);
}

int main(int argc, char **argv) {
    clock_gettime(CLOCK_MONOTONIC, &g_t0); setvbuf(stdout, NULL, _IOLBF, 0);
    if (argc < 2) usage();
    const char *mode = argv[1], *pos[4]; int np = 0, order10 = 0, pps_out = -1, samps = 65536; double freq = 1e9, rate = 200e6, gain = 0;
    for (int i = 2; i < argc; i++) {
        const char *a = argv[i]; int more = i + 1 < argc;
        if (!strcmp(a, "--run") && more) g_run = atoi(argv[++i]);
        else if (!strcmp(a, "--timeout") && more) g_limit = atoi(argv[++i]);
        else if (!strcmp(a, "--order") && more) { const char *v = argv[++i]; if (!strcmp(v, "10")) order10 = 1; else if (strcmp(v, "01")) usage(); }
        else if (!strcmp(a, "--pps-out") && more) { const char *v = argv[++i]; if (!strcmp(v, "on")) pps_out = 1; else if (!strcmp(v, "off")) pps_out = 0; else usage(); }
        else if (!strcmp(a, "--freq") && more) freq = atof(argv[++i]);
        else if (!strcmp(a, "--rate") && more) rate = atof(argv[++i]);
        else if (!strcmp(a, "--samps") && more) samps = atoi(argv[++i]);
        else if (!strcmp(a, "--gain") && more) gain = atof(argv[++i]);
        else if (a[0] == '-' && a[1] == '-') usage();
        else if (np < 4) pos[np++] = a; else usage();
    }
    int want = !strcmp(mode, "probe") || !strcmp(mode, "outputs-off") || !strcmp(mode, "follower-first") || !strcmp(mode, "compound") ? 1
             : !strcmp(mode, "separate") || !strcmp(mode, "reopen-source") || !strcmp(mode, "align") ? 2 : !strcmp(mode, "selftest") ? 0 : -1;
    if (want < 0 || np != want || samps < 256 || rate <= 0 || g_limit < 1) usage();
    if (!strcmp(mode, "selftest")) return selftest();

    int al = !strcmp(mode, "align");
    g_stage = !strcmp(mode, "probe") ? "S0" : !strcmp(mode, "reopen-source") ? "S5a" : !strcmp(mode, "outputs-off") ? "S5b-off" : !strcmp(mode, "follower-first") ? "S5b"
            : !strcmp(mode, "compound") ? "S2" : al ? "S6" : pps_out == 1 ? "S3d" : pps_out == 0 ? "S4b" : order10 ? "S4c" : "S3";
    pthread_t wd; if (pthread_create(&wd, NULL, watchdog, NULL) || pthread_detach(wd)) die_msg("pthread_create", "watchdog failed");

    if (!strcmp(mode, "probe")) {
        uhd_usrp_handle h; double ms = open_dev(pos[0], &h, 1, NULL, 0); dev_t_ d = {h, 0}; g_h = h;
        uhd_usrp_rx_info_t ri; double mcr; U(uhd_usrp_get_rx_info(h, 0, &ri)); U(uhd_usrp_get_master_clock_rate(h, 0, &mcr));
        char ser[80]; snprintf(ser, sizeof ser, "%s", ri.mboard_serial); uhd_usrp_rx_info_free(&ri);
        int lk = ref_locked(d); int64_t tn = tnow(d), lp = lpps(d);
        LINE(0, "S0 run=%d dev=%s open_ms=%.0f serial=%s mcr=%.0f ref_locked=%d time_now=%lld last_pps=%lld verdict=PASS\n", g_run, pos[0], ms, ser, mcr, lk, (long long)tn, (long long)lp);
        uhd_usrp_free(&h); return 0;
    }
    if (!strcmp(mode, "separate") || al) {
        sep_t r; separate(pos[0], pos[1], order10, pps_out, &r);
        if (!al) { int code = print_sep(g_stage, &r); sep_close(&r); return code; }
        if (r.res != 1) { rst(r.why); LINE(r.res == 2 ? 3 : 1, "S6 run=%d rate=%.0f samps=%d verdict=%s reason=\"%s\"\n", g_run, rate, samps, r.res == 2 ? "INCONCLUSIVE" : "REFUSED", r.why); sep_close(&r); return r.res == 2 ? 3 : 1; }
        uhd_rx_streamer_handle s[2]; uhd_rx_metadata_handle md; float _Complex *buf[2]; double _Complex *c[2];
        for (int i = 0; i < 2; i++) { rx_setup(r.d[i].h, freq, rate, gain, &s[i]); buf[i] = malloc(sizeof(float _Complex) * samps); c[i] = malloc(sizeof(double _Complex) * samps); }
        g_h = NULL; U(uhd_rx_metadata_make(&md));
        int64_t t = tnow(r.d[0]) + TICK_HZ / 2;
        uhd_stream_cmd_t cmd = {UHD_STREAM_MODE_NUM_SAMPS_AND_DONE, (size_t)samps, false, t / TICK_HZ, (double)(t % TICK_HZ) / (double)TICK_HZ};
        for (int i = 0; i < 2; i++) { g_h = r.d[i].h; U(uhd_rx_streamer_issue_stream_cmd(s[i], &cmd)); }
        for (int i = 0; i < 2; i++) { rx_grab(s[i], md, buf[i], samps); for (int k = 0; k < samps; k++) c[i][k] = buf[i][k]; }
        double p, sp; int lag = xcorr_lag(c[0], c[1], samps, 64, &p, &sp);
        LINE(0, "S6 run=%d rate=%.0f samps=%d t_start=%lld lag_samples=%d lag_ticks=%lld peak_db=%.1f second_peak_db=%.1f verdict=INFO\n",
               g_run, rate, samps, (long long)t, lag, llround(lag * (double)TICK_HZ / rate), p, sp);
        for (int i = 0; i < 2; i++) { uhd_rx_streamer_free(&s[i]); free(buf[i]); free(c[i]); }
        uhd_rx_metadata_free(&md); sep_close(&r); return 0;
    }
    if (!strcmp(mode, "reopen-source")) {
        uhd_usrp_handle h0, h1; open_dev(pos[1], &h1, 1, NULL, 0); dev_t_ d1 = {h1, 0};
        double lock1 = switch_follower(d1);
        mon_t m; memset(&m, 0, sizeof m); m.d = d1; atomic_init(&m.stop, 0); atomic_init(&m.samples, 0); atomic_init(&m.falses, 0);
        atomic_init(&m.first_false, -1); atomic_init(&m.relock, -1); m.t_ref = now_ms();
        pthread_t th; if (pthread_create(&th, NULL, monitor, &m)) die_msg("pthread_create", "failed");
        char oe[8]; double o0 = open_dev(pos[0], &h0, 0, oe, sizeof oe);
        if (!h0) { atomic_store(&m.stop, 1); pthread_join(th, NULL); g_h = NULL; die_msg("uhd_usrp_make", "source open failed"); }
        sleep_ms(500);
        for (int i = 0; i < 200 && atomic_load(&m.falses) > 0 && atomic_load(&m.relock) < 0; i++) sleep_ms(50);
        atomic_store(&m.stop, 1); pthread_join(th, NULL);
        LINE(0, "S5a run=%d lock1_before=%.0f open0_ms=%.0f samples=%d false_samples=%d first_false_ms=%d relock_ms=%d verdict=INFO\n", g_run, lock1, o0,
               atomic_load(&m.samples), atomic_load(&m.falses), atomic_load(&m.first_false), atomic_load(&m.relock));
        uhd_usrp_free(&h0); uhd_usrp_free(&h1); return 0;
    }
    if (!strcmp(mode, "outputs-off")) {
        uhd_usrp_handle h; open_dev(pos[0], &h, 1, NULL, 0); g_h = h;
        U(uhd_usrp_set_clock_source_out(h, false, 0)); U(uhd_usrp_set_time_source_out(h, false, 0)); uhd_usrp_free(&h);
        LINE(0, "S5b-off run=%d verdict=INFO\n", g_run); return 0;
    }
    if (!strcmp(mode, "follower-first")) {
        char args[400], err[400] = ""; snprintf(args, sizeof args, "%s,clock_source=external,time_source=external", pos[0]);
        uhd_usrp_handle h; open_dev(args, &h, 0, err, sizeof err); rst(err);
        if (!h) { LINE(0, "S5b run=%d open=error error=\"%s\" ref_locked=na verdict=INFO\n", g_run, err); return 0; }
        dev_t_ d = {h, 0}; LINE(0, "S5b run=%d open=ok error=\"\" ref_locked=%d verdict=INFO\n", g_run, ref_locked(d)); uhd_usrp_free(&h); return 0;
    }
    /* compound */
    uhd_usrp_handle h; double ms = open_dev(pos[0], &h, 1, NULL, 0); g_h = h;
    size_t nmb; U(uhd_usrp_get_num_mboards(h, &nmb)); if (nmb < 2 || nmb > MAXDEV) die_msg("get_num_mboards", "need 2..8 mboards");
    dev_t_ d[MAXDEV]; for (size_t i = 0; i < nmb; i++) { d[i].h = h; d[i].mb = i; }
    double lock1 = 0; if (!strstr(pos[0], "clock_source")) lock1 = switch_follower(d[1]);
    bool sync = false; int64_t lv[MAXDEV], lk[MAXDEV]; edges_t ed;
    U(uhd_usrp_set_time_unknown_pps(h, 0, 0.0)); U(uhd_usrp_get_time_synchronized(h, &sync));
    for (size_t i = 0; i < nmb; i++) { lv[i] = lpps(d[i]); lk[i] = ref_locked(d[i]); }
    incr(d, (int)nmb, 3, &ed);
    char sa[120], sb[120], sc[120], sd[120], se[120], args[300]; snprintf(args, sizeof args, "%s", pos[0]); rst(args);
    int64_t i0[8], i1[8], mv[MAXDEV]; int n0 = edge_diffs(&ed, 0, i0), n1 = edge_diffs(&ed, 1, i1);
    for (size_t i = 0; i < nmb; i++) mv[i] = missed_of(&ed, (int)i, 3);
    LINE(0, "S2 run=%d args=\"%s\" open_ms=%.0f lock1_ms=%.0f ref_locked=%s synchronized=%d last_pps=%s incr0=%s incr1=%s missed=%s verdict=INFO\n", g_run, args, ms, lock1,
           lst(sa, sizeof sa, lk, (int)nmb), (int)sync, lst(sb, sizeof sb, lv, (int)nmb), lst(sc, sizeof sc, i0, n0), lst(sd, sizeof sd, i1, n1), lst(se, sizeof se, mv, (int)nmb));
    uhd_usrp_free(&h);
    return 0;
}