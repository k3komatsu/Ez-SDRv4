#!/usr/bin/env python3
"""Checks every relative Markdown link in design/, plan/, AGENTS.md and handoff.md,
resolving each from the file that contains it (20-implementation-plan.md Step 5, Step 7).

Usage, from the repository root:
    python3 plan/phase3/tools/check_links.py

Prints `BROKEN <file>: <target>` for each link whose target does not exist and exits 1
if there is any; prints `ok: <n> links` otherwise. Links inside fenced code blocks and
inline code are ignored, as are URLs and pure `#anchor` links. Standard library only.
"""
import os
import re
import sys

LINK = re.compile(r"\]\(([^)\s]+)\)")


def markdown_files(root):
    for top in ("design", "plan"):
        for directory, _, names in os.walk(os.path.join(root, top)):
            for name in names:
                if name.endswith(".md"):
                    yield os.path.join(directory, name)
    for name in ("AGENTS.md", "handoff.md", "Ez-SDR_v4_ARCHITECTURE_VISION.md"):
        if os.path.isfile(os.path.join(root, name)):
            yield os.path.join(root, name)


def links(text):
    fenced = False
    for line in text.splitlines():
        if line.lstrip().startswith("```"):
            fenced = not fenced
            continue
        if fenced:
            continue
        line = re.sub(r"`[^`]*`", "", line)
        for target in LINK.findall(line):
            yield target


def main():
    root = os.getcwd()
    broken = 0
    count = 0
    for path in markdown_files(root):
        text = open(path, encoding="utf-8").read()
        for target in links(text):
            if re.match(r"^[a-z]+:", target) or target.startswith("#"):
                continue
            count += 1
            relative = target.split("#", 1)[0]
            if not os.path.exists(os.path.normpath(os.path.join(os.path.dirname(path), relative))):
                broken += 1
                print(f"BROKEN {os.path.relpath(path, root)}: {target}")
    if broken:
        return 1
    print(f"ok: {count} links")
    return 0


if __name__ == "__main__":
    sys.exit(main())
