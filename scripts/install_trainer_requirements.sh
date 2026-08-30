#!/usr/bin/env bash
set -euo pipefail

trainer_checkout="${1:-../haitaka-variant-nnue-pytorch}"

cd "$trainer_checkout"

if [ ! -d env ]; then
  python3 -m venv env
fi

source env/bin/activate
pip install --upgrade pip
# PyTorch Lightning 1.9 imports pkg_resources at startup. Setuptools 81 removes
# that compatibility module, so keep the isolated trainer environment on the
# newest compatible setuptools release.
pip install --default-timeout=1000 --retries=10 --no-cache-dir 'setuptools<81'

requirements="${HAITAKA_TRAINER_REQUIREMENTS:-}"
if [ -z "$requirements" ]; then
  requirements="requirements.txt"
  cuda_version="$(nvidia-smi 2>/dev/null | sed -n 's/.*CUDA Version: \([0-9.]*\).*/\1/p' | head -n 1 || true)"
  case "$cuda_version" in
    12.8*|12.9*|13.*)
      if [ -f requirements-CUDA128.txt ]; then
        requirements="requirements-CUDA128.txt"
      fi
      ;;
  esac
fi

if [ ! -f "$requirements" ]; then
  echo "requirements file not found: $trainer_checkout/$requirements" >&2
  exit 1
fi

echo "installing trainer requirements from $trainer_checkout/$requirements"
pip install --default-timeout=1000 --retries=10 --no-cache-dir -r "$requirements"
