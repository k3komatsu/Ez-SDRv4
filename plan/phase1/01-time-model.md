# Phase 1 spec 01 — Time model

| Field | Value |
|---|---|
| Status | Draft for Gate A. Normative for `ezsdr-kernel::time` once accepted. |
| Scope | Kernel time primitives: rational tick rates, clock domains and their registry, TimePoint and Duration, the two deadline kinds, ClockRelation, the TimeAuthority interface, SampleClock lifecycle, and the time records the Manifest carries. |
| Not in scope | The Simulation Engine (Phase 2); TimingEnvelope field values (Radio Model, Phase 2); how a Provider measures its device clock against the host (Phase 7); conversions from UHD or SoapySDR representations, which are Provider boundary code. |
| Vision § covered | §15 in full; §19 "Two kinds of deadline"; §24; §27's sample-rate rule; §49 for `ClockDomainId`; §50's epoch ↔ UTC record (shape only); §58 #2 and #3 (interface only); §61's capture-at-sample-index consequence. |
| Audit §14.1 items | 2 in full (TimePoint representation, epoch, TimeAuthority, step-driven simulation: F2, F18); 11 in part (the deadline kinds: F8); 10 for the records defined here. |
| Re-review | R6 (a sample-rate change starts a new SampleClock), R9 (time naming), R12 (language-neutral shapes). |
| Modal verbs | "must" and "must not" are the normative verbs (OV-4a). "Should" does not appear inside a rule; where a rule needs a default that is not binding, it says so in explanatory text. |

---

## 1. Purpose

Every Module contract in Ez-SDR v4 carries a timestamp, and Vision invariant 32 fixes its representation: integer ticks at a rational rate, in a named ClockDomain, never floating-point seconds. This document turns that sentence into types, an exactness rule, and an interface for "now" and "wait until".

Three things must be true when Phase 2 writes MockRadio:

1. A block's time is a sample index in a domain whose relation to the device clock is exact, so that "capture at sample 4 096" and "transmit at device time T" are the same kind of statement.
2. Two timestamps that cannot be compared exactly cannot be compared at all, by construction rather than by convention.
3. Virtual time and device time are the same type behind the same interface, so that a Processor written against one runs against the other (Vision §15).

## 2. Evidence

- **Field representations.** UHD's `time_spec_t` is `int64` seconds plus a `double` fraction; srsRAN carries `uint64` sample ticks; SoapySDR uses `int64` nanoseconds. Three integer-leaning conventions, none of them floating seconds end to end. Verified in the audit, Finding 2 and Appendix B.
- **Why exactness matters.** On an X310 both 20 Msps and 25 Msps derive from one 200 MHz master clock, by decimation 10 and 8. Deciding whether a sample instant of one stream is also a sample instant of the other is a divisibility question, and floating point cannot answer it: of the 40 001 indices below 200 000 that do land on a shared sample instant, `floor(n / 25e6 * 20e6)` misplaces 2 208, the first at n = 105, where the product lands at 83.999… instead of 84. This corrects the framing in audit Finding 2, which attributes the problem to magnitude ("rounding at 10^12 samples"); the ulp of 50 000 seconds is about 7 ps, far finer than a 5 ns tick. The mechanism is inexact products under `floor` and `==`, not the size of the numbers. The conclusion is unchanged.
- **Arbitrary epochs.** A USRP's t = 0 is whatever PPS synchronisation set it to, and `set_time_unknown_pps` can take up to two seconds. Hence Vision §15 and §50 require every Run to record an epoch ↔ UTC relation with its uncertainty.
- **v3 lost time entirely.** `v3/cpp/uhd_usrp/multiusrp.cpp:698-702` returns only the sample count from `recv`, discarding both `md.time_spec` and `md.error_code`. The RFNoC receive path is slightly better and still loses the time: it prints the error code and restarts on a late command (`v3/cpp/uhd_usrp/uhd_rfnoc.cpp:449-465`) but reads no timestamp. No receive `time_spec` is read anywhere in `v3/cpp/`, so v3's received data carried no time at all. `v3/client/ezsdr.py:115-119` converted float seconds to nanoseconds in the client (`onTime`).
- **v3's `alignSize` is the requirement in disguise.** `v3/source/controller/cyclicrx.d:84-145` only serviced a capture request when every channel's `alignSize`-sized buffer was full, so a capture could start only at multiples of that buffer length (default 4 096, lines 228 and 449). Users set it to the transmitted waveform length to make the capture phase deterministic and to detect one-sample drifts by hand. Under this model a SampleClock tick *is* a sample index, so Vision §61's `capture(n, at: TimePoint | sample_index)` needs no separate mechanism.

## 3. Model

There are two kinds of domain and exactly two conversion paths.

```text
Root domains (one per timekeeper)              Derived domains (SampleClocks and their decimations)
  utc            1 GHz, epoch 1970               rx0.clk#7   root_ticks_per_tick 10/1, origin 1 000 000 003
  host.monotonic 1 GHz, arbitrary epoch          tx0.clk#8   root_ticks_per_tick  8/1, origin 1 000 000 000
  dev:x310#3     200 MHz, arbitrary epoch        proc.dd#9   root_ticks_per_tick 40/1  (a decimate-by-4 of #7)
  virtual#4      engine-chosen, arbitrary

same root        → integer arithmetic → Exact, or Inexact with a floor and a remainder
different roots  → ClockRelation      → UncertainTimePoint
anything else    → refused
```

A Derived domain names its Root directly; there are no chains. One tick of a SampleClock is one sample of that stream, so a `TimePoint` in a SampleClock is a sample index relative to the domain's origin.

## 4. Types

Language-neutral shapes (normative):

```text
NodeId              unsigned 32-bit; LOCAL = 0, the only value in v4.0 (§49)
ClockDomainId       { node: NodeId, local: unsigned 32-bit }
                    reserved local ids: 0 = utc, 1 = host.monotonic (both Root, tick_rate 1 000 000 000 / 1)
Rational            { num: unsigned 64-bit, den: unsigned 64-bit }   num > 0, den > 0, gcd(num, den) = 1
EpochRef            Utc1970 | Arbitrary { set_by: string }           e.g. "uhd.set_time_unknown_pps", "sim.run_start"
ClockDomain         { id: ClockDomainId,
                      kind: Root    { tick_rate: Rational, epoch: EpochRef }
                          | Derived { root: ClockDomainId, root_ticks_per_tick: Rational,
                                      origin: signed 64-bit (root ticks) },
                      ended_at: optional TimePoint (in the root domain) }
TimePoint           { domain: ClockDomainId, ticks: signed 64-bit }
Duration            { domain: ClockDomainId, ticks: signed 64-bit }
Converted           Exact { point: TimePoint }
                  | Inexact { floor: TimePoint, remainder: Rational }
                    a reduced Rational (TM-2) with 0 < num < den: the fraction of one target tick left over
ClockRelation       { source: ClockDomainId, target: ClockDomainId,
                      measured_at: TimePoint (source),
                      offset: TimePoint (target),        the image of measured_at in the target domain
                      drift: float64,                    dimensionless: target seconds per source second, minus 1
                      drift_uncertainty: float64,        dimensionless, ≥ 0: the one-sided error bound on drift
                      uncertainty: Duration (target),    the bound at measured_at, before drift error accumulates
                      method: string (namespaced),
                      valid: { from: TimePoint (source), to: optional TimePoint (source) } }
UncertainTimePoint  { nominal: TimePoint, uncertainty: Duration }            both in the same domain
RelativeBudget      { duration: Duration }               the domain must be host.monotonic (TM-15)
AbsoluteDeadline    { time_point: TimePoint }
SampleClockRecord   { stream: ResourceId, domain: ClockDomainId, root: ClockDomainId,
                      root_ticks_per_tick: Rational, origin: TimePoint (root),
                      ended_at: optional TimePoint (root), nominal_rate: Rational }
TimeError           DomainMismatch { expected, found } | Unrelated { a, b }
                  | Inexact { floor }                the remainder lives on Converted, where callers actually use it
                  | Overflow
                  | UnknownDomain { id } | DuplicateDomain { id } | NotGoverned { id }
                  | OutsideValidity { at } | InvalidRational | LimitExceeded
                  | InPast { now, requested } | Stopped
```

Illustrative Rust sketch, not normative:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub struct ClockDomainId { pub node: u32, pub local: u32 }
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub struct Rational { num: u64, den: u64 } // ctor normalises
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub struct TimePoint { pub domain: ClockDomainId, pub ticks: i64 }
// deliberately no PartialOrd on TimePoint or Duration
impl TimePoint {
    pub fn try_cmp(self, other: TimePoint) -> Result<core::cmp::Ordering, TimeError>;
    pub fn checked_add(self, d: Duration) -> Result<TimePoint, TimeError>;
    pub fn checked_sub(self, other: TimePoint) -> Result<Duration, TimeError>;
    pub fn ticks_in(self, domain: ClockDomainId) -> Result<i64, TimeError>;   // one id check, then raw i64
}
pub struct ExactConversion { from: ClockDomainId, to: ClockDomainId, a: i128, n: u128, d: u128 }
impl ExactConversion {
    pub fn apply(&self, t: TimePoint) -> Result<Converted, TimeError>;
    pub fn try_exact(&self, t: TimePoint) -> Result<TimePoint, TimeError>;
}
pub struct ClockRegistry { /* RwLock<BTreeMap<ClockDomainId, ClockDomain>>, AtomicU32 next_local */ }
pub trait TimeAuthority: Send + Sync {
    fn now(&self, domain: ClockDomainId) -> Result<TimePoint, TimeError>;
    fn wait_until(&self, t: TimePoint) -> Result<(), TimeError>;
    fn schedule(&self, t: TimePoint, f: Box<dyn FnOnce(TimePoint) + Send>) -> Result<ScheduleHandle, TimeError>;
    fn cancel(&self, h: ScheduleHandle) -> bool;
}
pub struct ManualTimeAuthority { /* root id, Mutex<{now, BinaryHeap<(t, seq, f)>}>, Condvar, Arc<ClockRegistry> */ }
impl ManualTimeAuthority { pub fn advance_to(&self, t: TimePoint) -> Result<usize, TimeError>; }
```

## 5. Normative rules

### Representation and arithmetic

- **TM-1** A `TimePoint` is a pair of a `ClockDomainId` and a signed 64-bit tick count. No Kernel type stores a time or a duration as floating-point seconds. (Vision §15, invariant 32.) *Checked: the type definitions and `kernel_surface`, which fails on a public time field of floating-point type.*
- **TM-2** A `Rational` has `num > 0` and `den > 0`, is stored reduced by the greatest common divisor, and compares structurally. Constructing one with a zero component fails with `InvalidRational`. Multiplication, division and comparison are evaluated with 128-bit intermediates; a reduced result whose numerator or denominator exceeds 64 bits fails with `Overflow`. Nothing wraps, saturates or panics.
- **TM-3** A `ClockDomain` is either `Root`, carrying a tick rate and an epoch reference, or `Derived`, carrying its root's id, the exact number of root ticks per one of its own ticks, and an origin expressed in root ticks. A `Derived` domain names a `Root` directly: registration refuses a `Derived` domain whose root is unknown or is itself `Derived`, and refuses with `LimitExceeded` any `root_ticks_per_tick`, or any `Root`'s `tick_rate`, whose numerator or denominator exceeds 2^31. The cap on a `Root`'s rate is what bounds TM-21's cross-product: without it a rate of 2^40 / (2^40 − 1) is legal under TM-2 and the product reaches 2^190, so a wrapped comparison in a release build would answer "on time" for a late burst.
- **TM-4** Two domains are **exactly related** if and only if they have the same root, where a `Root` is its own root. For such a pair the conversion is
  `t_to = ((o_from − o_to) · d_from + t_from · n_from) · d_to / (d_from · n_to)`,
  evaluated with checked 128-bit integers, where `n`/`d` are the numerator and denominator of `root_ticks_per_tick` and `o` the origin (a `Root` has `o = 0`, `n = d = 1`). The result is `Exact` when the division leaves no remainder, and otherwise `Inexact`, whose `floor` is the largest tick at or before the true instant and whose `remainder` is the leftover fraction of one target tick. `try_exact` returns `Inexact` as an error.
- **TM-5** Converting a `TimePoint` between domains with different roots is possible only through a `ClockRelation` whose source and target match, and yields an `UncertainTimePoint` (TM-14). There is no other path. (Vision §15.)
- **TM-6** `TimePoint`s compare only within one domain. `try_cmp` and `checked_sub` on different domains fail with `DomainMismatch`. The type exposes no ordering operators; the only access to the raw integer is `ticks_in(domain)`, which fails on mismatch.
- **TM-7** A `Duration` names a domain. Adding or subtracting a `Duration` and a `TimePoint`, and subtracting two `TimePoint`s, require equal domains and fail with `DomainMismatch` otherwise.
- **TM-8** All tick arithmetic is checked. A result outside the signed 64-bit range fails with `Overflow`.
- **TM-9** A `Duration` may be **rescaled nominally** between any two registered domains by the ratio of their nominal tick rates (`ticks · rate_to / rate_from`, checked 128-bit, exact or with a remainder). Rescaling ignores drift and produces no uncertainty. It is permitted for budgets, admission arithmetic and display, and it must not be used to place a `TimePoint` in another domain, which is TM-4 and TM-5.
- **TM-10** A `Derived` domain's nominal tick rate is its root's tick rate divided by `root_ticks_per_tick`. Because one tick of a SampleClock is one sample, `ticks` in a SampleClock is the sample index relative to the domain's origin, and Vision §61's "capture at a requested sample index" is a `TimePoint` in that domain with no additional mechanism. *Checked: the conversion tests, whose expected values are all derived from this formula.*

### Identity and the registry

- **TM-11** `ClockDomainId`s are node-qualified; in v4.0 `node` is `LOCAL`. Local ids 0 and 1 are reserved for `utc` and `host.monotonic`, both `Root` at 1 GHz with epochs `Utc1970` and `Arbitrary` respectively. Every other id is allocated by the node's `ClockRegistry`, strictly increasing, and never reused within a Run.
- **TM-12** Every domain named by a `TimePoint` that leaves its producer — in a block, an event, an action or a record — must be registered first. Registering a duplicate id fails with `DuplicateDomain`. A registered domain is immutable except that `ended_at` may be set once.
- **TM-13a** A SampleClock is a `Derived` domain owned by exactly one stream. Its id and its `root_ticks_per_tick` are allocated at `prepare` from the effective, post-coercion rate in the PrepareReport. *Checked: `ClockRegistry`, `tm_13a_*`.*
- **TM-13b** A **receive** stream's SampleClock origin **must** be the root tick of the stream's first sample, so that the first block of the stream carries `ticks = 0`, and the domain **must** be registered before the first block naming it is published. This is not a recommendation: TM-10 makes `ticks` the sample index, so Vision §61's `capture(n, at: sample_index)` resolves to a different instant on Mock and on hardware if the origin is chosen freely, which is the promotion step Vision §59 exists to protect. *Producer obligation; the Kernel checks only that the domain is registered (TM-12). Tested against the Mock in Phase 2.*
- **TM-13e** A **transmit** stream's SampleClock id and ratio are allocated at `prepare` like any other (TM-13a), but its origin is fixed at **`arm`**, from the root tick at which the stream is armed, and the domain is registered before any burst is admitted against it. `prepare` precedes `arm`, so the anchor does not exist earlier. A plan-time admission check therefore compares leads only, which TM-21 does from the nominal rates TM-13a already fixed and which needs no origin; the conversion of a target onto the sample grid (spec 02, SC-23a) happens at the first admission point after `arm`. TM-13b cannot apply to a transmit stream: its first sample belongs to its first burst, while spec 02's SC-23a converts that burst's target into this very domain at admission, so the grid would be defined by the thing being converted and the first reactive burst would have nothing to be admitted against. A Provider that instead invented an origin at first transmission would restore the free choice TM-13b exists to remove. *Checked: the prepare path, `tm_13e_*`.*
- **TM-13c** A sample-rate change is a `cold` update that sets `ended_at` on the current SampleClock and allocates a new id. A stream never changes the rate of an existing domain, so a consumer detects the change from the block's own domain id. *Checked: `ClockRegistry::end`, `tm_13c_*`.* (Re-review R6.)
- **TM-13d** The Manifest records one `SampleClockRecord` per SampleClock, in order, per stream. *Checked: the record shape here; the envelope is spec 04.* (Vision §15, §23, §27, §50.)

### Relations and deadlines

- **TM-14** **Relation conversion.** Given `t` in the source domain with `valid.from ≤ t` and, when `valid.to` is present, `t ≤ valid.to` — otherwise `OutsideValidity` — let `delta = t − measured_at` in source ticks, let `delta_t` be `delta` rescaled to the target domain by TM-9, and let `drift_ticks = round(delta_t · drift)`. Then

  ```text
  nominal     = offset + delta_t + drift_ticks
  uncertainty = relation.uncertainty
              + |delta_t| · drift_uncertainty, rounded away from zero    drift error since measured_at
              + 1 tick + (1 tick if the rescale had a remainder)
  ```

  The uncertainty **must** grow with elapsed time, because `drift` is a measurement with an error. A relation measured once and used ten minutes later on an oscillator stable to 10^-8 carries a true bound near 6 µs; reporting the 51 ns of §6's worked example would overstate the timing claim of Vision §24 and §50 by two orders of magnitude, in the one field on which invariant 27's reproducibility rests. `drift` and `drift_uncertainty` are the only floating-point quantities in the time model, and neither reaches a stored `TimePoint` unrounded. A `drift_uncertainty` of zero claims that the drift is known exactly and is expected to be rare. The Kernel does not chain relations: a caller needing source → A → target measures or composes that relation explicitly.
- **TM-15** `RelativeBudget` and `AbsoluteDeadline` are distinct types with no common supertype. A `RelativeBudget`'s duration must be in `host.monotonic`, the domain in which an Executor measures elapsed processing time; constructing one in another domain fails. `RelativeBudget::deadline_from(arrival)` yields `AbsoluteDeadline(arrival + duration)` and fails with `DomainMismatch` unless `arrival` is in `host.monotonic`. `AbsoluteDeadline::remaining(now)` is a same-domain checked subtraction. Admission compares budgets against block periods by TM-9 rescaling; envelope checks compare absolute deadlines in device domains. (Vision §19; audit F8.)

- **TM-21** Two `Duration`s of the same domain compare on their ticks. Two `Duration`s of different domains compare **exactly**, by cross-multiplying against their nominal tick rates with 128-bit intermediates: `a` in domain A is at least `b` in domain B exactly when `a.ticks · num_B · den_A ≥ b.ticks · num_A · den_B`, where `num/den` are the domains' nominal rates (TM-10). The products are evaluated with checked 128-bit intermediates and TM-3's caps bound them at 2^124. Nothing is rescaled and nothing is rounded, so no direction has to be chosen and no resolution is lost. This is the rule TM-15's prose assumes when it says admission compares budgets against block periods, and it is what spec 02's SC-27 uses to compare a transmit target's lead, expressed in a SampleClock, against a `min_timed_command_lead` declared in `host.monotonic`.

  Rescaling one side with TM-9 and rounding, which was the first draft's rule, is wrong in both directions: a 1 ms lead rescaled into a 3 Hz SampleClock is 0.003 ticks, and rounding it up to one tick turns the requirement into 333 ms, a factor of 333. Rescaling the other way is inexact too — one tick at 30.72 Msps is 3125/96 ns. Cross-multiplication is exact for every rate pair, which matters because a check looser than the hardware is a defect and one needlessly stricter than the hardware fails experiments that would have run (Vision §59). TM-9 remains the rule for *producing* a `Duration` in another domain; it is not needed to compare two.

### Time Authority

- **TM-16a** A Run has exactly one `TimeAuthority`. It **declares** the set of domains it governs, which must include one **primary** `Root`, every `Derived` domain of that root, and `host.monotonic`. A Simulation Authority declares every root it simulates, including device timekeepers that drift independently of its primary root. `host.monotonic` is always governed; without that clause an Executor could not evaluate a `RelativeBudget` (TM-15) in a Simulation Run and Vision §29's `PROCESSOR_DEADLINE_MISS` could never fire there.
- **TM-16a1** The Authority **drives** `host.monotonic` as virtual time in the **Simulation** class only. In RealtimeEmulation, HardwareInLoop and Hardware it reads the real host clock and paces its primary root against that. RealtimeEmulation exists to expose real deadlines (Vision §14), so a Processor that spends two milliseconds of real processor time must be judged against a clock that advanced by two milliseconds; an Authority that drove `host.monotonic` there would under-report every deadline miss in the one class whose purpose is to surface them.
- **TM-16b** Governing a domain and being exactly related to it are different properties, and the governed set is declared rather than inferred from relatedness. An Authority advances the clocks it governs and accepts callbacks on them even where no exact conversion to its primary root exists. `NotGoverned` means that **no timekeeper in this Run advances that domain**, not that the domain is unrelated to the primary root. A Simulation Engine driving two independently drifting virtual devices governs both, so each model schedules on its own domain at an instant the Engine generated exactly, and the event order reproduces from the seed (Vision §58 #3). Were governance tied to relatedness, the second device could only schedule through a `ClockRelation` and the rounding of an `UncertainTimePoint` would decide the event order.
- **TM-16b1** Converting between a governed domain and a domain with another root still goes through a `ClockRelation` (TM-14) supplied by that domain's owner, and still yields an uncertain result. Governance settles who advances a clock, not what arithmetic is exact.
- **TM-16c** `now(d)` for a `Derived` domain returns the floor conversion of its root's current time. `schedule(t, f)` requires `t` in a governed domain at or after `now`, failing with `InPast` otherwise; callbacks fire in ascending time, ties in insertion order, and observe `now()` equal to their fire time while running. `cancel` reports whether the callback was still pending.
- **TM-16d** `wait_until(t)` fails with `NotGoverned` unless `t` is in a governed domain, and otherwise returns once `now` reaches `t`. It blocks, so it belongs to thread-driven components and must not be called by a step-driven one.
- **TM-17a** Phase 1 ships `ManualTimeAuthority`: time advances only on `advance_to(t)`, which fails with `InPast` when `t` precedes `now`, fires every due callback — including callbacks scheduled by callbacks during the same advance — in TM-16c order, then sets `now` to `t` and wakes every `wait_until` waiter. This is the seed of the Simulation Engine's `step(until)` semantics (audit F18); the Engine itself is Phase 2.
- **TM-17b** `advance_to` fails with `LimitExceeded` after a bounded number of callbacks fire at one instant (the default is 1 000). A model that reschedules itself at its own fire time otherwise makes the call spin forever, which hangs the test suite instead of failing it, in the very component on which Vision §58 #2 and #3 depend. Spec 05's stepping loop carries the same cap for the same reason.
- **TM-18** A Provider that owns a device timekeeper publishes a `ClockRelation` from that device's root to `host.monotonic`, and the Run's Authority does so for its primary root. Every Run records in its Manifest a `ClockRelation` from its root domain, device or virtual, to `utc`, with its uncertainty. The Kernel defines the record; measuring it is the Provider's work. *Producer obligation, tested in Phase 7.* (Vision §15, §50.)

### Serialisation

- **TM-19** In documents, a `TimePoint` serialises as `{domain: {node, local}, ticks}`, a `Rational` as `{num, den}` with integer members, and `ClockRelation.drift` as a JSON number. The encoding of 64-bit integers is settled in `00-overview.md` §6 and is not a semantic question. *Checked: `schema_freeze`.*
- **TM-20** Converting from an external representation — UHD seconds plus fraction, SoapySDR nanoseconds — is Provider boundary code that uses TM-9 rescaling with explicit rounding. The Kernel offers no approximation of a floating-point value by a `Rational` in v4.0. *Producer obligation at the Provider boundary, tested in Phase 7.*

## 6. Worked numbers

Exact conversion on an X310. Root `dev` runs at 200 MHz. Stream A at 20 Msps has `root_ticks_per_tick = 10/1`, stream B at 25 Msps has `8/1`.

| case | t_A → root | → t_B | result |
|---|---|---|---|
| origins 0, t_A = 4 | 40 | 40/8 | `Exact 5` |
| origins 0, t_A = 7 | 70 | 70/8 | `Inexact { floor 8, remainder 3/4 }` |
| origins 0, t_A = 10^12 | 10^13 | 1.25 · 10^12 | `Exact`; fits in `i64`, and floating point could not have proven exactness |
| o_A = 1 000 000 003, o_B = 1 000 000 000, t_A = 0 | 1 000 000 003 | 3/8 | `Inexact { 0, 3/8 }` |
| same origins, t_A = 5 | 1 000 000 053 | 53/8 | `Inexact { 6, 5/8 }` |
| same origins, any t_A | 3 + 10·t_A | always odd mod 8 | never exact: the two sample grids are disjoint |

The last row is why `origin` exists. A stream started with `stream_now` begins on an arbitrary root tick, and two such streams may share no sample instant at all. Floating seconds would report spurious coincidences between them.

Overflow bound. With `|ticks|` and `|origin|` at most 2^62 and every ratio term at most 2^31 by TM-3, the numerator of TM-4 needs at most 126 bits and fits in a signed 128-bit integer. At the extremes of all fields the expression would need 193 bits, which is why TM-3 caps the ratio and TM-8 checks every step rather than reaching for arbitrary precision.

Relation conversion. Source `dev` at 200 MHz, target `utc` at 1 GHz, `measured_at = 200 000 000` (device time one second), `offset = 1 700 000 000 000 000 000` ns, `drift = 1e-6`, `drift_uncertainty = 1e-8`, `uncertainty = 50` ns. Converting `t = 400 000 000`: `delta` is 200 000 000 device ticks, which rescales exactly to 1 000 000 000 ns; `drift_ticks = round(1e9 · 1e-6) = 1 000`; `nominal = offset + 1 000 001 000` ns; `uncertainty = 50 + ceil(1e9 · 1e-8) + 1 = 61` ns.

The growth term is the point of TM-14. One second after the measurement it adds 10 ns; ten minutes after it adds 6 µs and dominates. The `offset` in this example, 1.7 × 10^18, is also why the canonical form carries the integer profile of `00-overview.md` X5: through a strict RFC 8785 double it would lose its last bits and two Runs a nanosecond apart would share a Manifest hash.

## 7. Decisions

| # | Decision | Choice | Rejected (one line each) | Ceiling / upgrade path |
|---|---|---|---|---|
| T1 | Rational tick rates | Own `Rational` of two unsigned 64-bit fields, gcd-normalised, 128-bit intermediates, errors not panics; about forty lines | `num-rational` (a dependency plus `num-traits` for what fits in forty lines, and its `Ratio` multiplication panics on overflow where we must return an error); floating-point rates (TM-1); a fixed 1 ns tick (30.72 MHz is not an integer number of nanoseconds) | Ratio terms and `Root` tick-rate terms ≤ 2^31 with ticks ≤ 2^62 make overflow provably impossible (TM-3, TM-21). The cap puts a `Root` above about 2.1 GHz out of direct reach; a 4 Gsps direct-sampling clock registers as a 2 GHz root with `root_ticks_per_tick = 1/2`, which TM-3 and TM-4 both accept. Widening the fields to 128 bits is a layout change only, since documents already carry integers |
| T2 | Domain tree | `Root` plus `Derived`-to-root with a ratio and an origin in root ticks | Parent chains (composition arithmetic and a deeper overflow analysis for no gain in exactness); a shared epoch without an origin (cannot express a `stream_now` start on a root tick not divisible by the decimation, so the disjoint-grid row above becomes unrepresentable); block times in root ticks (then a SampleClock is not a domain and TM-10 is lost) | Every domain's tick zero lies on a root tick; a rational origin is the upgrade if a future device needs one |
| T3 | Cross-domain comparison | Runtime check returning `DomainMismatch`; no ordering operators; `ticks_in()` does the check once at ingress and then the hot loop is raw `i64` | `PartialOrd` returning `None` (`a < b` silently false is exactly the bug class this model exists to prevent); a type-level domain parameter (domains are runtime objects created at `prepare`, serialised forms carry ids anyway, and branded types would wreck link and plugin ergonomics); panicking (the real-time path) | One id comparison per block at ingress |
| T4 | Duration | Domain-tagged, checked against `TimePoint` | Untagged (1 000 ticks at 20 Msps added to a 200 MHz point is a silent factor-of-ten error) | none |
| T5 | Deadlines | Two newtypes; `RelativeBudget` in `host.monotonic` | One `Deadline` enum (the audit §13 tree writes it that way as shorthand, but Vision §19 says distinct types and the checks that consume them differ); a domain-less nanosecond budget type (a second duration type where the reserved domain gives the same thing with one) | In the Simulation class the Engine must drive `host.monotonic` as virtual time (Phase 2) |
| T6 | ClockRelation numerics | Offset and uncertainty in integer target ticks; `drift` and `drift_uncertainty` as floats; the stored uncertainty is the bound **at `measured_at`**, which TM-14 grows with elapsed time | Drift as a `Rational` (it is a measurement; there is nothing exact to preserve); deferring `drift_uncertainty` as an additive change — it is not, because the ClockRelation schema freezes at v4.0 and a reader of a v1 Manifest would take a constant bound at face value; storing relations inside `ClockDomain` (they are re-measured and there are many per Run) | A linear bound with no allowance for a non-linear excursion; a relation whose `valid.to` is absent grows its bound without limit, which is honest rather than wrong |
| T7 | Conversion API | `ExactConversion` precomputed at plan time, `apply` returning `Converted`, plus `try_exact` | Going through the registry on every call. The reason is not mainly the map lookup: precomputing the three integer terms once is where §6's 126-bit overflow bound is pinned, because registration enforces the caps that make every later `apply` provably safe, and a per-call path would have to re-derive them. Also rejected: returning a floating-point remainder | none |
| T8 | TimeAuthority shape and scope | Synchronous trait with `now`, `wait_until`, `schedule`, `cancel`; one Authority per Run that **declares** the domains it governs, which must include a primary root, that root's derived domains and `host.monotonic`, and for a Simulation Authority every root it simulates even where no exact conversion exists; `host.monotonic` is driven as virtual time in the Simulation class and read from the real clock in the other three; `ManualTimeAuthority` in Phase 1 behind a `testing` feature | An async trait (pulls a runtime into the Kernel and forces every Provider to be async); one Authority per *root*, which was the first draft's rule and made `now(host.monotonic)` fail in every Simulation Run, so no `RelativeBudget` could be evaluated and `PROCESSOR_DEADLINE_MISS` could never fire, while leaving a two-device Run with two Authorities and no rule for which one `wait_until` uses; an Authority that answers exactly for every root (it cannot — unrelated roots are related only by measurement); scheduling Kernel `Action` values directly (`Action` belongs to spec 04, which wraps `schedule`); building the discrete-event Engine now (Phase 2) | Conversion between roots stays uncertain however governance is declared (TM-16b1); the Engine generalises the manual authority without changing the trait |
| T9 | SampleClock lifecycle | Id and ratio at `prepare`, origin and registration before the first block, a new id on a rate change | Creating it at `start` (then the plan and the links cannot reference it); origin always zero (T2); changing the rate in place (re-review R6) | none |
| T10 | Reserved domains | `utc` and `host.monotonic` at 1 GHz with fixed local ids | Seconds plus fraction (UHD's shape, not ticks); reserving none (the Manifest's epoch ↔ UTC target becomes ad hoc and budgets have no static domain) | 1 ns resolution for host and UTC; finer would be a new reserved id |
| T11 | Overflow | Checked, returning `Overflow` | Wrapping, saturating, panicking | none |
| T12 | `now()` in a derived domain | Floor | Erroring on an inexact instant (makes `now` unusable in a SampleClock, which is where it is needed) | none |
| T13 | Relation chaining | Not in the Kernel | A graph search over registered relations (Finding 2 asks only for the type) | A composition helper can live in a Vocabulary crate |

## 8. Phase 1 tests

| test | input | expected | rules |
|---|---|---|---|
| `tm_02_rational_normalises_by_gcd` | 200 000 000/10, 6/4 | 20 000 000/1, 3/2, structurally equal | TM-2 |
| `tm_02_rational_rejects_zero` | 0/5, 5/0 | `InvalidRational` | TM-2 |
| `tm_02_rational_mul_div_use_wide_intermediates` | (u64::MAX/1)·(1/u64::MAX), (2^40/3)·(3/2^40) | 1/1 for both, no panic | TM-2 |
| `tm_02_rational_cmp_cross_multiplies` | 1/3 versus 2/5, then near-maximum values | correct ordering, no overflow | TM-2 |
| `tm_04_derived_to_root_exact` | ratio 10, origin 1 000 000 003, t = 7 | root 1 000 000 073, `Exact` | TM-4 |
| `tm_04_sibling_20_25_msps_exact` | origins 0, t_A = 4, then 10^12 | `Exact 5`, `Exact 1.25e12` | TM-4 |
| `tm_04_sibling_20_25_msps_inexact` | origins 0, t_A = 7 | `Inexact { 8, 3/4 }`; `try_exact` gives `Inexact` | TM-4 |
| `tm_04_disjoint_grids_never_exact` | o_A = 1e9+3, o_B = 1e9, t_A in 0..1000 | every result inexact | TM-4 |
| `tm_08_conversion_overflow_is_error` | t = 2^62 with ratio 2^31/1 in both directions | `Overflow`, no panic | TM-8 |
| `tm_03_registration_limits` | ratio 2^31+1; a derived domain of a derived domain; an unknown root | `LimitExceeded`, error, `UnknownDomain` | TM-3, TM-12 |
| `tm_06_cross_domain_cmp_is_error` | `try_cmp` and `checked_sub` across domains, then within one | `DomainMismatch`, then success | TM-6 |
| `tm_07_duration_add_checks_domain` | `t_A + d_B`, then `t_A + d_A` | error, then success | TM-7 |
| `tm_08_tick_add_overflow_is_error` | `i64::MAX` plus one tick | `Overflow` | TM-8 |
| `tm_09_duration_nominal_rescale` | 1 000 samples at 20 Msps into ns; one tick at 3 Hz into ns | 50 000 exactly; inexact with remainder 1/3 | TM-9 |
| `tm_05_unrelated_roots_need_relation` | a conversion between a device root and `utc` | `Unrelated` | TM-5 |
| `tm_14_relation_converts_with_uncertainty` | the example of §6 | nominal offset + 1 000 001 000 ns, uncertainty 61 ns | TM-14 |
| `tm_14_relation_outside_validity` | `t` beyond `valid.to` | `OutsideValidity` | TM-14 |
| `tm_15_budget_deadline_from_arrival` | a 500 µs budget with an arrival in `host.monotonic`, then in a device root | an absolute deadline, then `DomainMismatch` | TM-15 |
| `tm_15_budget_rejects_non_host_domain` | a budget in a SampleClock | error at construction | TM-15 |
| `tm_11_reserved_domains_present` | a fresh registry | `utc` and `host.monotonic` registered, `Root`, 1 GHz | TM-11 |
| `tm_11_registry_allocates_monotonic_unique` | three allocations | strictly increasing local ids, node `LOCAL` | TM-11 |
| `tm_13c_sample_clock_new_id_on_rate_change` | declare and register A, then change the rate | A's `ended_at` set, B's id differs, records in order with origins | TM-13c |
| `tm_13e_tx_origin_fixed_at_arm` | a transmit stream, with a burst target admitted before any block exists | the grid exists at admission and the origin is the declared arm anchor | TM-13e, SC-23a |
| `tm_16b_engine_governs_a_drifting_virtual_device` | a Simulation Authority declaring two roots, the second drifting | `now` and `schedule` succeed on the second root; the event order repeats from one seed | TM-16a, TM-16b |
| `tm_16a1_host_monotonic_driven_only_in_simulation` | a Simulation Authority and a RealtimeEmulation one, each advanced | the first moves `host.monotonic` itself; the second tracks the real clock | TM-16a1 |
| `tm_16d_wait_until_non_governed` | `wait_until` on a root no Authority declared | `NotGoverned` | TM-16d |
| `tm_03_root_tick_rate_capped` | a `Root` at 2^40 / (2^40 − 1) | `LimitExceeded` | TM-3, TM-21 |
| `tm_13b_first_block_is_tick_zero` | a stream whose origin is its first sample's root tick; then a block naming an unregistered domain | the first block's `ticks` is 0; the second is refused | TM-13b, TM-12 |
| `tm_16_authority_order_and_now` | schedule at 30, 10, 10, then `advance_to(30)` | fired in order 10, 10, 30; `now()` inside each equals its fire time | TM-16c, TM-17a |
| `tm_17_authority_nested_schedule` | a callback at 10 schedules one at 20, then `advance_to(30)` | 20 fires in the same advance, after 10 and before 30 | TM-17a |
| `tm_17_authority_cancel_and_in_past` | cancel a pending callback; schedule before now; advance backwards | true then false; `InPast`; `InPast` | TM-16c, TM-17a |
| `tm_16_authority_now_in_derived_floors` | root now 1 000 000 007, SampleClock ratio 10 origin 3 | 100 000 000 | TM-12, TM-16c |
| `tm_16_authority_wait_until_wakes` | a thread blocks in `wait_until(50)`, the main thread advances to 50 | the waiter returns | TM-16c |
| `tm_16a_host_monotonic_always_governed` | `now(host.monotonic)` on a device-root authority and on a virtual-root one | both succeed | TM-16a |
| `tm_16b_ungoverned_root_is_not_answered` | `now` on a root that no Authority declared | `NotGoverned`; conversion through its relation still succeeds | TM-16b, TM-16b1 |
| `tm_17b_advance_to_zero_delay_cap` | a callback that reschedules itself at its own fire time | `LimitExceeded` after the cap, not a hang | TM-17b |
| `tm_21_duration_cmp_same_domain` | 1 000 against 2 000 ticks in one SampleClock | ordered | TM-21 |
| `tm_21_duration_cmp_across_domains_is_exact` | 1 ms in `host.monotonic` against 30 000 ticks at 20 Msps | the host value is the smaller; no rescale, no rounding | TM-21 |
| `tm_21_duration_cmp_survives_awkward_rates` | 1 ms against one tick at 3 Hz, and against one tick at 30.72 Msps | correct both ways, although neither rate divides the other; a rescale-and-round implementation reports the 3 Hz case 333 times too strict and fails this test | TM-21 |
| `tm_21_duration_cmp_overflow_is_error` | cross-multiplication near `i64::MAX` with 2^31 rate terms | `Overflow`, no panic | TM-21, TM-8 |
| `tm_14_uncertainty_grows_with_elapsed_time` | `drift_uncertainty` 1e-8 with `delta_t` of 600 s | about 6 µs, not 51 ns | TM-14 |
| `tm_14_zero_drift_uncertainty_is_constant` | `drift_uncertainty` 0 | the stored bound plus rounding only | TM-14 |

## 9. Vision coverage

| Vision | This spec |
|---|---|
| §15 representation: ticks, SampleClock, exact versus ClockRelation, rate change, epoch | §3, §4, TM-1…TM-5, TM-10, TM-13a…d, TM-18 |
| §15 Time Authority | §4 trait, TM-16a…d, TM-17a, TM-17b |
| §19 two kinds of deadline | TM-15, decision T5 |
| §24 ClockRelation | §4, TM-14, decision T6 |
| §27 a sample-rate change is cold and starts a new SampleClock | TM-13c |
| §49 node-qualified `ClockDomainId` | TM-11 |
| §50 epoch ↔ UTC relation in the Manifest | TM-18 and the record shapes; the envelope is spec 04 |
| §58 #2, #3 | the interface only (TM-16a…d, TM-17a, TM-17b); the tests themselves are Phase 2 |
| §61 capture at a TimePoint or a sample index | TM-10 |

## 10. Vision issues found

1. **§19 versus §15 on a budget's domain.** §19 gives `RelativeBudget { duration }` while §15 says a `Duration` is "in a named domain", but a static `ComponentDescriptor` cannot name a domain created at `prepare`. Resolved by reserving `host.monotonic` (TM-11, TM-15).
2. **§15's "no other path" versus §19's admission arithmetic.** Admission must compare a SampleClock block period with a host-domain budget, and no `ClockRelation` exists at that moment. Resolved by TM-9: `Duration`s rescale nominally, while `TimePoint`s keep the no-other-path rule.
3. **§15 omits a SampleClock's origin.** It says a SampleClock is "derived from its device clock by an exact rational", but a `stream_now` start lands on an arbitrary root tick, so a ratio alone cannot represent block times exactly. Resolved by `origin` (decision T2), demonstrated by the disjoint-grid row in §6.
4. **Audit §13 writes one `Deadline{Relative | Absolute}` type** where Vision §19 says two distinct types. This spec follows §19; the audit tree is shorthand.
5. **Audit Finding 2's justification is wrong in mechanism.** "f64 rounds at 10^12 samples" attributes the failure to magnitude; the real failure is that grid membership is undecidable in floating point, first breaking at n = 105. The conclusion, integer ticks, stands.
6. **§15's "functional simulation may run faster than wall clock"** is a property of the Engine, not of these types, and is untestable here. Deferred to Phase 2 (§58 #2).
7. **§15 names the Time Authority in the singular but never scopes it.** "Something must be the authority on 'now' and on 'wait until'" leaves open whether that is per Run, per device or per clock domain. A Run with a USRP and a HackRF has two timekeepers, and a Simulation Run needs the Engine to answer for `host.monotonic` as well as for virtual device time. TM-16a and TM-16b scope it: one Authority per Run, governing its primary root, that root's derived domains and `host.monotonic`, with every other root reached by measurement. Without the `host.monotonic` clause a `RelativeBudget` could not be evaluated in simulation at all, so Vision §29's `PROCESSOR_DEADLINE_MISS` could never fire there.

## 11. Deferred

The Simulation Engine, including `step(until)` over Islands and the wall-clock pacing of RealtimeEmulation (Phase 2). TimingEnvelope fields and their measured values (Radio Model, Phase 2). Device-to-host relation measurement methods (Phase 7). UHD and SoapySDR boundary conversions (TM-20). Approximating a float by a `Rational`. Relation composition. Identifiers with `node ≠ LOCAL` (Vision §49).
