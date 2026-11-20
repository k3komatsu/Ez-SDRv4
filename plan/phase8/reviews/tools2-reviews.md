# tools2 — Terra reviews


## terra

1. **Major — A concurrent lane can replace the mutation image before shards start.** `remote_mutate.py:69` builds every checkout’s Dockerfile under the shared tag `ezsdr-mut`; `sweep_mutants.py:92` uploads and builds before starting containers by that tag. Existing mutation runs use the same tag.

   Minimal sequence: lane A builds the new image; lane B builds the old Dockerfile under `ezsdr-mut`; lane A starts its shards. Lane A then runs the old image, so the UHD row can fail to build for lack of libuhd, or a sweep can lack cargo-mutants. The reverse ordering changes the image used by B. Use a per-run image tag or immutable image ID.

2. **Major — A partial sweep can report success.** `sweep_mutants.py:96` starts detached containers and waits only until they disappear. It does not inspect their exit codes; `lines 70–83` count whichever result files exist, and `line 109` succeeds if *any* outcomes exist.

   Minimal sequence: start 16 shards; one exits before writing results and another completes; the runner prints partial totals and exits zero. A reused output directory can also contribute stale outcomes. Require one complete result per requested shard and reject failed or missing runs.

3. **Major — The trial treats timeouts as completed workspace survivors.** `trial.md:17` calls 38 mutants “all tests passed” despite stopping them at 400 seconds, while the estimated full suite takes about 550 seconds. A test after the cutoff could still kill one; a mutant could also hang. The same unsupported claim appears in individual classifications, for example `trial.md:59`. Minimal sequence: a mutant passes the tests reached before 400 seconds but fails a later test; the report classifies it as a workspace survivor. Keep these as unresolved timeouts until a run completes or the specific hang is established.

4. **Minor — The overnight conclusion is conditional, not established by the measurements.** The stated arithmetic is approximately consistent: the first pass is about 280,000 shard-seconds / 16 ≈ **4.9 hours**; the second is `(900 × 600 + 240 × 100) / 16` ≈ **9.8 hours**, for about **14.7 hours**, before imbalance and setup. But `trial.md:41` and `lines 45–46` extrapolate unmeasured costs and survivor rates, and the 38 timeouts do not establish how many complete.

   Holding the report’s other assumptions fixed, an overall first-pass survivor rate of roughly **11.5% or less**, instead of 16%, puts the estimate below 12 hours. A lower UHD package cost or a decision to sort package survivors without a full workspace rerun would also change the conclusion. The estimate supports “possibly longer than overnight,” not a measured impossibility.

5. **Minor — Sweep failures and resource coordination are not consistently surfaced.** `sweep_mutants.py:44` and `line 56` ignore the remote command’s return code and return success; the baseline loop also suppresses Cargo errors. The runner relies on the caller using `simq` when another lane uses the sims (`line 4`); it does not enforce the lock or CPU budget. Thus failed measurements can look valid, and an uncoordinated invocation can overlap a gate.

6. **Note — Sampled sorting is plausible, with two qualifications.** I checked five examples against the code:

   - `Grid::contains`: the finite and range guards at `device.rs:58` make the proposed boundary tests meaningful.
   - `at_or_above` / `at_or_below`: equality at an endpoint matters at `device.rs:99` and `line 124`, supporting tests for the changed comparisons.
   - `FrontEnd::receives`: returning `true` for `BasicTx` or `LfTx` is wrong according to `x3xx.rs:265`, supporting a test.
   - `made_subdev_spec`: changing `&&` to `||` can free a successfully made handle at `uhd.rs:429`, supporting the proposed test.
   - `Lines::line` / `book`: these are thin methods (`timeline.rs:293`), but delegation alone does not establish that Provider tests kill the mutants. The report correctly marks that as unverified; acceptance should remain provisional.

7. **Note — Other paths reviewed.** Rust row features reach the build, baseline, mutant test and exact rerun, and are included in the baseline cache key (`mutate.py:137`, `line 288`). Python rows take a separate path and do not use features; none is claimed for the new row. Anchor validation is independent of features. The new image retains a non-root user, and Python rows explicitly use their scratch target’s server. The sweep mounts the uploaded source read-only and invokes cargo-mutants inside containers; its extra arguments are not restricted, but that mount protects the uploaded source. I did not run builds, tests, mutations, or formatting.

**Verdict: FAIL**

## terra-recheck

**Verdict: FAIL.** The normal trial paths passed, but concurrent runs can still share a tag, exception paths can leak remote resources, and a sweep can accept an incomplete shard result.

**Major findings**

1. **Same-second runs can still replace each other’s image and files.** Two checkouts starting the same list or sweep on the same host within one second derive the same tag from `int(time.time())`: `remote_mutate.py:123`, `sweep_mutants.py:97`. They then upload to the same directory and build the same image tag; one can overwrite or delete the other’s inputs. The e2e, list and baseline paths have the same issue. Use a unique run identifier, including across checkouts.

2. **Exceptions leave images and containers behind.** For example, if the first sweep shard starts and the next `docker run` fails, `sweep_mutants.py:103` raises before reaching cleanup at `line 118`. The first container remains running or stopped, and the uploaded directories and images remain. Upload, wait and rsync failures have similar paths; the mutation runner and e2e also lack cleanup on exceptions. Track created resources and clean them in a `finally` path, including partially completed uploads.

3. **A shard with no mutant results can be counted as complete.** The sweep checks only that each `outcomes.json` exists at `sweep_mutants.py:120`, while `line 124` requires results only in the aggregate. If one shard exits with an accepted code and writes a valid JSON file containing no mutant outcomes, another shard’s results make the run return zero. Validate completeness per requested shard. Malformed JSON does produce a non-zero exception, but bypasses the normal failure report.

**Earlier findings**

1. **PARTLY** — Normal row, e2e and sweep containers now use the image built by `upload` (`remote_mutate.py:85`, `line 96`, `line 138`, `sweep_mutants.py:105`). Collisions and exception cleanup remain as above. The new cleanup names the run-specific tag, not the old shared `ezsdr-mut` tag (`remote_mutate.py:77`).

2. **PARTLY** — The sweep waits for exit codes, refuses a non-empty output directory and detects missing files (`sweep_mutants.py:91`, `line 109`). Per-shard completeness remains open as above.

3. **PARTLY** — The report now calls the 38 cases unresolved and explains the longer rerun (`trial.md:17`). However, `line 14` still calls the workspace cases reaching 400 seconds survivors, and `line 69` asserts that a timeout case has no workspace test.

4. **CLOSED** — The estimate is conditional, includes the approximately 11.5% break-even and names measurements that would resolve the assumptions (`trial.md:48`).

5. **CLOSED** — `list` propagates the remote return code; `baselines` stops on Cargo failure and propagates its return code (`sweep_mutants.py:48`, `line 61`). The coordination requirement is documented at `line 4`.

The supplied logs show e2e 16/16, four mutation rows passing, a two-shard sweep with results, and non-zero exits for a missing-results run and a reused output directory. The missing-results trial failed both shards, so it does not demonstrate the mixed successful/incomplete case. I did not run builds, mutation jobs or formatting.

## terra-recheck2

**Verdict: FAIL.** The run ID and the specific empty-shard finding are fixed, but cleanup still has paths that leak resources or hide failures.

Failure scenarios considered:

- Two checkouts start the same operation in the same second and share a work directory, image, or container name.
- Upload, container startup, waiting, or result collection fails—or the operator presses Ctrl-C—after resources exist on one or more hosts.
- Cleanup itself raises or a remote removal command fails, preventing later hosts from being cleaned or masking the original failure.
- One shard returns an accepted exit code but has missing, malformed, or no mutant outcomes while other shards succeed.

Findings, by severity:

1. **Major — Cleanup can raise and stop the remaining cleanup.** `remote_mutate.py:84` can raise, for example if the SSH executable cannot be started or a second `KeyboardInterrupt` occurs. In the multi-host loops at `remote_mutate.py:171` and `sweep_mutants.py:135`, that skips later hosts and can replace the original exception. Also, the remote command ends in `true` and suppresses Docker errors, so a failed container or image removal can be reported as successful cleanup.

2. **Major — `list` and `baselines` cannot identify their containers during cleanup.** Their `docker run` commands at `sweep_mutants.py:53` and `sweep_mutants.py:77` do not name the containers. If SSH is interrupted while one is running, the name-filtered cleanup at `remote_mutate.py:84` cannot find it. `--rm` removes it only after it exits.

3. **Minor — Some malformed outcome records bypass the shard failure report.** `sweep_mutants.py:86` accepts an outcome with `scenario` but no `summary`; `line 100` then raises before the `FAILED shards` report. This exits nonzero, but does not meet the brief’s reporting requirement.

Prior dispositions:

- **Major 1, run ID — CLOSED.** A 48-bit random UUID suffix is used across the row runner, e2e, list, baselines, and sweep paths; the default sweep output also gets one. See `remote_mutate.py:76` and `sweep_mutants.py:156`. Collisions are possible in principle but no longer follow from same-second starts.
- **Major 2, cleanup — PARTLY.** `finally` now covers normal exceptions and an initial `KeyboardInterrupt`, including partial uploads, but the two major findings above remain. The `fix run` demonstrates an upload failure after the first host, not interruption of a running container or a failure inside cleanup.
- **Major 3, per-shard completeness — CLOSED for the adopted minimum.** `sweep_mutants.py:139` requires at least one mutant outcome per requested shard. The `mixed run` shows 18 populated shards and two empty shards producing a nonzero result. The malformed-record reporting defect remains above; this check does not establish that every assigned mutant ran.
- **`trial.md` wording — CLOSED.** The 400-second cases are called unresolved at `trial.md:14`, and `line 69` no longer asserts that the workspace has no killing test.

This was a read-only inspection; I did not build, format, or run mutation jobs.