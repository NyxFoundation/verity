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

if [ ! -x "${CHARON}" ]; then
  echo "charon not found at ${CHARON}" >&2
  echo "install with: cargo hax tools install" >&2
  exit 1
fi
if [ ! -x "${AENEAS}" ]; then
  echo "aeneas not found at ${AENEAS}" >&2
  echo "install with: cargo hax tools install" >&2
  exit 1
fi

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

LEAN_ROOT="${ROOT}/proofs/lean"
LEAN_MODULES="${LEAN_ROOT}/VerityChain"
mkdir -p "${LEAN_MODULES}"

for generated in Funs.lean FunsExternal_Template.lean Types.lean TypesExternal_Template.lean; do
  if [ -f "${LEAN_ROOT}/${generated}" ]; then
    mv "${LEAN_ROOT}/${generated}" "${LEAN_MODULES}/${generated}"
  fi
done

for external in FunsExternal.lean TypesExternal.lean; do
  if [ ! -f "${LEAN_MODULES}/${external}" ]; then
    echo "required hand-written model missing: ${LEAN_MODULES}/${external}" >&2
    exit 1
  fi
done

cat > "${LEAN_ROOT}/VerityChain.lean" <<'EOF'
import VerityChain.Funs
import VerityChain.Correspondence
EOF
