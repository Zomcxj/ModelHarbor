#!/usr/bin/env python3
"""Rewrite `self.status = <expr>;` into `self.set_status(<expr>);`.

Statement scan is brace/paren/bracket- and literal-aware, so `format!(...)` spans and the
`if ... { .. } else { .. }` expression form both survive intact.
"""
from __future__ import annotations

import pathlib
import sys

NEEDLE = "self.status = "


def end_of_statement(src: str, i: int) -> int:
    """Index of the `;` ending the statement starting at `i`."""
    depth = 0
    n = len(src)
    while i < n:
        c = src[i]
        if c == '"':
            i += 1
            while i < n and src[i] != '"':
                i += 2 if src[i] == "\\" else 1
        elif c == "/" and i + 1 < n and src[i + 1] == "/":
            while i < n and src[i] != "\n":
                i += 1
            continue
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == ";" and depth == 0:
            return i
        i += 1
    raise ValueError("no statement end")


def rewrite(path: pathlib.Path) -> int:
    src = path.read_text(encoding="utf-8")
    out = []
    i = 0
    count = 0
    while True:
        j = src.find(NEEDLE, i)
        if j < 0:
            out.append(src[i:])
            break
        expr_start = j + len(NEEDLE)
        semi = end_of_statement(src, expr_start)
        expr = src[expr_start:semi].strip()
        out.append(src[i:j])
        out.append(f"self.set_status({expr});")
        count += 1
        i = semi + 1
    if count:
        path.write_text("".join(out), encoding="utf-8")
    return count


def main() -> int:
    total = 0
    for p in sorted(pathlib.Path("crates/app/src").rglob("*.rs")):
        n = rewrite(p)
        if n:
            print(f"{p}: {n}")
        total += n
    print(f"total: {total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
