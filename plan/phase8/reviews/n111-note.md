# Design note — `supplied-callback-under-lock` (three strikes, AGENTS §6; 2026-11-19)

## The mechanism and its count
A Kernel lock that a Module or Provider thread can wait on — `admission` (every `submit`, KC-24a) or `machine` (`state()`, and taken under `admission`) — is held across a call into embedder-supplied code: the `TimeAuthority` (`now`, `schedule`, `cancel`) or the `HostClock` (`utc_nanos`). A supplied object may take its own locks, and a Module thread may hold one of them while it submits, so the two wait on each other.
1. #103's test-plan review, finding F1 (2026-11-18): the start published `Running` under `admission` with `now()` and `utc_nanos()` read inside. Fixed in #103 (`Shared::sample_transition`, `move_to_at`).
2. #111: cleanup's freeze calls `TimeAuthority::cancel` under `admission` (`ending.rs:~293–316`), and — found in planning — takes `wake_handle` there, which `paced::wake` holds across `schedule` (a three-party cycle).
3. #119: ending enters `Stopping` and `CleanedUp` reading `utc_nanos()` under `machine` (`ending.rs:30`, `:42`).

## The question the rule asks
Can the concept be removed or merged? The locks stay: `admission` orders submissions against the freeze and the start's batch (KC-24a, RS-6, #103), and `machine` is the Run state. The supplied callbacks stay: time and the host clock are the embedder's by design (Vision §15, the Simulation Engine and the hardware Authority). What can go is the *freedom* to call a supplied object while holding one of those locks.

## Decision (orchestrator)
**One rule, stated once and checked by one double.** KC-24a gains the rule: *the coordinator calls no supplied `TimeAuthority` or `HostClock` method while it holds the admission or the machine lock; a value it needs under a lock is read before the lock, and a handle it must cancel is taken under the lock and cancelled after it.* Registered checks and `Provider::coerce` run under `admission` by design (KC-24 steps 10–12); KC-24a's Ceiling names them, so the rule is not read as covering them.
- Code: #111 and #119 fixed in this step with the shapes #103 used (read before, act after): the freeze drops `admission` before it takes `wake_handle` and `scheduled` and cancels; ending reads the transition with `sample_transition` before `machine` (`begin_stopping_at`, `finish_at`, as `move_to_at`).
- Tests that pin the rule rather than the three sites: the watched doubles (`WatchedAuthority`, a `WatchedHostClock`) record, for *every* call, whether `admission` or `machine` is held; one sweep test drives the Run through validate, start, a timed dispatch, a Module submission, a device loss, `finish` and an abort on a Simulation Run and on a device-paced Run, and asserts that no supplied call was made under either lock. A future fourth instance fails that sweep without a new test.

## Rejected
- *Make the supplied objects lock-free by contract* (a rule for embedders): it moves the burden to every Authority and HostClock, and the hardware Authority calls UHD, which locks.
- *Run supplied calls on a dedicated thread*: a new thread and queue in the Kernel for three call sites; the read-before/act-after shape is smaller and already used.
- *Fix the three sites without the sweep*: the mechanism would wait for its fourth instance.

## Open
Registered checks and `coerce` under `admission` stay as KC-24a's Ceiling states (a Provider-slot cycle is named there). If a fourth instance arises from them, this note is reopened.
