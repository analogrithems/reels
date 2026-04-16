"""Phase 0 stub for the FaceFusion bridge.

Reads one JSON request from stdin, writes one JSON response to stdout, logs
progress to stderr. Real FaceFusion wiring arrives in Phase 3.
"""

from __future__ import annotations

import json
import sys


def main() -> int:
    print("facefusion_bridge: stub ready", file=sys.stderr, flush=True)
    raw = sys.stdin.readline()
    if not raw:
        return 0
    try:
        req = json.loads(raw)
    except json.JSONDecodeError as e:
        json.dump({"status": "err", "reason": f"invalid json: {e}"}, sys.stdout)
        sys.stdout.write("\n")
        sys.stdout.flush()
        return 1
    print(f"facefusion_bridge: op={req.get('op', 'unknown')}", file=sys.stderr, flush=True)
    json.dump({"status": "stub", "echo": req}, sys.stdout)
    sys.stdout.write("\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
