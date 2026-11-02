//! The resumable device model `D` (note 45 §3): the parts of FakeDevice that the receive
//! slice's cases reach, each function naming the FakeDevice function it mirrors. Not modelled
//! here, left to E2: scripted faults, the per-block delivery of `rx_recv` and the transmit side. A
//! timed start below `now` is a missed start, and so is a start promoted from behind a stopped tail
//! at the next recv entry, as FakeDevice makes them.

use std::collections::{BTreeMap, VecDeque};

/// One chain's receive frequency state: the timed commands it holds (`fifo`), the instants it
/// has taken (`log`), the sample flows that pass it, and the tick it has looked at.
#[derive(Clone, Debug)]
pub(crate) struct Dsp {
    /// `(instant, value id)`: the timed writes the chain has not taken yet, in order.
    pub(crate) fifo: VecDeque<(i64, u64)>,
    /// `(tick, value id)`: the writes the chain has applied, with the sample that applied each.
    pub(crate) log: Vec<(i64, u64)>,
    /// `(start, end, logical)`: samples pass from `start` (`end` none while running).
    pub(crate) flows: Vec<(i64, Option<i64>, i64)>,
    pub(crate) cursor: i64,
}

impl Dsp {
    pub(crate) fn new() -> Dsp {
        Dsp { fifo: VecDeque::new(), log: Vec::new(), flows: Vec::new(), cursor: i64::MIN }
    }
}

/// One open receive streamer: its channels, its next start, its stopped tail's end, a start
/// waiting behind that tail, a missed start (`late`) and the sample ticks it has buffered.
#[derive(Clone, Debug)]
pub(crate) struct Streamer {
    pub(crate) channels: Vec<usize>,
    pub(crate) next: Option<i64>,
    pub(crate) end: Option<i64>,
    pub(crate) then: Option<i64>,
    pub(crate) late: Option<i64>,
    pub(crate) buffered: VecDeque<i64>,
}

impl Streamer {
    /// `rx_recv`'s tail transition (device.rs:1627–1640), run at a recv's entry ([`Device::enter_recv`]): once the
    /// stopped stream's tail is delivered (every sample before `end` produced and received), the start waiting behind
    /// it becomes the next start, or a missed one when its instant has passed.
    fn promote(&mut self, now: i64) {
        let delivered = self.buffered.is_empty() && self.end.is_some_and(|end| now >= end || self.next.is_none_or(|next| next >= end));
        if !delivered {
            return;
        }
        self.end = None;
        self.next = None;
        if let Some(at) = self.then.take() {
            if at < now {
                self.late = Some(at);
            } else {
                self.next = Some(at);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Apply {
    Accepted { effective: i64 },
    Blocked,
    Refused,
}

#[derive(Clone, Debug)]
pub(crate) struct Device {
    pub(crate) t: i64,
    /// The effective instants of the commands in the in-order queue.
    pub(crate) queue: VecDeque<i64>,
    pub(crate) queue_cap: usize,
    pub(crate) fifo_cap: usize,
    pub(crate) last_command: i64,
    pub(crate) dsp: BTreeMap<usize, Dsp>,
    pub(crate) streamers: BTreeMap<usize, Streamer>,
    /// The device is gone: no sample is produced. The executors (world.rs) make every call return lost.
    pub(crate) lost: bool,
    /// The tick from which the device produces no sample: time goes on, but nothing is produced, settled or applied.
    pub(crate) silent_from: i64,
}

impl Device {
    /// `queue_cap` is the command queue's depth; `fifo_cap` is the RX DSP FIFO's (`DSP_FIFO`,
    /// G17, which E2 checks against the constant).
    pub(crate) fn new(queue_cap: usize, fifo_cap: usize) -> Device {
        Device { t: 0, queue: VecDeque::new(), queue_cap, fifo_cap, last_command: i64::MIN, dsp: BTreeMap::new(), streamers: BTreeMap::new(), lost: false, silent_from: i64::MAX }
    }

    /// The last tick whose sample a chain can apply a write at: the present, or the last sample produced once the device
    /// is silent (no write is applied at or after `silent_from`). The command queue is time-driven and goes on leaving after
    /// it: a write accepted in the silent time would otherwise block the settings executor for good.
    fn last_tick(&self) -> i64 {
        self.t.min(self.silent_from.saturating_sub(1))
    }

    /// `settle` (device.rs:1028): the commands now due leave the queue, then the chains drain.
    pub(crate) fn settle(&mut self) {
        while self.queue.front().is_some_and(|at| *at <= self.t) {
            self.queue.pop_front();
        }
        self.drain();
    }

    /// `drain` (device.rs:1047): each chain takes the head of its FIFO, and only the head (G6),
    /// at the first tick at or after its instant that a sample of a flow passes, not later
    /// than now.
    fn drain(&mut self) {
        let now = self.last_tick();
        for chain in self.dsp.values_mut() {
            while let Some(&(instant, id)) = chain.fifo.front() {
                let want = instant.max(chain.cursor);
                let mut at = None;
                for &(start, end, logical) in &chain.flows {
                    let logical_tick = want.max(logical);
                    let tick = start.saturating_add(logical_tick.saturating_sub(logical));
                    if end.is_some_and(|end| tick >= end) {
                        continue;
                    }
                    if tick <= now {
                        at = Some((logical_tick, tick));
                    }
                    break;
                }
                let Some((logical_tick, tick)) = at else { break };
                chain.fifo.pop_front();
                chain.log.push((tick, id));
                chain.cursor = logical_tick.saturating_add(1);
            }
            chain.flows.retain(|(_, end, _)| end.is_none_or(|end| end > now));
        }
    }

    /// One sample (`Tick`): the running streamers buffer the sample with timestamp `t`, the device
    /// settles at `t`, and then time moves on, so the first tick produces sample 0 (E1-62).
    pub(crate) fn tick(&mut self) {
        let now = self.t;
        if now >= self.silent_from {
            self.t += 1;
            return;
        }
        for streamer in self.streamers.values_mut() {
            if !self.lost && streamer.next.is_some_and(|next| next <= now) && streamer.end.is_none_or(|end| now < end) {
                streamer.buffered.push_back(now);
            }
        }
        self.settle();
        self.t += 1;
    }

    /// Whether a timed command on `channel` would be blocked (`Some(Blocked)`) or refused
    /// (`Some(Refused)`) if it were applied now: the checks of [`Device::apply_timed`], on a
    /// settled copy, so the probe never changes the device.
    pub(crate) fn room(&self, channel: usize) -> Option<Apply> {
        let mut probe = self.clone();
        probe.settle();
        if probe.queue.len() >= probe.queue_cap {
            return Some(Apply::Blocked);
        }
        if probe.dsp.get(&channel).is_some_and(|chain| chain.fifo.len() >= probe.fifo_cap) {
            return Some(Apply::Refused);
        }
        None
    }

    /// `apply` (device.rs:1407), timed branch. A full queue blocks the caller. The effective
    /// instant is `max(at, last_command, now)` (device.rs:1433). An RX DSP FIFO that holds as
    /// many as its capacity refuses (the ACK timeout) before any state changes.
    pub(crate) fn apply_timed(&mut self, channel: usize, id: u64, at: i64) -> Apply {
        self.settle();
        if let Some(refusal) = self.room(channel) {
            return refusal;
        }
        let effective = at.max(self.last_command).max(self.t);
        let chain = self.dsp.entry(channel).or_insert_with(Dsp::new);
        chain.fifo.push_back((effective, id));
        self.last_command = effective;
        self.queue.push_back(effective);
        Apply::Accepted { effective }
    }

    /// `start_rx` (device.rs:1169), for a timed start. An instant below `now` is a missed start (device.rs:1195): no flow
    /// opens and the next recv returns `LateCommand`. While a stopped stream's tail is still being delivered the start
    /// waits in `then` and the tail is kept (device.rs:1196–1199). Otherwise the start is the stream's next start.
    pub(crate) fn start(&mut self, stream: usize, at: i64) {
        self.settle();
        let channels = self.streamers[&stream].channels.clone();
        if at < self.t {
            self.streamers.get_mut(&stream).expect("the stream is open").late = Some(at);
            return;
        }
        open_flow(&mut self.dsp, &channels, at);
        let streamer = self.streamers.get_mut(&stream).expect("the stream is open");
        if streamer.end.is_some() {
            streamer.then = Some(at);
        } else {
            streamer.next = Some(at);
        }
    }

    /// `rx_start_now` (device.rs:1169, `at = now`): never late, whatever the lag of the caller.
    pub(crate) fn start_now(&mut self, stream: usize) {
        self.start(stream, self.t);
    }

    /// `open` (device.rs `open_stream`): a streamer exists on `channels`, not yet started.
    pub(crate) fn open(&mut self, stream: usize, channels: Vec<usize>) {
        self.streamers.insert(stream, Streamer { channels, next: None, end: None, then: None, late: None, buffered: VecDeque::new() });
    }

    /// `close`: the streamer is gone.
    pub(crate) fn close(&mut self, stream: usize) {
        self.streamers.remove(&stream);
    }

    /// `rx_stop` (device.rs:1574): samples stop passing the chains now. A stream whose next
    /// start is in the past keeps its tail up to now (device.rs:1591–1595); otherwise it ends.
    pub(crate) fn stop(&mut self, stream: usize) {
        self.settle();
        let now = self.t;
        let channels = self.streamers[&stream].channels.clone();
        close_flow(&mut self.dsp, &channels, now);
        let streamer = self.streamers.get_mut(&stream).expect("the stream is open");
        streamer.then = None;
        match streamer.next {
            Some(next) if next < now => streamer.end = Some(streamer.end.map_or(now, |end| end.min(now))),
            _ => {
                streamer.next = None;
                streamer.end = None;
            }
        }
    }

    /// A recv's entry (device.rs:1627–1640): the stopped stream's tail, if every sample of it has been received, gives
    /// way to the start waiting behind it. Only a recv entry does this, never a tick or the recv that returned the tail.
    pub(crate) fn enter_recv(&mut self, stream: usize) {
        let now = self.t;
        if let Some(streamer) = self.streamers.get_mut(&stream) {
            streamer.promote(now);
        }
    }

    /// `rx_recv` (device.rs:1604), for the length only: the next `n` buffered sample ticks, in
    /// order. Generated samples are never returned (G8).
    pub(crate) fn recv(&mut self, stream: usize, n: usize) -> Vec<i64> {
        let Some(streamer) = self.streamers.get_mut(&stream) else { return Vec::new() };
        let mut out = Vec::new();
        while out.len() < n {
            match streamer.buffered.pop_front() {
                Some(tick) => out.push(tick),
                None => break,
            }
        }
        out
    }

    /// A missed start is due: `rx_recv` returns `LateCommand` at once (device.rs:1642–1647).
    pub(crate) fn late_due(&self, stream: usize) -> bool {
        self.streamers.get(&stream).is_some_and(|streamer| streamer.late.is_some_and(|at| self.t >= at))
    }

    /// `rx_recv` returning `LateCommand`: the missed start is consumed.
    pub(crate) fn take_late(&mut self, stream: usize) -> bool {
        let due = self.late_due(stream);
        if due {
            self.streamers.get_mut(&stream).expect("the stream is open").late = None;
        }
        due
    }
}

/// `open_flow` (device.rs:558): samples start passing the chains of `channels` at `at`.
fn open_flow(dsp: &mut BTreeMap<usize, Dsp>, channels: &[usize], at: i64) {
    for &channel in channels {
        dsp.entry(channel).or_insert_with(Dsp::new).flows.push((at, None, at));
    }
}

/// `close_flow` (device.rs:568): samples stop passing the chains of `channels` at `now`; a flow
/// that had not begun never does.
fn close_flow(dsp: &mut BTreeMap<usize, Dsp>, channels: &[usize], now: i64) {
    for channel in channels {
        if let Some(chain) = dsp.get_mut(channel) {
            chain.flows = std::mem::take(&mut chain.flows)
                .into_iter()
                .filter_map(|(start, end, logical)| match end {
                    None if start >= now => None,
                    None => Some((start, Some(now), logical)),
                    end => Some((start, end, logical)),
                })
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn streamer(channels: Vec<usize>, next: Option<i64>, end: Option<i64>) -> Streamer {
        Streamer { channels, next, end, then: None, late: None, buffered: VecDeque::new() }
    }

    #[test]
    fn u15a_full_queue_blocks_then_accepts() {
        let mut dev = Device::new(1, 32);
        assert_eq!(dev.apply_timed(0, 1, 40), Apply::Accepted { effective: 40 });
        assert_eq!(dev.apply_timed(0, 2, 8), Apply::Blocked);
        dev.t = 40;
        dev.settle();
        assert_eq!(dev.apply_timed(0, 2, 8), Apply::Accepted { effective: 40 });
    }

    #[test]
    fn u15b_dsp_fifo_full_refuses_rx_write() {
        let mut dev = Device::new(64, 32);
        for _ in 0..31 {
            assert!(matches!(dev.apply_timed(0, 1, 1000), Apply::Accepted { .. }));
        }
        assert!(matches!(dev.apply_timed(0, 1, 1000), Apply::Accepted { .. }));
        assert_eq!(dev.apply_timed(0, 1, 1000), Apply::Refused);
    }

    #[test]
    fn u15c_stop_with_tail_keeps_end() {
        let mut dev = Device::new(1, 32);
        dev.t = 10;
        dev.streamers.insert(0, streamer(vec![0], Some(5), None));
        dev.stop(0);
        assert_eq!(dev.streamers[&0].end, Some(10));
    }

    #[test]
    fn u15d_start_behind_stopped_tail_sets_then() {
        let mut dev = Device::new(1, 32);
        dev.t = 20;
        dev.streamers.insert(0, streamer(vec![0], Some(30), Some(40)));
        dev.start(0, 50);
        let streamer = &dev.streamers[&0];
        assert_eq!(streamer.then, Some(50));
        assert_eq!((streamer.end, streamer.next), (Some(40), Some(30)));
    }

    /// Sol E1 finding 4: a start queued behind a stopped tail runs once the tail is delivered. The stream produces 0..3,
    /// stops at 4, and a start at 8 waits; the tail is received first, then the next segment's samples arrive from 8
    /// (nothing between), and the start's DSP flow and its samples agree.
    #[test]
    fn u15g_start_behind_stopped_tail_delivers_the_next_segment() {
        let mut dev = Device::new(1, 32);
        dev.open(0, vec![0]);
        dev.start(0, 0);
        for _ in 0..4 {
            dev.tick();
        }
        dev.stop(0);
        dev.start(0, 8);
        assert_eq!(dev.streamers[&0].then, Some(8));
        let mut received = Vec::new();
        for _ in 4..14 {
            dev.tick();
            dev.enter_recv(0);
            received.extend(dev.recv(0, 4));
        }
        assert_eq!(received, vec![0, 1, 2, 3, 8, 9, 10, 11, 12, 13]);
        let streamer = &dev.streamers[&0];
        assert_eq!((streamer.end, streamer.then, streamer.next, streamer.late), (None, None, Some(8), None));
    }

    /// The tail's samples are not all received yet: the waiting start stays behind it, through ticks and recv entries alike.
    #[test]
    fn u15h_start_waits_while_the_tail_is_undelivered() {
        let mut dev = Device::new(1, 32);
        dev.open(0, vec![0]);
        dev.start(0, 0);
        for _ in 0..4 {
            dev.tick();
        }
        dev.stop(0);
        dev.start(0, 8);
        for _ in 4..14 {
            dev.tick();
            dev.enter_recv(0);
        }
        let streamer = &dev.streamers[&0];
        assert_eq!((streamer.end, streamer.then, streamer.buffered.len()), (Some(4), Some(8), 4), "no sample of the next start while the tail waits");
    }

    /// Sol E1 re-review P1-A: the stream stops at 4 with a start at 8 behind its tail; the tail is received at 6, the next recv
    /// enters at 10. The promotion belongs to that entry, not to a tick or the tail's recv, so the start is missed (`LateCommand`,
    /// once, no sample), and ticks 8 and 9 buffered nothing.
    #[test]
    fn u15i_start_promoted_at_the_next_recv_entry_after_its_instant_is_late() {
        let mut dev = Device::new(1, 32);
        dev.open(0, vec![0]);
        dev.start(0, 0);
        for _ in 0..4 {
            dev.tick();
        }
        dev.stop(0);
        dev.start(0, 8);
        for _ in 4..6 {
            dev.tick();
        }
        dev.enter_recv(0);
        assert_eq!(dev.recv(0, 4), vec![0, 1, 2, 3]);
        let streamer = &dev.streamers[&0];
        assert_eq!((streamer.end, streamer.then, streamer.late), (Some(4), Some(8), None), "the tail's recv does not promote");
        for _ in 6..10 {
            dev.tick();
        }
        assert!(dev.streamers[&0].buffered.is_empty(), "no sample of the next start before the entry that promotes it");
        assert!(!dev.late_due(0));
        dev.enter_recv(0);
        assert_eq!(dev.streamers[&0].late, Some(8), "the promoted start is late");
        assert!(dev.late_due(0));
        assert!(dev.take_late(0));
        assert!(!dev.late_due(0), "the missed start is reported once");
        assert!(dev.streamers[&0].buffered.is_empty(), "a missed start produces no sample");
    }

    /// Sol E1 re-review P1-B: a timed start below `now` is missed (no flow, no sample, `LateCommand` once); at `now` it is not;
    /// a start-now never is, however late the caller.
    #[test]
    fn u15j_timed_start_below_now_is_late_start_now_is_not() {
        let mut dev = Device::new(1, 32);
        dev.open(0, vec![0]);
        dev.open(1, vec![1]);
        dev.t = 10;
        dev.start(0, 8);
        assert_eq!((dev.streamers[&0].late, dev.streamers[&0].next), (Some(8), None));
        assert!(dev.dsp.get(&0).is_none_or(|chain| chain.flows.is_empty()), "a missed start opens no flow");
        dev.tick();
        assert!(dev.streamers[&0].buffered.is_empty());
        assert!(dev.late_due(0) && dev.take_late(0) && !dev.late_due(0));
        dev.start(0, 11);
        assert_eq!((dev.streamers[&0].late, dev.streamers[&0].next), (None, Some(11)));
        dev.t = 20;
        dev.start(0, 20);
        assert_eq!(dev.streamers[&0].late, None, "a start at now is not late");
        dev.start_now(1);
        assert_eq!((dev.streamers[&1].late, dev.streamers[&1].next), (None, Some(20)));
    }

    /// Sol E1 re-review P2-C: nothing is applied at or after `silent_from`, not even by the settle of a Stop at the boundary.
    #[test]
    fn u15k_no_write_is_applied_at_silent_from() {
        let mut dev = Device::new(4, 32);
        dev.silent_from = 16;
        dev.open(0, vec![0]);
        dev.start(0, 0);
        assert!(matches!(dev.apply_timed(0, 8, 15), Apply::Accepted { effective: 15 }));
        assert!(matches!(dev.apply_timed(0, 9, 16), Apply::Accepted { effective: 16 }));
        for _ in 0..16 {
            dev.tick();
        }
        assert_eq!(dev.t, 16);
        dev.stop(0);
        assert_eq!(dev.dsp[&0].log, vec![(15, 8)], "the write at silent_from is not applied");
        assert_eq!(dev.dsp[&0].fifo.len(), 1);
    }

    #[test]
    fn u15e_drain_is_head_of_line() {
        let mut dev = Device::new(1, 32);
        dev.dsp.insert(0, Dsp { fifo: VecDeque::from([(40, 1), (20, 2)]), ..Dsp::new() });
        dev.dsp.get_mut(&0).expect("the chain").flows.push((0, None, 0));
        dev.t = 19;
        for _ in 0..23 {
            dev.tick();
        }
        assert_eq!(dev.dsp[&0].log, vec![(40, 1), (41, 2)]);
    }

    #[test]
    fn u15f_recv_delivers_buffered_samples_in_order() {
        let mut dev = Device::new(1, 32);
        let mut buffered = streamer(vec![0], None, None);
        buffered.buffered = VecDeque::from([8, 9, 10, 11]);
        dev.streamers.insert(0, buffered);
        assert_eq!(dev.recv(0, 2), vec![8, 9]);
        assert_eq!(dev.recv(0, 2), vec![10, 11]);
    }
}
