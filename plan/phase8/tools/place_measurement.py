#!/usr/bin/env python3
"""Place one record-suite output in `crates/ezsdr-radio/measurements/` (D13, stage-4 P-admission).

    place_measurement.py RECORD CONDITIONS [--dest DIR] [--replace] [--args UHD_ARGS] [--reviewed NOTE]
    place_measurement.py --self-test

RECORD is the raw `measurement.json` of a `run-record.sh` run (it has `observations`). CONDITIONS is that session's
`bench/<session>/conditions.json`: its date names the file and its `boards` give the real `selector.args`, which the bench's
records replace with the fake's `type=fake-rig`.

Refuses a bench copy (`bench_copy`), a replacement that fails to serialize leaves the old file, and a record that is not: from hardware, `load: none`, with the keys the code has now, written by the code it has now (the
files that define a record are unchanged between the record's commit and HEAD), the only one for its (board serial, layout)
in DEST (unless --replace). Every observation is printed; `not_compared_*` ones are expected, any other needs --reviewed NOTE
(a stopped ladder, a blocker, a failed trial: the note goes in the bench README). Writes the record with `observations: []`;
only `selector.args` and `observations` change, every other number keeps its source text. Nothing else is placed.
Stdlib only; run with PYTHONDONTWRITEBYTECODE=1.
"""
import argparse
import glob
import json
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
DEST = os.path.join("crates", "ezsdr-radio", "measurements")
# the files that define what a record contains: the record-suite entry point (selects the probes and groups), its writer, the keys and the comparison rule
DEFINES = [os.path.join("crates", "ezsdr-radio-uhd", "tests", "hardware.rs"), os.path.join("crates", "ezsdr-radio-uhd", "tests", "common", "measurement.rs"), os.path.join("crates", "ezsdr-radio-uhd", "tests", "common", "measurement"),
           os.path.join("crates", "ezsdr-radio", "src", "lib.rs"), os.path.join("crates", "ezsdr-radio", "src", "measurement.rs")]
LAYOUT_KEYS = ("rx", "tx", "image", "links", "link_gbps")  # the selector keys that make a layout; `args` and `id` do not


class Refused(Exception):
    pass


class Raw:
    """A JSON float kept as its source text, so that placing a record never changes a measured number."""

    def __init__(self, text):
        self.text = text


def load_raw(path):
    with open(path) as fh:
        return json.load(fh, parse_float=Raw)


def dump_raw(doc):
    found = []

    def hold(value):
        found.append(value.text)
        return f"@@RAW{len(found) - 1}@@"

    out = json.dumps(doc, ensure_ascii=False, indent=1, default=hold)
    if out.count("@@RAW") != len(found):
        raise Refused("the record contains the text @@RAW: the lossless writer cannot tell it from its placeholder")
    for n, text in enumerate(found):
        out = out.replace(f'"@@RAW{n}@@"', text)
    return out


def read(root, *path):
    with open(os.path.join(root, *path)) as fh:
        return fh.read()


def current_keys(root, rx, tx):
    """The comparable value keys for a layout of `rx` and `tx` channels, read from `keys` (lib.rs) and `rule()` (measurement.rs)."""
    lib = read(root, "crates", "ezsdr-radio", "src", "lib.rs")
    keys = lib[lib.index("pub mod keys {"):]
    keys = keys[:keys.index("\n}\n")]
    consts = dict(re.findall(r'pub const (\w+): &str = "([^"]*)";', keys))
    leaves = re.findall(r'"(\w+)"', re.search(r"CHANNEL_LEAVES: \[&str; \d+\] = \[([^\]]*)\]", keys).group(1))
    rule = read(root, "crates", "ezsdr-radio", "src", "measurement.rs")
    rule = rule[rule.index("pub fn rule("):rule.index("pub fn profile_values")]
    skipped = set(re.findall(r"keys::(\w+) => return Some\(NotCompared\)", rule))
    skipped_leaves = set(re.findall(r'leaf: "(\w+)", \.\. \} => Some\(NotCompared\)', rule))
    fixed = {value for name, value in consts.items() if value.startswith("radio.") and name not in skipped}
    for direction, count in (("rx", rx), ("tx", tx)):
        for n in range(count):  # one stream per channel
            fixed |= {f"radio.{direction}.{consts['CHANNEL']}{n}.{leaf}" for leaf in leaves if leaf not in skipped_leaves}
            fixed.add(f"radio.{direction}.{consts['STREAM']}{n}.{consts['STREAM_LEAF']}")
    if len(fixed) < 12 or not skipped or not skipped_leaves:
        raise Refused("could not read the key set from lib.rs and measurement.rs; this script needs updating")
    return fixed


def layout_of(selector):
    return json.dumps({k: selector[k] for k in LAYOUT_KEYS if k in selector}, sort_keys=True, default=lambda raw: raw.text)


def tag_of(selector):
    """`a-obx`, `a-ubx+b-cbx`: the slot and front end of each element (no `#name`), `rx_..._tx_...` only when they differ."""
    def one(elements):
        return "+".join(f"{e.split('#')[0].split(':')[1].lower()}-{e.split('#')[0].split(':')[2]}" for e in elements)
    rx, tx = one(selector.get("rx", [])), one(selector.get("tx", []))
    return rx if rx == tx else f"rx_{rx}_tx_{tx}"


def real_args(conditions, serial):
    for board in conditions.get("boards", []):
        if board.get("serial") == serial:
            if not board.get("addr"):
                break
            return f"addr={board['addr']}" + (f",second_addr={board['second_addr']}" if board.get("second_addr") else "")
    raise Refused(f"no board with serial {serial} and an addr in conditions.json; pass --args (the UHD args the run used)")


def check(record, root):
    """The reasons `record` is not admitted by P-admission (a), (b); empty when it is."""
    why = []
    if record.get("schema") != "ezsdr.radio.measurement.v1":
        why.append(f"schema is {record.get('schema')!r}")
    if record.get("source") != "hardware":
        why.append(f"source is {record.get('source')!r}, not hardware")
    if record.get("load") != "none":
        why.append(f"load is {record.get('load')!r}, not none (P-admission b)")
    selector = record.get("selector", {})
    keys = [v["key"] for v in record.get("values", [])]
    try:
        want = current_keys(root, len(selector.get("rx", [])), len(selector.get("tx", [])))
    except (OSError, ValueError, AttributeError) as error:
        raise Refused(f"cannot read the current key set: {error}")
    if dup := sorted({k for k in keys if keys.count(k) > 1}):
        why.append(f"keys more than once: {dup}")
    if removed := sorted(set(keys) - want):
        why.append(f"keys the code no longer has (removed key): {removed}")
    if missing := sorted(want - set(keys)):
        why.append(f"keys the code has that the record lacks: {missing}")
    if changed := code_changed(record.get("commit"), root):
        why.append(f"the code that defines a record changed since the record's commit {str(record.get('commit'))[:8]}: {changed}")
    return why


def code_changed(commit, root):
    """The DEFINES paths that differ between `commit` and HEAD (a record's code is current when none does); refuses a commit not in HEAD's history."""
    git = lambda *a: subprocess.run(["git", "-C", root, *a], capture_output=True, text=True)
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit) or git("merge-base", "--is-ancestor", commit, "HEAD").returncode != 0:
        raise Refused(f"the record's commit {str(commit)[:12]} is not in this repository's history up to HEAD: the code that wrote it cannot be compared")
    if git("diff", "--quiet", "HEAD", "--", *DEFINES).returncode != 0:
        raise Refused(f"uncommitted changes in {DEFINES}: the key set this tool reads must be HEAD's; commit or stash them first")
    return git("diff", "--name-only", commit, "HEAD", "--", *DEFINES).stdout.split()


def place(record_path, conditions_path, dest, replace=False, args=None, root=ROOT, reviewed=None):
    record = load_raw(record_path)
    with open(conditions_path) as fh:
        conditions = json.load(fh)
    if why := check(record, root):
        raise Refused("not admitted:\n  " + "\n  ".join(why))
    if record.get("bench_copy"):
        raise Refused("the record is a bench copy (bench_extract.py re-parsed its numbers): place the raw measurement.json of the run")
    if "observations" not in record:
        raise Refused("the record has no `observations` field (an old bench copy): place the raw measurement.json of the run")
    observations = record["observations"]
    for o in observations:
        print("  observation: " + dump_raw(o).replace("\n", " "))
    other = [o.get("name") for o in observations if not str(o.get("name")).startswith("not_compared_")]
    if other and not reviewed:
        raise Refused(f"observations other than not_compared_*: {other} (a stopped ladder, a blocker, a failed trial); read them above, take a clean record, or place with --reviewed \"<note>\"")
    selector = record["selector"]
    serial = record["device"]["serial"]
    layout = layout_of(selector)
    same = []
    for path in sorted(glob.glob(os.path.join(dest, "*.json"))):
        placed = load_raw(path)
        if placed["device"]["serial"] == serial and layout_of(placed["selector"]) == layout:
            same.append(path)
    if same and not replace:
        raise Refused(f"{serial} with this layout is already placed ({', '.join(os.path.basename(p) for p in same)}); one per (board, layout): use --replace")
    record["selector"] = {**selector, "args": args or real_args(conditions, serial)}
    record["observations"] = []
    name = f"{conditions['session'][:10]}-{serial}-{tag_of(selector)}.json"
    target = os.path.join(dest, name)
    if os.path.exists(target) and target not in same:
        raise Refused(f"{name} exists with another layout key; not overwritten")
    text = dump_raw(record) + "\n"  # serialized and checked before anything is removed
    os.makedirs(dest, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=dest, suffix=".tmp")
    with os.fdopen(fd, "w") as fh:
        fh.write(text)
    os.chmod(tmp, 0o644)
    os.replace(tmp, target)
    for path in same:
        if path != target:
            os.remove(path)
    print(f"placed {target}\n  board {serial} ({record['device']['mboard']}), layout {layout}\n  selector.args = {record['selector']['args']}, load none, {len(record['values'])} values, observations dropped ({len(observations)})")
    if other:
        print(f"  REVIEWED OBSERVATIONS {other}: {reviewed}  (put this note in the bench README)")
    for path in same:
        print(f"  replaced {os.path.basename(path)}")
    return target


def self_test():
    global code_changed
    real_code_changed = code_changed
    with tempfile.TemporaryDirectory(dir="/tmp") as tmp:
        root = ROOT
        want = current_keys(root, 1, 1)
        assert len(want) == 27 and "radio.perf.tx_bytes_per_s" not in want and "radio.tx.s0.sample_rate_hz" in want and "radio.rx.block_len" not in want and "radio.rx.c0.stream" not in want, sorted(want)
        assert len(current_keys(root, 2, 2)) == 11 + 32
        record = {
            "schema": "ezsdr.radio.measurement.v1", "source": "hardware", "load": "none", "commit": "0" * 40, "observations": [],
            "profile": {"name": "x3xx"}, "device": {"serial": "S1", "mboard": "X300"},
            "selector": {"args": "type=fake-rig", "id": "usrp", "link_gbps": [10], "rx": ["0:A:obx:0"], "tx": ["0:A:obx:0"]},
            "conditions": [{"id": "rx-10msps", "streams": [], "concurrent": [], "reopened": False}, {"id": "restart-retune", "streams": [], "concurrent": [], "reopened": False}],
            "values": [{"status": "unmeasured", "key": k, "reason": "x"} for k in sorted(want)],
        }
        conditions = {"session": "2026-11-27-test", "boards": [{"id": "#1", "serial": "S1", "addr": "192.168.40.36"}]}

        def write(name, doc):
            path = os.path.join(tmp, name)
            with open(path, "w") as fh:
                json.dump(doc, fh)
            return path

        # code_changed on a temporary git repo: unchanged, changed, and a commit outside the history
        repo = os.path.join(tmp, "repo")
        git = lambda *a: subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@t", *a], capture_output=True, text=True, check=True).stdout.strip()
        os.makedirs(os.path.join(repo, "crates", "ezsdr-radio", "src"))
        git("init", "-q")
        lib = os.path.join(repo, DEFINES[3])
        open(lib, "w").write("a\n")
        git("add", "."), git("commit", "-qm", "1")
        first = git("rev-parse", "HEAD")
        open(os.path.join(repo, "other.txt"), "w").write("x\n")
        git("add", "."), git("commit", "-qm", "2")
        assert real_code_changed(first, repo) == []
        open(lib, "w").write("b\n")
        git("commit", "-qam", "3")
        assert real_code_changed(first, repo) == [DEFINES[3]]
        for bad in ("1" * 40, None, "main"):
            try:
                real_code_changed(bad, repo)
            except Refused as error:
                assert "not in this repository" in str(error)
            else:
                raise AssertionError(bad)
        open(lib, "w").write("c\n")  # uncommitted change to a DEFINES path
        try:
            real_code_changed(first, repo)
        except Refused as error:
            assert "uncommitted" in str(error)
        else:
            raise AssertionError("dirty")
        git("checkout", "--", DEFINES[3])
        code_changed = lambda commit, root: ["x"] if commit == "1" * 40 else []  # the rest of the test does not need a repository
        cond = write("conditions.json", conditions)
        dest = os.path.join(tmp, "measurements")

        def refused(doc, expect, **kw):
            try:
                place(write("r.json", doc), kw.pop("conditions", cond), dest, **kw)
            except Refused as error:
                assert expect in str(error), f"expected {expect!r}, got {error}"
                return
            raise AssertionError(f"expected a refusal containing {expect!r}")

        refused({**record, "source": "fake"}, "not hardware")
        refused({**record, "load": "h4"}, "not none")
        refused({**record, "values": record["values"] + [{"status": "unmeasured", "key": "radio.timing.command_queue_depth", "reason": "x"}]}, "removed key")
        refused({**record, "values": record["values"] + [{"status": "unmeasured", "key": "radio.perf.tx_bytes_per_s", "reason": "x"}]}, "removed key")  # NotCompared since D13: a record of the older writer
        refused({**record, "values": record["values"][1:]}, "the record lacks")
        refused({**record, "commit": "1" * 40}, "code that defines a record changed")
        refused({k: v for k, v in record.items() if k != "observations"}, "no `observations` field")
        refused({**record, "bench_copy": True}, "bench copy")
        refused({**record, "observations": [{"name": "not_compared_radio_perf_tx_bytes_per_s"}, {"name": "provider_tx_throughput_200000000"}]}, "other than not_compared_")
        refused(record, "no board with serial S1", conditions=write("c2.json", {"session": "2026-11-27-x", "boards": [{"serial": "S2", "addr": "a"}]}))
        assert not os.path.exists(dest), "a refusal must not create the destination"
        # not_compared_* observations need nothing; any other needs a note; all are dropped from the placed record
        target = place(write("r.json", {**record, "observations": [{"name": "not_compared_a", "row": {"v": 1.5}}, {"name": "n"}]}), cond, dest, reviewed="blocker is a known stall")
        placed = json.load(open(target))
        assert os.path.basename(target) == "2026-11-27-S1-a-obx.json", target
        assert placed["observations"] == [] and placed["selector"]["args"] == "addr=192.168.40.36" and placed["selector"]["id"] == "usrp", placed["selector"]
        assert oct(os.stat(target).st_mode & 0o777) == "0o644"
        refused(record, "already placed")
        # a refused replacement leaves the old file intact
        (old,) = glob.glob(os.path.join(dest, "*.json"))
        old_text = open(old).read()
        refused({**record, "note": "@@RAW0@@", "values": [{**record["values"][0], "x": 1.5}] + record["values"][1:]}, "@@RAW", replace=True)
        assert open(old).read() == old_text and glob.glob(os.path.join(dest, "*.tmp")) == []
        place(write("r.json", record), cond, dest, replace=True, args="addr=1.2.3.4")
        assert [json.load(open(p))["selector"]["args"] for p in glob.glob(os.path.join(dest, "*.json"))] == ["addr=1.2.3.4"]
        # nothing but selector.args and observations changes: a float of 17 digits and a long exponent keep their text
        text = json.dumps({**record, "observations": [{"name": "not_compared_n"}]}).replace('"reason": "x"}', '"reason": "x", "worst": 9007199254740993.0, "t": 1.0e-7, "n": 5}', 1)
        assert "9007199254740993.0" in text
        with open(os.path.join(tmp, "exact.json"), "w") as fh:
            fh.write(text)
        place(os.path.join(tmp, "exact.json"), cond, dest, replace=True)
        (placed_file,) = glob.glob(os.path.join(dest, "*.json"))
        out = open(placed_file).read()
        assert "9007199254740993.0" in out and "1.0e-7" in out, out[:300]
        strs = lambda path_or_text, is_text=False: json.loads(path_or_text if is_text else open(path_or_text).read(), parse_float=str)
        before, after = strs(text, True), strs(placed_file)
        assert after["selector"].pop("args") != before["selector"].pop("args") and after.pop("observations") == [] and before.pop("observations")
        assert before == after
        # a string that looks like the lossless writer's placeholder is refused, not turned into a number
        try:
            dump_raw({"note": "@@RAW0@@", "value": Raw("1.5")})
        except Refused as error:
            assert "@@RAW" in str(error)
        else:
            raise AssertionError("placeholder collision")
        assert dump_raw({"v": Raw("1.50")}) == '{\n "v": 1.50\n}'
        other = {**record, "device": {"serial": "S2", "mboard": "X300"}}  # another board, same layout: placed beside
        place(write("r2.json", other), write("c3.json", {"session": "2026-11-27-x", "boards": [{"serial": "S2", "addr": "b"}]}), dest)
        assert len(glob.glob(os.path.join(dest, "*.json"))) == 2
    code_changed = real_code_changed
    print("self-test ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("record", nargs="?")
    parser.add_argument("conditions", nargs="?")
    parser.add_argument("--dest", default=os.path.join(ROOT, DEST))
    parser.add_argument("--replace", action="store_true", help="replace the placed record of the same (board, layout)")
    parser.add_argument("--args", help="the UHD args the run used, instead of the board's addr in conditions.json (XG: addr=...,second_addr=...)")
    parser.add_argument("--reviewed", metavar="NOTE", help="place a record that has observations other than not_compared_*; the note is printed, and goes in the bench README step")
    parser.add_argument("--self-test", action="store_true")
    opts = parser.parse_args()
    if opts.self_test:
        return self_test()
    if not (opts.record and opts.conditions):
        parser.error("RECORD and CONDITIONS are required")
    try:
        place(opts.record, opts.conditions, opts.dest, opts.replace, opts.args, reviewed=opts.reviewed)
    except Refused as error:
        sys.exit(f"place_measurement: {error}")


if __name__ == "__main__":
    main()
