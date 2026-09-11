import Ssz.Codec.Deserialize
import Ssz.Codec.Root

/-! Verity's narrow C-export adapter over the vendored implementation. -/

namespace VeritySsz

open Ssz

private def container (fields : List Desc) : Desc :=
  .container (List.replicate fields.length "_") fields

private def uint64 : Desc := .uint 8
private def bytes32 : Desc := .byteVector 32
private def bytes52 : Desc := .byteVector 52
private def checkpoint : Desc := container [bytes32, uint64]
private def attestationData : Desc :=
  container [uint64, checkpoint, checkpoint, checkpoint]
private def aggregationBits : Desc := .bitList (1 <<< 12)
private def aggregatedAttestation : Desc := container [aggregationBits, attestationData]
private def blockBody : Desc := container [.list aggregatedAttestation (1 <<< 12)]
private def blockHeader : Desc :=
  container [uint64, uint64, bytes32, bytes32, bytes32]
private def block : Desc :=
  container [uint64, uint64, bytes32, bytes32, blockBody]
private def validator : Desc := container [bytes52, bytes52, uint64]
private def genesisConfig : Desc := container [uint64]
private def state : Desc := container [
  genesisConfig,
  uint64,
  blockHeader,
  checkpoint,
  checkpoint,
  .list bytes32 (1 <<< 18),
  .bitList (1 <<< 18),
  .list validator (1 <<< 12),
  .list bytes32 (1 <<< 18),
  .bitList ((1 <<< 18) * (1 <<< 12))
]

private def shape : UInt8 → Option Desc
  | 1 => some state
  | 2 => some block
  | 3 => some blockBody
  | 4 => some blockHeader
  | 5 => some attestationData
  | _ => none

/--
Computes a root from canonical SSZ bytes.

An empty result is the C ABI's error sentinel. A valid root is always exactly 32 bytes.
-/
@[export verity_ssz_hash_tree_root_lean]
def hashTreeRootExport (tag : UInt8) (encoded : ByteArray) : ByteArray :=
  match shape tag with
  | none => ByteArray.empty
  | some desc =>
    match Ssz.deserialize desc encoded.data with
    | .error _ => ByteArray.empty
    | .ok value =>
      match Ssz.hashTreeRoot desc value with
      | .error _ => ByteArray.empty
      | .ok root => ⟨root⟩

end VeritySsz
