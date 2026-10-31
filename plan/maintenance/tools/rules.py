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
- a relative link does not resolve, in the accepted texts or in any other tracked Markdown file
  (links inside code spans and fenced blocks are text, not links);
- a mutation row's `old`, or its `old2` after `old` is replaced, does not match exactly once
  with the runner's anchor matcher (`mutate.py` is not run);
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
import anchor

RID = r"[A-Z]{2,3}-\d+[a-z]?\d*"
DEFINITION = re.compile(rf"^(?:- \*\*({RID})\*\* |#{{2,4}} ({RID})(?: |$))", re.M)
TICKED = re.compile(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`")
TEST_LIKE = re.compile(r"^(?:[a-z]+\d*_\d{2}[a-z0-9]*_|test_)")  # ur_07_…, v58_16_…, sb_49a_…, test_…
LINK = re.compile(r"\]\(([^)#\s]+)(?:#[^)]*)?\)")
CODE = re.compile(r"```.*?```|`[^`\n]*`", re.S)
MARKER = re.compile(r"\((?:Phase \d+|K[ABD-Z]-\d+|Review [A-Z]\b|review [A-Z0-9-]+|issue #\d+|owner, 20\d\d)[^)]*\)"
                    r"|\*(?:Settled in|Forward|Superseded)|Amended in Phase|(?<![\w/])#\d{2,3}\b")
LONG = 1500
HANDOFF_LINES, HANDOFF_BYTES = 300, 40000
# The OV and PO rules still bind (the accepted specs cite them) although they live in plan/,
# among history: only their ids are protected there.
LIVE_PLAN = ["plan/phase1/00-overview.md", "plan/phase2/00-overview.md"]
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
    return sorted({p for p in here + there if keep(p)}) + ["Ez-SDR_v4_ARCHITECTURE_VISION.md"] + LIVE_PLAN


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


def id_problems(base, head):
    problems = []
    b_ids, h_ids = ids(base), ids(head)
    for rid in sorted(set(b_ids) - set(h_ids)):
        problems.append(f"rule id {rid} ({b_ids[rid][0]}) is no longer defined: keep it, or a one-line stub")
    for rid, where in sorted(h_ids.items()):
        if len(where) > len(b_ids.get(rid, [1])):
            problems.append(f"rule id {rid} is defined {len(where)} times ({', '.join(where)})")
    return problems


def check(base, head, tests, renames, exists=os.path.exists, known_missing=KNOWN_MISSING, base_tests=frozenset()):
    """Every problem, as text. `base`/`head` map a path to its text (a deleted file is "");
    `tests` is the set of test names in the tree, `base_tests` those at the base (so a test
    deleted with its citation is still seen as cited at the base); `renames` maps an old
    cited name to its new one or to "-"."""
    problems = id_problems(base, head)
    base, head = ({p: t for p, t in d.items() if p not in LIVE_PLAN} for d in (base, head))  # ids only there
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
        problems += broken_links(path, text, exists)
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


def broken_links(path, text, exists=os.path.exists):
    """The relative links of one Markdown file that do not resolve; code is not searched."""
    return [f"{path}: link {target} does not resolve" for target in LINK.findall(CODE.sub("", text))
            if not target.startswith(("http:", "https:", "mailto:"))
            and not exists(os.path.normpath(os.path.join(os.path.dirname(path), target)))]


def rows_exact_once(rows_of, read):
    """`rows_of(list)` gives a list's rows, `read(file)` a file's text."""
    problems = []
    texts = {}
    for name in LISTS:
        for row in rows_of(name):
            if row["file"] not in texts:
                texts[row["file"]] = read(row["file"])
            try:
                anchor.apply(texts[row["file"]], row)
            except anchor.AnchorError as error:
                problems.append(f"mutation row {name}/{row['id']} {error} in {row['file']}")
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
            "mutation row maintenance/Z5 old2 occurs 0 times after old is replaced in a.rs"]
    assert got == want, got  # Z6's old2 is made by its new: accepted, as mutate.py would
    rust = 'fn first(){ let x=1; } fn second() { let x = 1; }'
    assert len(anchor.find(rust, "let x = 1;", path="a.rs")) == 2, "self-test: Rust whitespace-insensitive matches"
    assert len(anchor.find(rust, "let x=1;", scope="fn second", path="a.rs")) == 1, "self-test: scope disambiguates"
    numbers = "1 0xff 0o77 0b10 1.5 1e-6 2.5E+3 1u32 1.0f64 1_000"
    assert [token.value for token in anchor._tokens(numbers)] == numbers.split(), \
        "self-test: numeric literals, exponents, and suffixes are indivisible"
    for source, partial, complete in (("1e-6", "6", "1e-6"),
                                      ("r#name", "name", "r#name"),
                                      ("αname", "name", "αname"),
                                      ("1.5", "5", "1.5")):
        assert not anchor.find(source, partial, path="a.rs"), \
            f"self-test: partial token {partial!r} matched in {source!r}"
        assert len(anchor.find(source, complete, path="a.rs")) == 1, \
            f"self-test: complete token {complete!r} did not match"
    assert [token.value for token in anchor._tokens("1..2")] == ["1", ".", ".", "2"], \
        "self-test: a range is not a fractional literal"
    assert [token.value for token in anchor._tokens("t.0.1")] == ["t", ".", "0", ".", "1"], \
        "self-test: tuple indices after dots remain integer tokens"
    assert [token.value for token in anchor._tokens("1.max")] == ["1", ".", "max"], \
        "self-test: a method call on an integer is not a fractional literal"
    for invalid in ("0b102", "1e-", "١"):
        try:
            anchor._tokens(invalid)
        except anchor.AnchorError:
            pass
        else:
            raise AssertionError(f"self-test: unsupported partial tokenization of {invalid!r}")
    for scope, scoped, count in (("fn absent", rust, 0),
                                 ("fn first", rust + " fn first() { let x=1; }", 2)):
        try:
            anchor.find(scoped, "let x=1;", scope=scope, path="a.rs")
        except anchor.AnchorError as error:
            assert f"scope matches {count} times" in str(error), error
        else:
            raise AssertionError(f"self-test: expected invalid scope {scope!r}")
    quoted = 'fn f(){ let s = "a  b"; /* one  two */ x(); }'
    assert len(anchor.find(quoted, 'let s="a  b";', path="a.rs")) == 1, "self-test: whitespace around tokens"
    assert not anchor.find(quoted, 'let s = "a b";', path="a.rs"), "self-test: string whitespace is significant"
    assert len(anchor.find(quoted, "/* one two */ x();", path="a.rs")) == 1, "self-test: comment whitespace collapses"
    braces = ('impl TxChain { const OPEN: &str = "{"; // } { don\'t close the block\n'
              '/* outer { /* nested } { */ still } */ fn close_device_burst() { action(); } }')
    assert len(anchor.find(braces, "action();", scope="impl TxChain", path="a.rs")) == 1, \
        "self-test: braces in strings and nested comments do not end scope"
    nested = "fn outer() { match value { inner(); } action(); }"
    assert len(anchor.find(nested, "action();", scope="fn outer() {", path="a.rs")) == 1, \
        "self-test: a scope ending with its opening brace selects that block"
    try:
        anchor.find(braces + " impl TxChain {}", "action();", scope="impl TxChain", path="a.rs")
    except anchor.AnchorError as error:
        assert "scope matches 2 times" in str(error), error
    else:
        raise AssertionError("self-test: ambiguous scope")
    assert not anchor.find("x  = 1", "x = 1", path="a.py"), "self-test: Python anchors stay exact"
    assert anchor.find("x = 1", "x = 1", path="a.py") == [(0, 5)], "self-test: Python exact span"
    raw = 'fn raw() { let s = r##"{ a  b }"##; let b = br#"{x}"#; }'
    assert len(anchor.find(raw, 'r##"{ a  b }"##', path="a.rs")) == 1, \
        "self-test: hashed raw-string contents stay one exact token"
    assert len(anchor.find(raw, 'br#"{x}"#', scope="fn raw", path="a.rs")) == 1, \
        "self-test: raw byte string braces do not affect scope"
    literals = "fn f<'a>() { let c = 'a'; let q = '\\''; let b = b'x'; }"
    assert len(anchor.find(literals, "'a", path="a.rs")) == 1, \
        "self-test: a lifetime does not match a char literal"
    assert len(anchor.find(literals, "'a'", path="a.rs")) == 1, \
        "self-test: char literals are one token"
    assert len(anchor.find(literals, "'\\''", path="a.rs")) == 1, \
        "self-test: escaped char literals are one token"
    assert len(anchor.find(literals, "b'x'", path="a.rs")) == 1, \
        "self-test: byte char literals are one token"
    assert not anchor.find("let x = 1; /* comment tail */", "let x = 1; /* comment", path="a.rs"), \
        "self-test: a partial block-comment anchor does not match"
    edited = anchor.apply(texts["a.rs"], rows["maintenance"][3])
    assert "y = 1" not in edited, "self-test: old2 is applied after old"
    assert not run({**base, "design/02-y.md": two.replace("SC-1** d", "SC-1** d (KC-24)")}), "self-test: a KC citation is no marker"
    live = {**base, LIVE_PLAN[0]: "- **OV-1** a rule.\n"}
    assert any("rule id OV-1 " in p for p in check(live, {**live, LIVE_PLAN[0]: "- OV-1 a rule.\n"}, tests, {})), "self-test: a live plan id"
    assert not check(live, {**live, LIVE_PLAN[0]: "- **OV-1** a rule (Phase 3) `xx_01_gone`.\n"}, tests, {}), "self-test: ids only in a live plan file"
    assert LIVE_PLAN[0] in doc_paths("HEAD"), "self-test: the live plan files are read"
    assert broken_links("plan/x.md", "[a](gone.md) `[b](gone.md)`\n```\n[c](gone.md)\n```\n", lambda p: False) == \
        ["plan/x.md: link gone.md does not resolve"], "self-test: links in code are text"
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
    tracked = subprocess.run(["git", "ls-files", "*.md"], capture_output=True, text=True, check=True).stdout.split()
    for path in sorted(set(tracked) - set(paths)):
        problems += broken_links(path, read(path) or "")
    if "--sentences" in args:
        sentences(base, head)
    for p in problems:
        print(p)
    print(f"rules.py: {len(problems)} problem(s) against {base_rev}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
