"""FaceFusion bridge — line-delimited JSON RPC over stdio.

See `crates/reel-core/src/sidecar.rs` for the protocol contract.

Supported ops:
- `ping`      → `{status: "ok"}`
- `shutdown`  → exits the loop with status 0
- `swap`      → reads `in_path` as raw RGBA8 (width*height*4), applies the
                transform from `params.model` (default "identity"), writes
                to `<in_path>.out`, returns `{status: "ok", out_path: …}`.

Transforms (`params.model`):
- `identity` / `face_enhance` — copy in → out (enhance is stub until GFPGAN-style path lands)
- `invert` — flip R/G/B, leave A alone (visible-change sanity test)
- `facefusion` — optional import from `FACE_FUSION_ROOT` checkout; otherwise identity + log
- `rvm_chroma` — **stub** chroma-style matte (green spill removal + alpha), not the full
  Robust Video Matting network — used for TDD and offline preview until ONNX RVM is wired

Test hooks on `params`:
- `sleep_ms: int` — delay the swap response, for client-timeout tests
- `crash: bool`   — `sys.exit(1)` before responding, for client-crash tests
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path
from typing import Any


def log(msg: str) -> None:
    print(f"facefusion_bridge: {msg}", file=sys.stderr, flush=True)


def _identity(data: bytes, _w: int, _h: int) -> bytes:
    return data


def _invert(data: bytes, _w: int, _h: int) -> bytes:
    out = bytearray(data)
    # RGBA: flip R, G, B; keep A.
    for i in range(0, len(out), 4):
        out[i] = 255 - out[i]
        out[i + 1] = 255 - out[i + 1]
        out[i + 2] = 255 - out[i + 2]
    return bytes(out)


def _facefusion(data: bytes, _w: int, _h: int) -> bytes:
    """Placeholder for FaceFusion inference; verifies checkout when possible."""
    root = os.environ.get("FACE_FUSION_ROOT", "").strip()
    if not root:
        log(
            "facefusion: set FACE_FUSION_ROOT to your clone, run install.py, "
            "then wire the pipeline — returning input unchanged"
        )
        return data
    p = Path(root)
    if not p.is_dir():
        log(f"facefusion: FACE_FUSION_ROOT not a directory: {root}")
        return data
    ff = p / "facefusion.py"
    if not ff.is_file():
        log(f"facefusion: expected {ff} — run scripts/setup_facefusion.sh")
        return data
    try:
        sys.path.insert(0, str(p))
        import facefusion  # noqa: F401, I001

        log(
            "facefusion: import ok; frame pipeline not yet invoked from bridge — "
            "identity passthrough"
        )
    except ImportError as e:
        log(f"facefusion: import failed ({e}); run `python install.py` in checkout")
    return data


def _face_enhance(data: bytes, _w: int, _h: int) -> bytes:
    log("face_enhance: stub (GFPGAN/CodeFormer-style path TBD)")
    return data


def _rvm_chroma(data: bytes, _w: int, _h: int) -> bytes:
    """Chroma-style green spill removal + alpha (RVM full model is future work)."""
    out = bytearray(data)
    for i in range(0, len(out), 4):
        r, g, b = out[i], out[i + 1], out[i + 2]
        # Simple green-screen key: strong G vs R/B.
        if g > 90 and g > r + 40 and g > b + 40:
            out[i + 3] = 0
        else:
            out[i + 3] = 255
    return bytes(out)


TRANSFORMS = {
    "identity": _identity,
    "invert": _invert,
    "facefusion": _facefusion,
    "face_enhance": _face_enhance,
    "rvm_chroma": _rvm_chroma,
}


def handle_swap(req: dict[str, Any]) -> dict[str, Any]:
    params = req.get("params") or {}
    if params.get("crash"):
        log("crash requested; exiting")
        sys.exit(1)
    sleep_ms = params.get("sleep_ms")
    if isinstance(sleep_ms, (int, float)) and sleep_ms > 0:
        time.sleep(sleep_ms / 1000.0)

    in_path = req.get("in_path")
    if not isinstance(in_path, str):
        return {"status": "err", "reason": "missing in_path"}
    try:
        width = int(req["width"])
        height = int(req["height"])
    except (KeyError, TypeError, ValueError):
        return {"status": "err", "reason": "missing/invalid width/height"}

    model = params.get("model", "identity")
    fn = TRANSFORMS.get(model)
    if fn is None:
        return {"status": "err", "reason": f"unknown model: {model}"}

    try:
        with open(in_path, "rb") as f:
            data = f.read()
    except OSError as e:
        return {"status": "err", "reason": f"read {in_path}: {e}"}

    expected = width * height * 4
    if len(data) != expected:
        return {
            "status": "err",
            "reason": f"rgba length {len(data)} != {expected}",
        }

    out = fn(data, width, height)
    out_path = in_path + ".out"
    try:
        with open(out_path, "wb") as f:
            f.write(out)
    except OSError as e:
        return {"status": "err", "reason": f"write {out_path}: {e}"}

    return {"status": "ok", "out_path": out_path}


def dispatch(req: dict[str, Any]) -> dict[str, Any] | None:
    """Return the response dict, or None if the op is `shutdown`."""
    op = req.get("op")
    req_id = req.get("id", 0)
    if op == "shutdown":
        return None
    if op == "ping":
        return {"id": req_id, "status": "ok"}
    if op == "swap":
        body = handle_swap(req)
        body["id"] = req_id
        return body
    return {"id": req_id, "status": "err", "reason": f"unknown op: {op}"}


def main() -> int:
    log("ready")
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            req = json.loads(raw)
        except json.JSONDecodeError as e:
            log(f"invalid json: {e}")
            continue
        if not isinstance(req, dict):
            log("request is not a JSON object; ignoring")
            continue
        resp = dispatch(req)
        if resp is None:
            log("shutdown requested")
            return 0
        print(json.dumps(resp), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
