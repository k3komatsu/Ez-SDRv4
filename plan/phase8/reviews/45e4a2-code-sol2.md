# REVISE

Paths below are relative to `crates/ezsdr-radio-uhd/src/` unless stated otherwise. No P1 findings.

## Previous P2 dispositions
- **Brain exit/join: partly closed.** `brain_exited`, `BExit`, and recorded brain returns repair the model and forced replay (`brain/world.rs:666–670`, `brain/threads.rs:535–567`). Free-running termination remains incorrect below.
- **Native owner release: closed within the scoped model.** Release is independent of cleanup (`brain/world.rs:919–937`); the missing-mark panic regression reaches `OwnerRelease` and I19 (`brain/stop_tests.rs:83–99`).
- **Failed-start shutdown: closed for the tested receive prefix.** Failure itself queues shutdown, suppresses Actions and later spawns, and starts teardown (`brain/world.rs:1146–1174`). The surviving executor’s device stop is asserted (`brain/stop_tests.rs:164–173`).

## Findings
1. **P2 — A clean free-running brain can wait forever after termination.**  
   `brain/threads.rs:530–531,562`; `brain/shared.rs:260–262`.  
   Sequence: brain processes Shutdown → sends executor terminates → checks `senders_gone` before executors exit → parks → executors exit without another input.  
   The wait predicate excludes sender completion, so even a notification cannot make it proceed. The brain never records exit; shutdown unnecessarily times out/detaches and retains handles.  
   Fix: make sender completion wake and satisfy the brain’s exit wait. Add this ordering without forced `BExit`; current lifecycle tests explicitly inject that event (`brain/stop_tests.rs:222–245`).

2. **P2 — The thread implementation still treats “about to send Lost” as “sender gone.”**  
   `brain/threads.rs:699–704`; `brain/shared.rs:255–257`; `brain/threads.rs:299–304,562`.  
   Sequence: R publishes loss → marks itself exited → pauses before `RSend` → a terminated brain finishes its current input and sees all senders exited → brain returns → R sends Lost.  
   The boundary remains unserved, potentially leaving the value it must dispose unresolved—the seed-6739 failure again. `join_within` also reports success before that send.  
   Fix: distinguish actual sender/thread completion from loop exit, and publish completion after the final send. The world already requires an empty receive program (`brain/world.rs:555–556`), but threads do not.

## Rules, Abort, and mutation evidence
- **Seed 3868:** waiting for detached device owners is sound: today’s threads retain the shared device. However, `joinee_done` still accepts an executor with its final send pending (`brain/world.rs:566–570,1307–1309`); it is not an actual thread-exit fact.
- **Seed 6739:** the no-more-senders rule is sound, not a reason to discard the late boundary. Its thread implementation is incomplete as finding 2 shows.
- Native release omits `UhdDevice::drop`’s `answers()` fallback (`uhd.rs:676–678`). This is conservative for the reachable panic/missing-mark regression: a responsive device would not rescue the missing mark.
- Treating Abort as ordinary shutdown is acceptable **for this failed-start ordering**: S never exists and no Actions were admitted. It does not establish Abort/sample-cut equivalence (`provider/rx.rs:253–261`); §4.7’s native-lock blocking remains relevant.
- `/tmp/e4a2b-probe.log:1–31` records 31 test kills, not compile errors. **Not all are reachable lifecycle kills:** E4-86/87/100 use hand-built states; E4-97–99 are labelled oracle self-tests; retargeted E4-79/80 remain destructor-unit coverage. E4-88 exercises forced exit, so it misses both free-running failures above.

Not checked: fresh builds, test/mutation reruns, hardware, or either forbidden holdout directory. No files or git state changed.
