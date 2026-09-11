#!/bin/sh
# Drive Charon then Aeneas on this crate. Verification branch only.
#
# Binaries default to the versions cargo-hax 0.4.0 pins:
#   charon  nightly-2026.09.02
#   aeneas  nightly-2026.09.03-6852e64
# Override with CHARON=... AENEAS=... if you built your own.
set -eu

CHARON="${CHARON:-${HOME}/.cache/hax/tools/charon/nightly-2026.09.02/charon}"
AENEAS="${AENEAS:-${HOME}/.cache/hax/tools/aeneas/nightly-2026.09.03-6852e64/aeneas}"
ROOT="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
LLBC="${ROOT}/proofs/llbc/verity_justifiability.llbc"

mkdir -p "${ROOT}/proofs/llbc" "${ROOT}/proofs/lean"
PATH="$(dirname "${CHARON}"):${PATH}"
export PATH

"${CHARON}" cargo --preset=aeneas \
  --dest-file "${LLBC}" \
  --start-from verity_justifiability::is_justifiable_after

"${AENEAS}" -backend lean -dest "${ROOT}/proofs/lean" -split-files -gen-lib-entry \
  "${LLBC}"

# Aeneas regenerates the template every run and never touches FunsExternal.lean.
if [ ! -f "${ROOT}/proofs/lean/FunsExternal.lean" ]; then
  cp "${ROOT}/proofs/lean/FunsExternal_Template.lean" \
    "${ROOT}/proofs/lean/FunsExternal.lean"
fi
