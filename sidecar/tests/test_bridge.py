"""Unit tests for `facefusion_bridge.py`.

Exercises the two pure functions (`_identity`, `_invert`) plus one end-to-end
stdio round-trip to prove the protocol parser still works if the shape of
`dispatch()` changes.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

BRIDGE = Path(__file__).resolve().parents[1] / "facefusion_bridge.py"


def _run_bridge(requests: list[dict], *, timeout: float = 5.0) -> list[dict]:
    proc = subprocess.Popen(
        [sys.executable, str(BRIDGE)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    payload = ("\n".join(json.dumps(r) for r in requests) + "\n").encode()
    out, _ = proc.communicate(input=payload, timeout=timeout)
    return [json.loads(line) for line in out.decode().splitlines() if line.strip()]


def test_identity_preserves_bytes():
    # Import after defining BRIDGE so the source file is on the path.
    sys.path.insert(0, str(BRIDGE.parent))
    import facefusion_bridge as bridge  # noqa: I001

    data = bytes(range(256)) * 2  # 512 bytes = 128 RGBA px
    assert bridge._identity(data, 16, 8) == data


def test_invert_flips_rgb_keeps_alpha():
    sys.path.insert(0, str(BRIDGE.parent))
    import facefusion_bridge as bridge  # noqa: I001

    # One pixel: R=10, G=20, B=30, A=200.
    out = bridge._invert(bytes([10, 20, 30, 200]), 1, 1)
    assert out == bytes([245, 235, 225, 200])


def test_ping_shutdown_round_trip():
    out = _run_bridge([{"id": 1, "op": "ping"}, {"id": 2, "op": "shutdown"}])
    assert out == [{"id": 1, "status": "ok"}]


def test_swap_identity_end_to_end():
    with tempfile.TemporaryDirectory() as td:
        in_path = Path(td) / "in.rgba"
        data = bytes([1, 2, 3, 255] * 4)  # 2x2 px
        in_path.write_bytes(data)
        out = _run_bridge([
            {
                "id": 7,
                "op": "swap",
                "in_path": str(in_path),
                "width": 2,
                "height": 2,
                "params": {"model": "identity"},
            },
            {"id": 8, "op": "shutdown"},
        ])
        assert len(out) == 1
        assert out[0]["id"] == 7
        assert out[0]["status"] == "ok"
        out_bytes = Path(out[0]["out_path"]).read_bytes()
        assert out_bytes == data


def test_unknown_op_returns_err():
    out = _run_bridge([
        {"id": 1, "op": "bogus"},
        {"id": 2, "op": "shutdown"},
    ])
    assert out == [{"id": 1, "status": "err", "reason": "unknown op: bogus"}]
