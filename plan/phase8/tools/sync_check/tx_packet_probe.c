/* tx_packet_probe: Phase 8 H3. Records the transmit streamer's max_num_samps (and the receive one's) per rate and channel count.
 * Creating a streamer puts nothing on the air; this program makes streamers, reads the number, frees them and exits.
 * Stream args are the UHD Module's: cpu "fc32", otw "sc16", args "", channel_list = the channels
 * (crates/ezsdr-radio-uhd/src/uhd.rs: stream_args at line 517, rx_open 970-986, tx_open 1066-1079). */
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <uhd.h>

static uhd_usrp_handle g_h = NULL;
static void die(const char *call, uhd_error e) {
    char m[400] = "";
    if (g_h) uhd_usrp_last_error(g_h, m, sizeof m); else uhd_get_last_error(m, sizeof m);
    if (!m[0]) snprintf(m, sizeof m, "uhd_error %d", (int)e);
    for (char *p = m; *p; p++) if (*p == '"' || *p == '\n' || *p == '\r') *p = '\'';
    char c[80]; snprintf(c, sizeof c, "%s", call); char *q = strchr(c, '('); if (q) *q = 0;
    printf("H3 verdict=ERROR error=\"%s: %s\"\n", c, m); fflush(stdout); exit(2);
}
#define U(call) do { uhd_error e_ = (call); if (e_) die(#call, e_); } while (0)

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: tx_packet_probe ARGS [RATE ...]   (ARGS e.g. addr=192.168.40.36; default rates 1e6 10e6 50e6 100e6 200e6)\n"); return 2; }
    double def[] = {1e6, 10e6, 50e6, 100e6, 200e6}, *rates = def; int nr = 5;
    if (argc > 2) { nr = argc - 2; rates = malloc(sizeof(double) * nr); for (int i = 0; i < nr; i++) rates[i] = atof(argv[2 + i]); }
    uhd_usrp_handle h; U(uhd_usrp_make(&h, argv[1])); g_h = h;
    size_t ntx, nrx; U(uhd_usrp_get_tx_num_channels(h, &ntx)); U(uhd_usrp_get_rx_num_channels(h, &nrx));
    for (int r = 0; r < nr; r++) {
        for (size_t nch = 1; nch <= 2; nch++) {
            if (nch > ntx || nch > nrx) { printf("H3 rate=%.0f channels=%zu skipped=device has %zu tx and %zu rx channels\n", rates[r], nch, ntx, nrx); continue; }
            for (size_t c = 0; c < nch; c++) { U(uhd_usrp_set_rx_rate(h, rates[r], c)); U(uhd_usrp_set_tx_rate(h, rates[r], c)); }
            size_t list[2] = {0, 1}; char cpu[] = "fc32", otw[] = "sc16", none[] = "";
            uhd_stream_args_t sa = {cpu, otw, none, list, (int)nch};
            uhd_tx_streamer_handle ts; uhd_rx_streamer_handle rs; size_t tn = 0, rn = 0;
            U(uhd_tx_streamer_make(&ts)); U(uhd_usrp_get_tx_stream(h, &sa, ts)); U(uhd_tx_streamer_max_num_samps(ts, &tn));
            printf("H3 rate=%.0f channels=%zu cpu=%s otw=%s max_num_samps=%zu\n", rates[r], nch, cpu, otw, tn);
            U(uhd_tx_streamer_free(&ts));
            U(uhd_rx_streamer_make(&rs)); U(uhd_usrp_get_rx_stream(h, &sa, rs)); U(uhd_rx_streamer_max_num_samps(rs, &rn));
            printf("H3rx rate=%.0f channels=%zu cpu=%s otw=%s max_num_samps=%zu\n", rates[r], nch, cpu, otw, rn);
            U(uhd_rx_streamer_free(&rs));
        }
    }
    uhd_usrp_free(&h);
    return 0;
}
