#!/bin/sh
# Drive Charon then Aeneas on the extractable modules of verity-chain.
# Verification branch only.
#
# Binaries default to the versions cargo-hax 0.4.0 pins:
#   charon  nightly-2026.09.02
#   aeneas  nightly-2026.09.03-6852e64
set -eu

CHARON="${CHARON:-${HOME}/.cache/hax/tools/charon/nightly-2026.09.02/charon}"
AENEAS="${AENEAS:-${HOME}/.cache/hax/tools/aeneas/nightly-2026.09.03-6852e64/aeneas}"
ROOT="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
LLBC="${ROOT}/proofs/llbc/verity_chain.llbc"

mkdir -p "${ROOT}/proofs/llbc" "${ROOT}/proofs/lean"
PATH="$(dirname "${CHARON}"):${PATH}"
export PATH

"${CHARON}" cargo --preset=aeneas \
  --dest-file "${LLBC}" \
  --start-from verity_chain::justification \
  --start-from verity_chain::proposer \
  --start-from verity_chain::slot_clock \
  --start-from verity_chain::merkle

"${AENEAS}" -backend lean -dest "${ROOT}/proofs/lean" -split-files -gen-lib-entry \
  "${LLBC}"

if [ ! -f "${ROOT}/proofs/lean/FunsExternal.lean" ] &&
   [ -f "${ROOT}/proofs/lean/FunsExternal_Template.lean" ]; then
  cp "${ROOT}/proofs/lean/FunsExternal_Template.lean" \
    "${ROOT}/proofs/lean/FunsExternal.lean"
fi
if [ ! -f "${ROOT}/proofs/lean/TypesExternal.lean" ] &&
   [ -f "${ROOT}/proofs/lean/TypesExternal_Template.lean" ]; then
  cp "${ROOT}/proofs/lean/TypesExternal_Template.lean" \
    "${ROOT}/proofs/lean/TypesExternal.lean"
fi
