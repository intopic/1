#!/usr/bin/env python3
"""Fails if a relative Markdown link in the repo points to a missing file (audit F-06).

Usage: python3 -I tools/check_md_links.py [root]
"""
import os
import re
import sys

LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
SKIP = {".git", "target", "node_modules", ".tools"}


def main() -> int:
    root = sys.argv[1] if len(sys.argv) > 1 else "."
    bad = []
    for d, dirs, files in os.walk(root):
        dirs[:] = [x for x in dirs if x not in SKIP]
        for f in files:
            if not f.endswith(".md"):
                continue
            path = os.path.join(d, f)
            for n, line in enumerate(open(path, encoding="utf-8"), 1):
                for target in LINK.findall(line):
                    if re.match(r"^[a-z]+:", target) or target.startswith("#"):
                        continue
                    t = target.split("#", 1)[0]
                    if t and not os.path.exists(os.path.join(d, t)):
                        bad.append(f"{path}:{n}: {target}")
    for b in bad:
        print("broken link:", b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
