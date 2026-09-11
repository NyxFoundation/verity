/-! Tree nodes, and the fold that joins two of them. -/

namespace Ssz

/--
Width of a tree node in bytes.

The specification fixes it at the width of the hash output.
-/
abbrev bytesPerChunk : Nat :=
  -- The SHA-256 digest width fixes the byte boundary used by every parent hash.
  32

/-- Width of a tree node in bits. -/
abbrev bitsPerChunk : Nat :=
  -- Eight bits per byte give 256 bit positions in one node.
  8 * bytesPerChunk

/-- One node of a Merkle tree, with its width carried in the type. -/
abbrev Chunk :=
  -- The finite vector length makes an incorrectly sized node unrepresentable here.
  Vector UInt8 bytesPerChunk

/-- The node an absent leaf contributes. -/
def Chunk.zero : Chunk :=
  -- Every missing leaf starts as thirty-two zero bytes.
  Vector.replicate bytesPerChunk 0

/--
Folding two child nodes into their parent.

No proof below inspects it, so none assumes anything about the hash.
Changing the hash therefore costs nothing already proven.
-/
abbrev Combine :=
  -- Both operands and the result retain their complete-node widths.
  Chunk → Chunk → Chunk

end Ssz
