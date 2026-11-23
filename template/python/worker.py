#!/usr/bin/env python3
"""Python worker example: a long-lived child process the app talks to over stdio.

Same newline-delimited JSON as the Rust worker: one request per line,
{"id": 1, "method": "stats", "params": {"text": "..."}}, answered by
{"id": 1, "result": ...} or {"id": 1, "error": "..."}. Standard library only, so any
Python 3.9+ runs it; add your own methods to METHODS (and dependencies via uv).
"""

import collections
import json
import platform
import re
import sys

STOP = set("a an and are as at be by for from has in is it of on or that the to was were will with".split())


def ping(_params):
    return {"python": platform.python_version(), "executable": sys.executable}


def stats(params):
    text = str(params.get("text", ""))
    words = re.findall(r"[A-Za-z][A-Za-z'-]+", text)
    sentences = [s for s in re.split(r"[.!?]+", text) if s.strip()]
    common = collections.Counter(w.lower() for w in words if w.lower() not in STOP).most_common(5)
    return {"words": len(words), "sentences": len(sentences), "top": [w for w, _ in common]}


METHODS = {"ping": ping, "stats": stats}


def handle(line):
    try:
        req = json.loads(line)
    except json.JSONDecodeError as e:
        return {"id": None, "error": f"bad JSON: {e}"}
    rid = req.get("id")
    fn = METHODS.get(req.get("method"))
    if fn is None:
        return {"id": rid, "error": f"unknown method {req.get('method')!r}"}
    try:
        return {"id": rid, "result": fn(req.get("params") or {})}
    except Exception as e:  # report, keep serving
        return {"id": rid, "error": f"{type(e).__name__}: {e}"}


def main():
    for line in sys.stdin:
        if line.strip():
            sys.stdout.write(json.dumps(handle(line)) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
