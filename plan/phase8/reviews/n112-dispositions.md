# #112 and #114 — dispositions and counts

#112: a lone surrogate in a Python request ended the Session; the client now encodes before the exchange (owner: fix it in the client). #114: `Violation.requested` held a Module's reported value at seven Kernel producers; it now holds what the experimenter or the Action asked, or nothing. Way of working: the orchestrator wrote #112; a Sonnet subagent supervised Go DeepSeek for #114 and the fix round. Reviews: Sol 6.2 on the code ([n112-sol-code.md](n112-sol-code.md)), Opus re-check ([n112-recheck.md](n112-recheck.md)). Briefs and reports: [n112-briefs.md](n112-briefs.md), [n112-report.md](n112-report.md). Gate: `n112-gate.json` (beside this file).

## Counts (source, severity, mechanism)

| Source | Major | Minor | Note | Mechanism |
|---|---|---|---|---|
| Sol 6.2, code | 1 | 3 | 1 | none counted (F1 a Sink lookup missed in the step) |
| Opus re-check | 0 | 2 | 2 | none |

Follow-ups filed: #115 (a Module can record a false `Coercion.requested` in an accepted Manifest), #116 (the RF-envelope check's `requested`).

## Decisions

- #112 (owner 2026-11-19 「Python側でOKです」): the client encodes the request before writing; a lone surrogate raises UnicodeEncodeError and the Session stays. Implemented by the orchestrator (a few lines), row N112-1, C89-3-04 re-anchored.
- #114 (orchestrator): extended from the issue's three producers to every prepare/validate producer that recorded a Module's value (seven); Session-admission SB-46 unchanged (Kernel-built coercion from the Action's value).

## Sol 6.2 code review (sol-code.md): FAIL, 1 major, 3 minors, 1 note — dispositions (orchestrator)
- F1 major adopted: a Sink fragment's requested values are its output's `params`; SB-46 and SB-6 at prepare look them up there (Provider fragments keep `spec.resources`). A test per site.
- F2 minor adopted: `call` checks the closed connection before encoding (EA-16: later calls raise ProtocolError).
- F3 minor adopted: the #112 test asserts the exception type explicitly (an assertion kill for N112-1) and checks the logged Action and its value, not a substring search.
- F4 minor adopted: the stray-key `requested` assertions go into the two existing SB-44 stray tests; N114-4/5 retargeted; the duplicate tests removed.
- F5 note: KC-24 step 10's pairing of the rejected key with the Action's value is what the KC-24 table states; no change.
- Sol's verification note (Manifest coercions): a Module can report a false `Coercion.requested` in an accepted Manifest — pre-existing, filed as its own issue.
- Opus re-check of the fix round (recheck.md): F1–F4 CLOSED, no major. N4 ("(Sinks too)" on the Executor test's entry) fixed by the orchestrator; N1 (the registered RF-envelope check records a Module's value) filed as #116; N2, N3 notes, no action.
- mech (orchestrator): #112 and #114 "mech: none" (comments on the issues).
