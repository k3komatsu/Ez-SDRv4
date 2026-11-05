// Minimal UHD 4.10 reproduction for design note 48 S1 / issue #78 (bench 2026-11-05).
// No Ez-SDR code: multi_usrp + tx_streamer + recv_async_msg only. Transmit stays inside
// bench.md's RF conditions: 1 GHz centre, tone offset 100 kHz, 1 MS/s, TX gain 0 dB, slot A,
// TX/RX into a >= 30 dB attenuated loopback (owner confirms before every run).
//
//   case 1 "flush":  two zero samples, SOB+EOB, no time, on an idle streamer.
//                    Q: does the device report Underflow before the burst ACK, and how many?
//   case 2 "gap":    burst A (10 000 samples, timed first send, untimed continuations, two-sample
//                    untimed EOB close) then burst B (1 000 samples, timed SOB, EOB) at A's end + g
//                    samples, g in {0, 1, 2, 3, 1000}. Q: is B played (ACK at its end) or late?
//   case 3 "p70":    burst A of 2 samples (timed SOB+EOB; the Module pads a one-sample burst) at t, then burst B of 10 000 samples
//                    at t + 25 us sent in 1 000-sample chunks, (i) every chunk timed, (ii) only the
//                    first chunk timed. Q: which shape draws TIME_ERROR / late packets / underflow?
//
// Output: one line per async report ("report <case> <trial> <code> <tick>") and one summary line
// per trial. Ticks are device ticks at the master clock rate (200 MHz on X300).
//
// Build (in ezsdr-v4-dev:uhd4.10): g++ -O2 -std=c++17 tx_repro.cpp -o tx_repro -luhd -lpthread
// Run: tx_repro --args addr=192.168.47.37 [--case all|flush|gap|p70] [--trials 5]

#include <uhd/usrp/multi_usrp.hpp>
#include <uhd/types/time_spec.hpp>
#include <uhd/utils/thread.hpp>

#include <chrono>
#include <cmath>
#include <complex>
#include <cstdio>
#include <cstdlib>
#include <map>
#include <string>
#include <thread>
#include <vector>

using samp = std::complex<float>;

static double RATE = 1e6;
static const double FREQ = 1e9;      // bench.md SAFE_TX_HZ: 999.5–1000.5 MHz
static const double TONE = 100e3;
static const double MCR = 200e6;
static long long PER = 200;    // ticks per sample at RATE

struct Report {
    int code;
    long long tick;
};

static const char* code_name(int code) {
    switch (code) {
        case uhd::async_metadata_t::EVENT_CODE_BURST_ACK: return "BurstAck";
        case uhd::async_metadata_t::EVENT_CODE_UNDERFLOW: return "Underflow";
        case uhd::async_metadata_t::EVENT_CODE_SEQ_ERROR: return "SeqError";
        case uhd::async_metadata_t::EVENT_CODE_TIME_ERROR: return "TimeError";
        case uhd::async_metadata_t::EVENT_CODE_UNDERFLOW_IN_PACKET: return "UnderflowInPacket";
        case uhd::async_metadata_t::EVENT_CODE_SEQ_ERROR_IN_BURST: return "SeqErrorInBurst";
        default: return "Other";
    }
}

static long long ticks(const uhd::time_spec_t& t) { return t.to_ticks(MCR); }
static uhd::time_spec_t at_tick(long long tick) { return uhd::time_spec_t::from_ticks(tick, MCR); }

// Reads async reports until `quiet` seconds pass with none (or `total` seconds in all).
static std::vector<Report> drain(uhd::tx_streamer::sptr& tx, double quiet, double total) {
    std::vector<Report> out;
    uhd::async_metadata_t md;
    auto end = std::chrono::steady_clock::now() + std::chrono::duration<double>(total);
    while (std::chrono::steady_clock::now() < end) {
        if (!tx->recv_async_msg(md, quiet)) break;
        out.push_back({int(md.event_code), md.has_time_spec ? ticks(md.time_spec) : -1});
    }
    return out;
}

static std::vector<samp> tone(size_t n, size_t phase0 = 0) {
    std::vector<samp> v(n);
    for (size_t i = 0; i < n; i++) {
        double p = 2.0 * M_PI * TONE * double(i + phase0) / RATE;
        v[i] = samp(float(0.4 * std::cos(p)), float(0.4 * std::sin(p)));
    }
    return v;
}

static bool g_invalid = false;

// One send; returns the samples accepted. `at` < 0 means untimed.
static size_t send(uhd::tx_streamer::sptr& tx, const samp* data, size_t n, long long at, bool sob, bool eob) {
    uhd::tx_metadata_t md;
    md.start_of_burst = sob;
    md.end_of_burst = eob;
    md.has_time_spec = at >= 0;
    if (at >= 0) md.time_spec = at_tick(at);
    size_t got = tx->send(data, n, md, 1.0);
    if (got != n) {
        std::printf("INVALID short send %zu of %zu (at %lld sob %d eob %d)\n", got, n, at, int(sob), int(eob));
        g_invalid = true;
    }
    return got;
}

static void print_reports(const char* name, int trial, const std::vector<Report>& r) {
    for (auto& x : r) std::printf("report %s %d %s %lld\n", name, trial, code_name(x.code), x.tick);
}

static std::map<std::string, int> count(const std::vector<Report>& r) {
    std::map<std::string, int> c;
    for (auto& x : r) c[code_name(x.code)]++;
    return c;
}

static std::string counts(const std::vector<Report>& r) {
    std::string s;
    for (auto& [k, v] : count(r)) s += k + "=" + std::to_string(v) + " ";
    return s.empty() ? "none" : s;
}

static long long ahead(uhd::usrp::multi_usrp::sptr& u, double seconds) {
    long long now = ticks(u->get_time_now());
    long long t = now + (long long)(seconds * MCR);
    return (t / PER + 1) * PER;   // on the 1 MS/s lattice
}

// Sends `n` samples starting at device tick `start` in chunks of `chunk`, the first timed SOB;
// continuations timed only if `every_timed`; the burst closes with a separate untimed two-sample
// EOB (one held tail sample + one zero), as the Module's two-sample close does.
static void burst(uhd::tx_streamer::sptr& tx, const std::vector<samp>& w, long long start, size_t chunk, bool every_timed) {
    size_t body = w.size() - 1;   // the last sample goes in the closing send
    size_t off = 0;
    while (off < body) {
        size_t k = std::min(chunk, body - off);
        long long at = (off == 0 || every_timed) ? start + (long long)off * PER : -1;
        if (send(tx, &w[off], k, at, off == 0, false) != k) return;
        off += k;
    }
    samp close[2] = {w.back(), samp(0, 0)};
    long long at = every_timed ? start + (long long)body * PER : -1;
    send(tx, close, 2, at, false, true);
}

// The Module's T3 shape: two zero samples, SOB+EOB, untimed; true when its BURST_ACK arrives.
static bool flush(uhd::tx_streamer::sptr& tx, std::vector<Report>* out) {
    samp zeros[2] = {samp(0, 0), samp(0, 0)};
    if (send(tx, zeros, 2, -1, true, true) != 2) return false;
    auto rep = drain(tx, 0.2, 1.0);
    if (out) *out = rep;
    for (auto& r : rep) if (r.code == uhd::async_metadata_t::EVENT_CODE_BURST_ACK) return true;
    return false;
}

static int finish(uhd::tx_streamer::sptr& tx, int status) {
    std::vector<Report> rep;
    bool ok = flush(tx, &rep);
    std::printf("cleanup flush %s: %s\n", ok ? "acknowledged" : "NOT acknowledged", counts(rep).c_str());
    return ok ? status : (status ? status : 4);
}

static int acks(const std::vector<Report>& r) {
    int n = 0;
    for (auto& x : r) n += x.code == uhd::async_metadata_t::EVENT_CODE_BURST_ACK;
    return n;
}

static void ack_offsets(const char* name, const std::vector<Report>& r, long long expected_end) {
    for (auto& x : r)
        if (x.code == uhd::async_metadata_t::EVENT_CODE_BURST_ACK) std::printf("ack %s tick %lld (%+lld from the expected exclusive end %lld)\n", name, x.tick, x.tick - expected_end, expected_end);
}

int main(int argc, char** argv) {
    std::string args, which = "all";
    int trials = 5;
    for (int i = 1; i + 1 < argc; i += 2) {
        std::string k = argv[i], v = argv[i + 1];
        if (k == "--args") args = v;
        else if (k == "--case") which = v;
        else if (k == "--trials") trials = std::atoi(v.c_str());
        else if (k == "--rate") { RATE = std::atof(v.c_str()); PER = (long long)std::llround(MCR / RATE); }
    }
    if (args.empty()) { std::fprintf(stderr, "--args required\n"); return 2; }
    uhd::set_thread_priority_safe(1.0, true);

    auto u = uhd::usrp::multi_usrp::make(args);
    u->set_clock_source("internal");
    u->set_time_source("internal");
    u->set_tx_subdev_spec(uhd::usrp::subdev_spec_t("A:0"));
    u->set_tx_rate(RATE);
    u->set_tx_freq(uhd::tune_request_t(FREQ));
    u->set_tx_gain(0.0);
    u->set_tx_antenna("TX/RX");
    double f = u->get_tx_freq(), g = u->get_tx_gain(), r = u->get_tx_rate(), m = u->get_master_clock_rate();
    std::string ant = u->get_tx_antenna();
    if (!std::isfinite(f) || !std::isfinite(g) || !std::isfinite(r) || std::fabs(f - FREQ) > 500e3 || g != 0.0 || std::fabs(r - RATE) > 1 || ant != "TX/RX" || std::fabs(m - MCR) > 1) {
        std::fprintf(stderr, "RF safety / setup: freq %.0f gain %.1f rate %.0f antenna %s mcr %.0f\n", f, g, r, ant.c_str(), m);
        return 3;
    }
    u->set_time_now(uhd::time_spec_t(0.0));
    std::this_thread::sleep_for(std::chrono::milliseconds(200));
    std::printf("device %s mcr %.0f tx rate %.0f freq %.0f gain %.1f\n", u->get_mboard_name().c_str(), u->get_master_clock_rate(), r, f, g);

    uhd::stream_args_t sa("fc32", "sc16");
    sa.channels = {0};
    auto tx = u->get_tx_stream(sa);
    std::printf("max_num_samps %zu\n", tx->get_max_num_samps());
    {
        std::vector<Report> init;
        drain(tx, 0.2, 1.0);
        bool ok = flush(tx, &init);
        std::printf("initial flush %s: %s\n", ok ? "acknowledged" : "NOT acknowledged", counts(init).c_str());
        if (!ok && which != "flush") return finish(tx, 5);
    }

    try {
    if (which == "all" || which == "flush") {
        for (int t = 0; t < trials; t++) {
            drain(tx, 0.05, 0.5);
            long long before = ticks(u->get_time_now());
            samp zeros[2] = {samp(0, 0), samp(0, 0)};
            send(tx, zeros, 2, -1, true, true);
            auto rep = drain(tx, 0.2, 1.0);
            print_reports("flush", t, rep);
            std::printf("summary flush %d sent_at %lld %s\n", t, before, counts(rep).c_str());
            if (g_invalid) return finish(tx, 6);
        }
    }

    if (which == "all" || which == "gap") {
        auto a = tone(10000), b = tone(1000);
        for (long long gap : {0LL, 1LL, 2LL, 3LL, 1000LL}) {
            for (int t = 0; t < trials; t++) {
                drain(tx, 0.05, 0.5);
                long long a_start = ahead(u, 0.1);
                long long e = a_start + (long long)(a.size() + 1) * PER;   // exclusive end of the 2-sample close (tail + zero)
                long long b_start = e + gap * PER;
                burst(tx, a, a_start, 1000, false);
                send(tx, b.data(), b.size() - 0, b_start, true, true);
                auto rep = drain(tx, 0.3, 2.0);
                print_reports("gap", int(gap), rep);
                long long b_end = b_start + (long long)b.size() * PER;
                std::printf("summary gap %lld trial %d a_start %lld e %lld b_start %lld b_end %lld %s\n", gap, t, a_start, e, b_start, b_end, counts(rep).c_str());
                ack_offsets("gap-A", rep, e);
                ack_offsets("gap-B", rep, b_end);
                if (g_invalid) return finish(tx, 6);
                std::printf("acks gap %lld trial %d: %d\n", gap, t, acks(rep));
            }
        }
    }

    if (which == "all" || which == "p70") {
        auto b = tone(10000);
        for (int every : {1, 0}) {
            for (int first_len : {2}) {   // the Module pads a one-sample close to two samples (#70)
                for (int t = 0; t < trials; t++) {
                    drain(tx, 0.05, 0.5);
                    long long a_start = ahead(u, 0.1);
                    std::vector<samp> first(first_len, samp(0.4f, 0.0f));
                    send(tx, first.data(), first.size(), a_start, true, true);
                    long long b_start = a_start + 5000;   // + 25 us
                    burst(tx, b, b_start, 1000, every == 1);
                    auto rep = drain(tx, 0.3, 2.0);
                    char name[32];
                    std::snprintf(name, sizeof name, "p70-%s-%d", every ? "every" : "first", first_len);
                    print_reports(name, t, rep);
                    long long b_end = b_start + (long long)(b.size() + 1) * PER;   // + the close's padding zero
                    std::printf("summary %s trial %d a_start %lld b_start %lld b_end %lld %s\n", name, t, a_start, b_start, b_end, counts(rep).c_str());
                    ack_offsets(name, rep, b_end);
                    if (g_invalid) return finish(tx, 6);
                    std::printf("acks %s trial %d: %d\n", name, t, acks(rep));
                }
            }
        }
    }
    if (which == "len") {
        // case 4: one isolated burst of `len` samples, one SOB+EOB send, timed (now + 100 ms) or untimed.
        for (int timed : {1, 0}) {
            for (size_t len : {2, 3, 4, 5, 8}) {
                for (int t = 0; t < trials; t++) {
                    drain(tx, 0.05, 0.5);
                    auto w = tone(len);
                    long long start = timed ? ahead(u, 0.1) : -1;
                    send(tx, w.data(), w.size(), start, true, true);
                    auto rep = drain(tx, 0.3, 2.0);
                    char name[32];
                    std::snprintf(name, sizeof name, "len-%.0fMSps-%s-%zu", RATE / 1e6, timed ? "timed" : "untimed", len);
                    print_reports(name, t, rep);
                    std::printf("summary %s trial %d start %lld %s\n", name, t, start, counts(rep).c_str());
                    if (timed) ack_offsets(name, rep, start + (long long)len * PER);
                    if (g_invalid) return finish(tx, 6);
                }
            }
        }
    }
    if (which == "split") {
        // case 5: one device burst sent as two sends, like the Module's open-ended short burst: a first send of `head`
        // samples (SOB, timed now + 100 ms, no EOB) and a closing send of `tail` samples (EOB, untimed).
        for (auto [head, tail] : std::vector<std::pair<size_t, size_t>>{{1, 2}, {1, 3}, {2, 2}, {3, 2}, {2, 3}, {4, 2}}) {
            for (int t = 0; t < trials; t++) {
                drain(tx, 0.05, 0.5);
                auto w = tone(head + tail);
                long long start = ahead(u, 0.1);
                send(tx, w.data(), head, start, true, false);
                send(tx, w.data() + head, tail, -1, false, true);
                auto rep = drain(tx, 0.3, 2.0);
                char name[48];
                std::snprintf(name, sizeof name, "split-%.0fMSps-%zu+%zu", RATE / 1e6, head, tail);
                print_reports(name, t, rep);
                std::printf("summary %s trial %d start %lld %s\n", name, t, start, counts(rep).c_str());
                ack_offsets(name, rep, start + (long long)(head + tail) * PER);
                if (g_invalid) return finish(tx, 6);
            }
        }
    }
    } catch (const std::exception& ex) {
        std::printf("EXCEPTION %s\n", ex.what());
        return finish(tx, 8);
    }
    return finish(tx, 0);
}
