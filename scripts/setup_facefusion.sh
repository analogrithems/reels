#!/usr/bin/env bash
# Clone FaceFusion next to the repo (gitignored) and print install instructions.
# Full setup: https://docs.facefusion.io/facefusion-next/installation
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${ROOT}/third_party/facefusion"
mkdir -p "${ROOT}/third_party"
if [[ ! -d "${DEST}/.git" ]]; then
  echo "Cloning FaceFusion into ${DEST} …"
  git clone --depth 1 https://github.com/facefusion/facefusion "${DEST}"
else
  echo "Already present: ${DEST}"
fi
echo ""
echo "Next (from FaceFusion docs):"
echo "  cd ${DEST}"
echo "  python install.py --onnxruntime default   # or cuda / directml / openvino"
echo ""
echo "Then point Reel at the checkout:"
echo "  export FACE_FUSION_ROOT=${DEST}"
echo "  make run"
echo ""
echo "The sidecar will try to \`import facefusion\` from that tree; frame-level"
echo "inference is still stubbed until we wire their pipeline into the bridge."
