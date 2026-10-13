#!/usr/bin/env python3
"""spec 22 DA-3's mechanical check for rewriting accepted specs (plan 31 §4.1).

Compares the accepted texts at a base revision with the working tree and fails when:
- a rule defined at the base — a `- **XX-12a** ` bullet (`TM-16a1` included) or a
  `### KA-3` heading; a citation such as `- **TM-1**:` is not a definition — is no longer
  defined anywhere (it may move between files), or an id is defined more often than at
  the base;
- a test name the base cites is no longer cited and not listed in `--renames`, a renamed
  test is not cited or not a test, or a newly cited name is no test (`#[test] fn` or
  `def test_`) — only the names in KNOWN_MISSING may be cited without existing;
- a relative link does not resolve;
- a mutation row's `old`, or its `old2` after `old` is replaced as `mutate.py` does it,
  does not occur exactly once (a string check; `mutate.py` is not run);
- a file's history markers or lines over 1 500 characters grow outside its closing
  Changes table (the table under a `## Changes` heading; text after the table counts);
- handoff.md is missing or exceeds 300 lines or 40 000 bytes (AGENTS.md §9: handoff holds
  only the present).
A file deleted since the base is compared as empty, so its rules and citations count.

Usage (from the repository root):
    python3 plan/maintenance/tools/rules.py [--base REV] [--renames FILE] [--sentences]
    python3 plan/maintenance/tools/rules.py --self-test

`--renames FILE`: lines `old -> new` (a renamed test) or `old -> -` (dropped by a recorded
decision). `--sentences` (an aid, not a gate) lists base sentences with no head sentence
at a `difflib` ratio of 0.6 or more. Exit 1 when a check fails. Standard library only.
"""
import difflib
import glob
import json
import os
import re
import subprocess
import sys

RID = r"[A-Z]{2,3}-\d+[a-z]?\d*"
DEFINITION = re.compile(rf"^(?:- \*\*({RID})\*\* |#{{2,4}} ({RID})(?: |$))", re.M)
TICKED = re.compile(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`")
TEST_LIKE = re.compile(r"^(?:[a-z]+\d*_\d{2}[a-z0-9]*_|test_)")  # ur_07_…, v58_16_…, sb_49a_…, test_…
LINK = re.compile(r"\]\(([^)#\s]+)(?:#[^)]*)?\)")
MARKER = re.compile(r"\((?:Phase \d+|K[A-Z]-\d+|Review [A-Z]\b|review [A-Z0-9-]+|issue #\d+|owner, 20\d\d)[^)]*\)"
                    r"|\*(?:Settled in|Forward|Superseded)|Amended in Phase|(?<![\w/])#\d{2,3}\b")
LONG = 1500
HANDOFF_LINES, HANDOFF_BYTES = 300, 40000
LISTS = ["maintenance", "phase3", "phase4", "phase5", "phase6", "phase7"]
# Names the accepted specs cited, as history, before plan 31 while no test had them; no
# other name may be cited without existing.
KNOWN_MISSING = {"rs_14_capture_without_recorder_rejected", "sb_22h_the_sink_path_segment_is_reserved",
                 "sc_08_buffer_map_host_none_for_gpu_domain", "se_01_register_adds_the_descriptor_and_both_checks"}


def doc_paths(rev):
    keep = lambda p: p.endswith(".md") and not p.endswith(("v4-vision-audit.md", "v4-vision-rereview.md"))
    here = glob.glob("design/*.md") + glob.glob("design/vision/*.md")
    there = subprocess.run(["git", "ls-tree", "--name-only", rev, "design/", "design/vision/"],
                           capture_output=True, text=True, check=True).stdout.split()
    return sorted({p for p in here + there if keep(p)}) + ["Ez-SDR_v4_ARCHITECTURE_VISION.md"]


def body(text):
    """The text without its closing Changes table: the table rows (and blank lines) under a
    line that is exactly `## Changes`, which hold history by design (DA-1)."""
    lines, out, skipping = text.split("\n"), [], False
    for line in lines:
        if line.strip() == "## Changes":
            skipping = True
            continue
        if skipping and (line.startswith("|") or not line.strip()):
            continue
        skipping = False
        out.append(line)
    return "\n".join(out)


def ids(docs):
    found = {}
    for path, text in docs.items():
        for m in DEFINITION.finditer(text):
            found.setdefault(m[1] or m[2], []).append(path)
    return found


def cited(docs, tests):
    return {n for text in docs.values() for n in TICKED.findall(text) if n in tests or TEST_LIKE.match(n)}


def check(base, head, tests, renames, exists=os.path.exists, known_missing=KNOWN_MISSING, base_tests=frozenset()):
    """Every problem, as text. `base`/`head` map a path to its text (a deleted file is "");
    `tests` is the set of test names in the tree, `base_tests` those at the base (so a test
    deleted with its citation is still seen as cited at the base); `renames` maps an old
    cited name to its new one or to "-"."""
    problems = []
    b_ids, h_ids = ids(base), ids(head)
    for rid in sorted(set(b_ids) - set(h_ids)):
        problems.append(f"rule id {rid} ({b_ids[rid][0]}) is no longer defined: keep it, or a one-line stub")
    for rid, where in sorted(h_ids.items()):
        if len(where) > len(b_ids.get(rid, [1])):
            problems.append(f"rule id {rid} is defined {len(where)} times ({', '.join(where)})")
    known = set(tests) | set(base_tests)
    b_cited, h_cited = cited(base, known), cited(head, known)
    for name in sorted(b_cited - h_cited):
        new = renames.get(name)
        if new is None:
            problems.append(f"cited test {name} is no longer cited: cite it, or list it in --renames")
        elif new != "-" and (new not in h_cited or new not in tests):
            problems.append(f"cited test {name} is renamed to {new}, which is not cited or is no test")
    for name in sorted(h_cited - tests - known_missing):
        problems.append(f"cited test {name} is no #[test] fn or def test_ in the tree")
    for path, text in head.items():
        for target in LINK.findall(text):
            if not target.startswith(("http:", "https:", "mailto:")) and not exists(os.path.normpath(os.path.join(os.path.dirname(path), target))):
                problems.append(f"{path}: link {target} does not resolve")
        for what, count in (("history markers", lambda t: len(MARKER.findall(body(t)))),
                            ("lines over 1 500 characters", lambda t: sum(len(l) > LONG for l in body(t).split("\n")))):
            if count(text) > count(base.get(path, "")):
                problems.append(f"{path}: {what} grew from {count(base.get(path, ''))} to {count(text)}")
    return problems


def handoff_problems(handoff):
    """handoff.md's size limit (AGENTS.md §9); a missing file is None."""
    if handoff is None:
        return ["handoff.md is missing"]
    problems = []
    lines = handoff.count("\n") + (not handoff.endswith("\n") and handoff != "")
    if lines > HANDOFF_LINES or len(handoff.encode("utf-8")) > HANDOFF_BYTES:
        problems.append(f"handoff.md has {lines} lines and {len(handoff.encode('utf-8'))} bytes, over {HANDOFF_LINES} or "
                        f"{HANDOFF_BYTES}: move finished records to their plan files, do not raise the limit")
    return problems


def rows_exact_once(rows_of, read):
    """`rows_of(list)` gives a list's rows, `read(file)` a file's text."""
    problems = []
    for name in LISTS:
        for row in rows_of(name):
            text = read(row["file"])
            n = text.count(row["old"])
            if n != 1:
                problems.append(f"mutation row {name}/{row['id']} old occurs {n} times in {row['file']}")
            elif "old2" in row:
                n2 = text.replace(row["old"], row["new"]).count(row["old2"])
                if n2 != 1:
                    problems.append(f"mutation row {name}/{row['id']} old2 occurs {n2} times in {row['file']} after old is replaced")
    return problems


def test_names_in(texts):
    """Test names in `(path, text)` pairs: `#[test] fn` in Rust, `def test_` in Python."""
    names = set()
    for path, text in texts:
        if path.endswith(".rs"):
            names |= set(re.findall(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*fn\s+([a-z0-9_]+)", text))
        elif path.endswith(".py"):
            names |= set(re.findall(r"def (test_[a-z0-9_]+)", text))
    return names


def test_names():
    paths = glob.glob("crates/**/*.rs", recursive=True) + glob.glob("python/**/*.py", recursive=True)
    return test_names_in((p, open(p, encoding="utf-8").read()) for p in paths)


def test_names_at(rev):
    paths = [p for p in subprocess.run(["git", "ls-tree", "-r", "--name-only", rev, "--", "crates", "python"],
                                       capture_output=True, text=True, check=True).stdout.split()
             if p.endswith((".rs", ".py"))]
    batch = subprocess.run(["git", "cat-file", "--batch"], input="".join(f"{rev}:{p}\n" for p in paths).encode(),
                           capture_output=True, check=True).stdout  # sizes are in bytes
    texts, i = [], 0
    for p in paths:
        header_end = batch.index(b"\n", i)
        size = int(batch[i:header_end].split()[2])
        texts.append((p, batch[header_end + 1:header_end + 1 + size].decode("utf-8")))
        i = header_end + 1 + size + 1
    return test_names_in(texts)


def at(rev, path):
    r = subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


def sentences(base, head):
    split = lambda t: [s.strip() for s in re.split(r"(?<=[.;])\s+", body(t)) if len(s.strip()) > 40]
    for path, text in base.items():
        theirs = split(head.get(path, ""))
        for s in split(text):
            if not difflib.get_close_matches(s, theirs, n=1, cutoff=0.6):
                print(f"{path}: no close head sentence: {s[:160]}")


def self_test():
    base = {"design/01-x.md": "- **TM-1** a rule. *Checked by `tm_01_one`.*\n- **TM-16a1** b.\n### KA-1 — c\n"
                              "| `tm_01_two` | a table row |\n",
            "design/02-y.md": "- **SC-1** d (Phase 2, KA-3).\n- **TM-1**: a citation, not a definition.\n"
                              "\n## Changes\n\n| date | change |\n|---|---|\n| x | (Phase 3, KB-1) " + "x" * 1600 + " |\n"}
    tests = {"tm_01_one", "tm_01_two", "tm_01_three"}
    run = lambda head, renames={}, tests=tests: check(base, head, tests, renames, exists=lambda p: True,
                                                       known_missing={"tm_01_ghost"})
    expect = lambda head, needle, **kw: (lambda got: (any(needle in p for p in got), got))(run(head, **kw))
    assert run(dict(base)) == [], run(dict(base))
    one, two = base["design/01-x.md"], base["design/02-y.md"]
    cases = [
        ({**base, "design/01-x.md": one.replace("- **TM-16a1** b.\n", "")}, "rule id TM-16a1"),
        ({**base, "design/01-x.md": one.replace("### KA-1 — c\n", "")}, "rule id KA-1"),
        ({**base, "design/01-x.md": one.replace("- **TM-1** a rule.", "A rule.")}, "rule id TM-1"),  # only the citation left
        ({**base, "design/01-x.md": ""}, "rule id TM-1"),  # the file deleted
        ({**base, "design/02-y.md": two + "- **TM-16a1** again.\n"}, "defined 2 times"),
        ({**base, "design/01-x.md": one.replace("`tm_01_one`", "a test")}, "cited test tm_01_one"),
        ({**base, "design/01-x.md": one.replace("`tm_01_two`", "a test")}, "cited test tm_01_two"),  # cited in a table
        ({**base, "design/01-x.md": one + "`tm_01_four`\n"}, "tm_01_four is no"),
        ({**base, "design/01-x.md": one + "`test_missing_case`\n"}, "test_missing_case is no"),
        ({**base, "design/02-y.md": two.replace("SC-1** d", "SC-1** d (Phase 3)")}, "history markers grew"),
        ({**base, "design/02-y.md": two + "## Changes pending\n(Phase 4) " + "y" * 1501 + "\n"}, "history markers grew"),
        ({**base, "design/02-y.md": two + "## Changes pending\n(Phase 4) " + "y" * 1501 + "\n"}, "lines over 1 500 characters grew"),
        ({**base, "design/01-x.md": one + "*Test:* `test_missing`.\n"}, "test_missing is no"),
        ({**base, "design/02-y.md": two + "\nA normative sentence after the table (Phase 4).\n"}, "history markers grew"),
    ]
    for head, needle in cases:
        hit, got = expect(head, needle)
        assert hit, f"self-test: expected {needle!r}, got {got}"
    hit, got = expect(dict(base), "tm_01_one is no", tests={"tm_01_two", "tm_01_three"})  # the test itself deleted
    assert hit, f"self-test: a deleted test still cited: {got}"
    # A test that fits no pattern, deleted together with its citation: seen through the base's tests.
    plain = {**base, "design/01-x.md": one + "`captures_across_cold_changes`\n"}
    gone = check(plain, base, {"tm_01_one", "tm_01_two"}, {}, exists=lambda p: True, known_missing=set(),
                 base_tests={"captures_across_cold_changes"})
    assert any("captures_across_cold_changes is no longer cited" in p for p in gone), gone
    renamed = {**base, "design/01-x.md": one.replace("tm_01_one", "tm_01_three")}
    assert expect(renamed, "cited test tm_01_one is no longer cited")[0] and not run(renamed, {"tm_01_one": "tm_01_three"}), "self-test: renames"
    ghost = {**base, "design/01-x.md": one.replace("tm_01_one", "tm_01_ghost")}
    assert expect(ghost, "renamed to tm_01_ghost", renames={"tm_01_one": "tm_01_ghost"})[0], "self-test: rename to a missing test"
    broken = check(base, {**base, "design/02-y.md": "[x](missing.md)\n" + two}, tests, {}, exists=lambda p: False)
    assert any("does not resolve" in p for p in broken), broken
    texts = {"a.rs": "x = 1; y = 1; z = 2;"}
    rows = {"maintenance": [{"id": "Z1", "file": "a.rs", "old": "x = 1", "new": "x = 0"},
                            {"id": "Z2", "file": "a.rs", "old": "= 1", "new": "= 0"},
                            {"id": "Z3", "file": "a.rs", "old": "w = 9", "new": ""},
                            {"id": "Z4", "file": "a.rs", "old": "x = 1", "new": "x = 0", "old2": "y = 1", "new2": ""},
                            {"id": "Z5", "file": "a.rs", "old": "x = 1; y", "new": "q", "old2": "y = 1", "new2": ""},
                            {"id": "Z6", "file": "a.rs", "old": "z = 2", "new": "z = 2; z = 3", "old2": "z = 3", "new2": ""}]}
    got = sorted(rows_exact_once(lambda n: rows.get(n, []), lambda f: texts.get(f, "")))
    want = ["mutation row maintenance/Z2 old occurs 2 times in a.rs",
            "mutation row maintenance/Z3 old occurs 0 times in a.rs",
            "mutation row maintenance/Z5 old2 occurs 0 times in a.rs after old is replaced"]
    assert got == want, got  # Z6's old2 is made by its new: accepted, as mutate.py would
    ok = "x\n" * 300
    assert handoff_problems(ok) == [], "self-test: 300 lines"
    assert handoff_problems(ok + "x"), "self-test: 301 lines, the last without a newline"
    assert handoff_problems("あ" * 13333 + "\n") == [], "self-test: 40 000 bytes"
    assert handoff_problems("あ" * 13334), "self-test: one long line over 40 000 bytes"
    assert handoff_problems(None) == ["handoff.md is missing"], "self-test: missing handoff"
    print("rules.py self-test: ok")


def main():
    args = sys.argv[1:]
    if "--self-test" in args:
        self_test()
        return 0
    base_rev = args[args.index("--base") + 1] if "--base" in args else "HEAD"
    if subprocess.run(["git", "rev-parse", "--verify", "--quiet", f"{base_rev}^{{commit}}"], capture_output=True).returncode != 0:
        print(f"rules.py: {base_rev} is not a commit")
        return 2
    renames = {}
    if "--renames" in args:
        for line in open(args[args.index("--renames") + 1], encoding="utf-8"):
            if "->" in line:
                old, new = (s.strip() for s in line.split("->"))
                renames[old] = new
    paths = doc_paths(base_rev)
    head = {p: open(p, encoding="utf-8").read() if os.path.exists(p) else "" for p in paths}
    base = {p: at(base_rev, p) for p in paths}
    problems = check(base, head, test_names(), renames, base_tests=test_names_at(base_rev))
    problems += rows_exact_once(lambda n: json.load(open(f"plan/{n}/tools/mutations.json", encoding="utf-8")),
                                lambda f: open(f, encoding="utf-8").read() if os.path.exists(f) else "")
    read = lambda f: open(f, encoding="utf-8").read() if os.path.exists(f) else None
    problems += handoff_problems(read("handoff.md"))
    if "--sentences" in args:
        sentences(base, head)
    for p in problems:
        print(p)
    print(f"rules.py: {len(problems)} problem(s) against {base_rev}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
