#!/usr/bin/env python3
"""Test policy only: remember successful reads and reject edits without one."""
import argparse
import json
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", required=True)
    parser.add_argument("--audit", required=True)
    args = parser.parse_args()
    event = json.load(sys.stdin)
    cwd = Path(event["cwd"])
    state = cwd / args.state
    audit = cwd / args.audit
    path = (cwd / event["tool_input"]["path"]).resolve()
    read_files = set(json.loads(state.read_text()) if state.exists() else [])
    result = "ignored"
    blocked = False
    if event["hook_event_name"] == "PostToolUse" and event["tool_name"] == "read_file":
        # hel calls Post only on success. The fixture uses ordinary path-based reads.
        read_files.add(str(path))
        state.parent.mkdir(parents=True, exist_ok=True)
        pending = state.with_suffix(".tmp")
        pending.write_text(json.dumps(sorted(read_files)) + "\n")
        pending.replace(state)
        result = "read-recorded"
    elif event["hook_event_name"] == "PreToolUse":
        blocked = str(path) not in read_files
        result = "blocked" if blocked else "allowed"
    audit.parent.mkdir(parents=True, exist_ok=True)
    with audit.open("a") as out:
        out.write(json.dumps({
            "tool_use_id": event["tool_use_id"],
            "event": event["hook_event_name"],
            "tool": event["tool_name"],
            "path": str(path),
            "result": result,
        }) + "\n")
    if blocked:
        print("config.ini를 먼저 read_file로 읽은 뒤 편집을 다시 요청해 주세요.", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
