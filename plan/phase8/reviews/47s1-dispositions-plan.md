# Dispositions of Sol's review of test-plan-s1 (orchestrator, 2026-11-04)
Source: ~/.cache/ezsdr-fixes/2026-11-04-n46/sol-s1-plan-final.md (PLAN NEEDS CHANGES, 9 findings: 7 major, 2 minor). All adopted.
1 (major) adopted: client and raw-server cases refusing one stream name across rx and tx; keep a distinct-name rx/tx success (P1343).
2 (major) adopted: throughput 200+100 MS/s refused at Validate and 200+50 accepted; no device open/configure/start on refusal; add tx_enabled=false with an assigned TX channel; receive-only success with explicit rx membership; RF band case on an assigned TX channel.
3 (major) adopted: server expansion errors observed as zero coordinator::connect entries and no failed-Run Manifest. Contract kept: an undeclared key is a Validate-stage failed Run (S12), not a pre-Run refusal.
4 (major) adopted: [2,0] with distinguishable channel signals, asserting logical sample order; empty action log immediately after connect, no membership updates later; stream names through the server path.
5 (major) adopted: initial applies before the first rx_open/tx_open; start/send checked separately; P1344 redesigned as a real compilable mutation.
6 (major) adopted: literal old/new anchors for every new row; P49/P137 replacements keep both insert arguments and P49 gets a direct defaults() assertion; P1328/P1329 observe the Spec's expanded requires (or are dropped if explicit none-requirements are not contractual; say which); P1331 tests the exact count boundary; P1336 asserts the empty RX frame.
7 (major) adopted: sweep example's child Spec carries rx membership in its requires; audit explicit Spec fixtures too.
8 (minor) adopted: use manifest.clocks.sample_clocks, block headers/Provider records, coercions_preview; count coerce calls in validation and prepare separately.
9 (minor) adopted: exact error categories; nonnumeric/nonfinite values refused before any request; direct alias reads raise Error; raw string-channel refusal at the server.
D3: VERIFIED Session warn applies; drop any policy workaround, keep the regression test. D4: VERIFIED by source; S1 confirms end to end.
