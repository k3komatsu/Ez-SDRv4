#!/usr/bin/env python3
"""Per first-parent commit: changed lines by category, test functions added/removed/renamed/changed,
mutation rows added/removed/changed. Read-only on the repository (git show / git diff only).

Usage (rerun):  python3 -I churn.py <repo> <from-exclusive> <to>  > churn.jsonl
                e.g. python3 -I churn.py ~/local_work/Ez-SDRv4-n111 ec95e4b2 4679abc1

Categories of a changed line (Rust lines by position, at the side's own revision):
  prod      crates/*/src, outside any cfg(test)/testing-gated item, not a cfg(test)-only file,
            not FakeDevice's region of device.rs, not brain/, not uhd.rs
  uhdffi    crates/ezsdr-radio-uhd/src/uhd.rs outside cfg(test) (feature `uhd`, libuhd FFI)
  fake      FakeDevice's region of crates/ezsdr-radio-uhd/src/device.rs (from `pub enum FakeFault`
            to the first top-level #[cfg(test)]), and items gated by feature "testing"/"test-support"
            or cfg(any(test, ...)) (test infrastructure compiled into a feature)
  utest     inline unit tests: items gated by #[cfg(test)] and files that are cfg(test)-only modules
  itest     crates/*/tests/** except tests/common/**
  icommon   crates/*/tests/common/** (rigs, helpers, bench measurement tools) and ezsdr-acceptance/src
  brain     crates/ezsdr-radio-uhd/src/brain/** (note 45 experiment, added then deleted)
  py, pytest  python/ezsdr/** , python/tests/**
  mut       plan/*/tools/mutations.json
  tools     plan/maintenance/tools/** (other), docker/**, scripts
  design    design/**, Ez-SDR_v4_ARCHITECTURE_VISION.md, KNOWN_LIMITATIONS.md
  schema    schemas/**
  records   plan/**, handoff.md, AGENTS.md, bench/**, CLAUDE.md, README*
  other     anything else
"""
import json
import re
import subprocess
import sys
from collections import Counter, defaultdict

REPO = sys.argv[1]
FROM, TO = sys.argv[2], sys.argv[3]


def git(*args, ok_fail=False):
    p = subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True, errors="replace")
    if p.returncode != 0:
        if ok_fail:
            return None
        raise RuntimeError(f"git {args}: {p.stderr}")
    return p.stdout


_show_cache = {}


def show(rev, path):
    key = (rev, path)
    if key not in _show_cache:
        _show_cache[key] = git("show", f"{rev}:{path}", ok_fail=True)
    return _show_cache[key]


def sanitize(text):
    """Replace comments, string and char literal contents by spaces (newlines kept)."""
    out = []
    i, n = 0, len(text)
    block = 0
    while i < n:
        c = text[i]
        if block:
            if text.startswith("*/", i):
                block -= 1
                out.append("  ")
                i += 2
            elif text.startswith("/*", i):
                block += 1
                out.append("  ")
                i += 2
            else:
                out.append("\n" if c == "\n" else " ")
                i += 1
            continue
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
            continue
        if text.startswith("/*", i):
            block = 1
            out.append("  ")
            i += 2
            continue
        m = re.match(r'b?r(#*)"', text[i:i + 40]) if c in "rb" and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")) else None
        if m:
            hashes = m.group(1)
            end = text.find('"' + hashes, i + m.end())
            end = n if end < 0 else end + 1 + len(hashes)
            seg = text[i:end]
            out.append("".join("\n" if ch == "\n" else " " for ch in seg))
            i = end
            continue
        if c == '"' or (c == "b" and text.startswith('b"', i)):
            j = i + (2 if c == "b" else 1)
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            seg = text[i:j]
            out.append("".join("\n" if ch == "\n" else " " for ch in seg))
            i = j
            continue
        if c == "'":
            m = re.match(r"'(\\(x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'", text[i:i + 16])
            if m:
                out.append(" " * m.end())
                i += m.end()
                continue
        out.append(c)
        i += 1
    return "".join(out)


ATTR_TEST = re.compile(r"^\s*#\[cfg\((all\()?test\b")
ATTR_FAKE = re.compile(r'^\s*#\[cfg\((any\(test\b|feature\s*=\s*"(testing|test-support)"|any\(feature\s*=\s*"testing")')


def item_end(slines, j):
    """Index of the last line of the item starting at line j (sanitized lines)."""
    depth, opened = 0, False
    k = j
    n = len(slines)
    while k < n:
        for ch in slines[k]:
            if ch == "{":
                depth += 1
                opened = True
            elif ch == "}":
                depth -= 1
                if opened and depth == 0:
                    return k
            elif ch in ";," and depth == 0 and not opened:
                return k
        k += 1
    return n - 1


def region_map(path, text, rev):
    """Per-line category for one Rust file under crates/*/src."""
    lines = text.split("\n")
    n = len(lines)
    if "/src/brain/" in path:
        return ["brain"] * n
    if path.startswith("crates/ezsdr-acceptance/src/"):
        return ["icommon"] * n
    if is_test_file(path, rev):
        return ["utest"] * n
    base = "uhdffi" if path.endswith("ezsdr-radio-uhd/src/uhd.rs") else "prod"
    cat = [base] * n
    slines = sanitize(text).split("\n")
    if path.endswith("ezsdr-radio-uhd/src/device.rs"):
        start = next((i for i, l in enumerate(lines) if re.match(r"^pub enum FakeFault\b", l)), None)
        if start is not None:
            k = start
            while k > 0 and (lines[k - 1].startswith("///") or lines[k - 1].startswith("#[")):
                k -= 1
            end = next((i for i in range(start, n) if re.match(r"^#\[cfg\(test\)\]", lines[i])), n)
            for i in range(k, end):
                cat[i] = "fake"
    i = 0
    while i < n:
        raw = lines[i]
        kind = "utest" if ATTR_TEST.match(raw) else ("fake" if ATTR_FAKE.match(raw) else None)
        if kind and "not(test)" not in raw:
            j = i + 1
            while j < n and (lines[j].strip().startswith("#[") or lines[j].strip().startswith("///") or lines[j].strip() == ""):
                j += 1
            e = item_end(slines, j) if j < n else i
            for k in range(i, e + 1):
                if cat[k] != "fake" or kind == "utest":
                    cat[k] = kind
            i = e + 1
            continue
        i += 1
    return cat


_test_file_cache = {}


def parent_module(path):
    """(parent file candidates, module name) for a Rust file under crates/X/src."""
    m = re.match(r"^(crates/[^/]+/src)/(.*)$", path)
    root, rel = m.group(1), m.group(2)
    parts = rel.split("/")
    if rel in ("lib.rs", "main.rs") or parts[0] == "bin":
        return [], None
    if parts[-1] == "mod.rs":
        name = parts[-2]
        dirs = parts[:-2]
    else:
        name = parts[-1][:-3]
        dirs = parts[:-1]
    if not dirs:
        return [f"{root}/lib.rs", f"{root}/main.rs"], name
    d = "/".join(dirs)
    return [f"{root}/{d}.rs", f"{root}/{d}/mod.rs"], name


def is_test_file(path, rev):
    key = (path, rev)
    if key in _test_file_cache:
        return _test_file_cache[key]
    m = re.match(r"^crates/[^/]+/src/(.*)$", path)
    result = False
    if m and ("/tests/" in "/" + m.group(1) or m.group(1).startswith("tests/")):
        result = True
    elif m:
        cands, name = parent_module(path)
        for cand in cands:
            t = show(rev, cand)
            if t is None:
                continue
            lines = t.split("\n")
            pat = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+" + re.escape(name) + r"\s*;")
            for idx, l in enumerate(lines):
                if pat.match(l):
                    k = idx - 1
                    while k >= 0 and lines[k].strip().startswith("#["):
                        if ATTR_TEST.match(lines[k]):
                            result = True
                        k -= 1
                    if not result:
                        cmap = region_map(cand, t, rev)
                        result = cmap[idx] == "utest"
                    break
            break
    _test_file_cache[key] = result
    return result


def path_category(path):
    if path.endswith("/mutations.json") and path.startswith("plan/"):
        return "mut"
    if path.startswith("crates/"):
        if re.match(r"^crates/[^/]+/tests/common/", path):
            return "icommon"
        if re.match(r"^crates/[^/]+/tests/", path):
            return "itest"
        if re.match(r"^crates/[^/]+/src/", path) and path.endswith(".rs"):
            return None  # by position
        if re.match(r"^crates/ezsdr-acceptance/", path):
            return "icommon"
        if re.match(r"^crates/[^/]+/measurements/", path):
            return "records"
        return "other"
    if path.startswith("python/"):
        return "pytest" if path.startswith("python/tests/") else ("py" if path.startswith("python/ezsdr/") else "other")
    if path.startswith("plan/maintenance/tools/") or path.startswith("docker/") or path.startswith("scripts/"):
        return "tools"
    if path.startswith("design/") or path in ("Ez-SDR_v4_ARCHITECTURE_VISION.md", "KNOWN_LIMITATIONS.md"):
        return "design"
    if path.startswith("schemas/"):
        return "schema"
    if path.startswith("plan/") or path.startswith("bench/") or path in ("handoff.md", "AGENTS.md", "CLAUDE.md") or path.startswith("README"):
        return "records"
    return "other"


HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


def diff_lines(parent, commit):
    """{(old_path, new_path): {'-': [old line numbers], '+': [new line numbers]}}"""
    out = git("diff", "-U0", "--no-color", "-M", "--no-ext-diff", parent, commit)
    files = {}
    cur = None
    old_path = new_path = None
    o = nl = 0
    for line in out.split("\n"):
        if line.startswith("diff --git "):
            cur = None
            old_path = new_path = None
            continue
        if line.startswith("--- "):
            old_path = None if line[4:] == "/dev/null" else line[6:]
            continue
        if line.startswith("+++ "):
            new_path = None if line[4:] == "/dev/null" else line[6:]
            cur = files.setdefault((old_path, new_path), {"-": [], "+": []})
            continue
        m = HUNK.match(line)
        if m:
            o = int(m.group(1))
            nl = int(m.group(3))
            continue
        if cur is None or not line:
            continue
        if line.startswith("-"):
            cur["-"].append(o)
            o += 1
        elif line.startswith("+"):
            cur["+"].append(nl)
            nl += 1
    return files


TEST_ATTR = re.compile(r"^\s*#\[(test|tokio::test)\]")
FN = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(async\s+)?fn\s+(\w+)")


def norm(s):
    s = re.sub(r"\s+", "", s)
    return re.sub(r",([)}\]])", r"\1", s)


def rust_tests(text):
    """{name: normalized body} of #[test] functions."""
    lines = text.split("\n")
    sl = sanitize(text).split("\n")
    tests = {}
    i = 0
    n = len(lines)
    while i < n:
        if TEST_ATTR.match(lines[i]):
            j = i + 1
            while j < n and not FN.match(lines[j]):
                j += 1
            if j >= n:
                break
            name = FN.match(lines[j]).group(4)
            e = item_end(sl, j)
            body = "\n".join(lines[j + 1:e + 1])
            tests[name] = norm(body)
            i = e + 1
            continue
        i += 1
    return tests


def py_tests(text):
    lines = text.split("\n")
    tests = {}
    for i, l in enumerate(lines):
        m = re.match(r"^(\s*)def (test_\w+)\(", l)
        if m:
            ind = len(m.group(1))
            body = []
            for l2 in lines[i + 1:]:
                if l2.strip() and len(l2) - len(l2.lstrip()) <= ind:
                    break
                body.append(l2)
            tests[m.group(2)] = norm("\n".join(body))
    return tests


def crate_of(path):
    m = re.match(r"^crates/([^/]+)/", path)
    return m.group(1) if m else ("python" if path.startswith("python/") else None)


MUT_FIELDS_ANCHOR = ("file", "old", "old2", "scope")
MUT_FIELDS_MUTANT = ("new", "new2")
MUT_FIELDS_KILLER = ("filter", "test_bin", "python", "crate", "kill", "equivalent")


def mut_diff(parent, commit, path):
    a = show(parent, path)
    b = show(commit, path)
    try:
        ra = {r["id"]: r for r in json.loads(a)} if a else {}
        rb = {r["id"]: r for r in json.loads(b)} if b else {}
    except ValueError:
        return {"error": path}
    res = Counter()
    res["added"] = len(rb.keys() - ra.keys())
    res["removed"] = len(ra.keys() - rb.keys())
    for k in ra.keys() & rb.keys():
        x, y = ra[k], rb[k]
        if x == y:
            continue
        res["changed"] += 1
        if any(x.get(f) != y.get(f) for f in MUT_FIELDS_ANCHOR):
            res["anchor"] += 1
        if any(x.get(f) != y.get(f) for f in MUT_FIELDS_MUTANT):
            res["mutant"] += 1
        if any(x.get(f) != y.get(f) for f in MUT_FIELDS_KILLER):
            res["killer"] += 1
    res["rows_after"] = len(rb)
    return dict(res)


def main():
    commits = git("rev-list", "--first-parent", "--reverse", f"{FROM}..{TO}").split()
    for c in commits:
        parent = git("rev-parse", f"{c}^1").strip()
        meta = git("show", "-s", "--format=%h%x00%ad%x00%s", "--date=short", c).strip().split("\x00")
        files = diff_lines(parent, c)
        cats = defaultdict(lambda: [0, 0])  # [added, deleted]
        per_crate = defaultdict(lambda: defaultdict(lambda: [0, 0]))
        tests_old, tests_new = defaultdict(dict), defaultdict(dict)
        mut = {}
        added_records = []
        for (op, np_), ch in files.items():
            for side, path, rev in (("-", op, parent), ("+", np_, c)):
                if path is None or not ch[side]:
                    continue
                pc = path_category(path)
                if pc is None:
                    text = show(rev, path) or ""
                    cmap = region_map(path, text, rev)
                    for ln in ch[side]:
                        cat = cmap[ln - 1] if 0 < ln <= len(cmap) else "prod"
                        cats[cat][0 if side == "+" else 1] += 1
                        per_crate[crate_of(path)][cat][0 if side == "+" else 1] += 1
                else:
                    cats[pc][0 if side == "+" else 1] += len(ch[side])
                    if crate_of(path):
                        per_crate[crate_of(path)][pc][0 if side == "+" else 1] += len(ch[side])
            # tests
            for side, path, rev, store in (("-", op, parent, tests_old), ("+", np_, c, tests_new)):
                pass
            for path, rev, store in ((op, parent, tests_old), (np_, c, tests_new)):
                if path is None:
                    continue
                if path.endswith(".rs") and path.startswith("crates/"):
                    text = show(rev, path) or ""
                    if "#[test]" in text:
                        for k, v in rust_tests(text).items():
                            store[crate_of(path)][k] = v
                elif path.startswith("python/tests/") and path.endswith(".py"):
                    text = show(rev, path) or ""
                    for k, v in py_tests(text).items():
                        store["python"][k] = v
            if np_ and np_.endswith("/mutations.json") and np_.startswith("plan/"):
                mut[np_] = mut_diff(parent, c, np_)
            elif op and op.endswith("/mutations.json") and op.startswith("plan/") and np_ is None:
                mut[op] = mut_diff(parent, c, op)
            if op is None and np_ and np_.startswith("plan/phase8/reviews/"):
                added_records.append(np_.rsplit("/", 1)[1])
        t = Counter()
        for crate in set(tests_old) | set(tests_new):
            a, b = tests_old.get(crate, {}), tests_new.get(crate, {})
            removed = {k: v for k, v in a.items() if k not in b}
            added = {k: v for k, v in b.items() if k not in a}
            bodies_added = Counter(added.values())
            renamed = 0
            for v in removed.values():
                if bodies_added[v] > 0:
                    bodies_added[v] -= 1
                    renamed += 1
            t["added"] += len(added) - renamed
            t["removed"] += len(removed) - renamed
            t["renamed"] += renamed
            t["changed"] += sum(1 for k in a.keys() & b.keys() if a[k] != b[k])
        print(json.dumps({"sha": meta[0], "date": meta[1], "subject": meta[2],
                          "cats": {k: v for k, v in sorted(cats.items())},
                          "crates": {cr: {k: v for k, v in d.items()} for cr, d in per_crate.items() if cr},
                          "tests": dict(t), "mut": mut, "records": sorted(added_records)}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
