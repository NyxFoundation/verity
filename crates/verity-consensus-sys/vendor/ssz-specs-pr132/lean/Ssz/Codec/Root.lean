import Ssz.Codec.Layout

/-! The Merkle root of a value, taken from the tree its layout describes. -/

namespace Ssz

mutual

/--
The SSZ Merkle root of a value, read against the type it is meant to fit.

The layout supplies the leaves, capacity, and optional mixing word.
Rooting evaluates that layout without redefining any type-specific tree rule.

The budget bounds how far the rooting may descend into nested values.
The public operation supplies the declaration’s full nesting depth.
-/
def hashTreeRootAt : Nat → Desc → Value → Except Err Bytes
  -- Public calls cover every nesting level, so only an insufficient explicit budget reaches this case.
  | 0, _, _ => .error .proofIncomplete
  | budget + 1, shape, value => do
    -- What this value puts under its tree, and which of the two shapes that tree takes.
    let layout ← merkleLayout shape value
    -- The leaves, with every nested value rooted in its own right.
    let chunks ← layoutChunksAt budget layout
    -- A capacity names a bounded tree, and its absence a progressive spine.
    let root ← match layout.limit with
      | none => .ok (merkleizeProgressive chunks.toList)
      | some capacity => merkleizeBounded chunks (some capacity)
    -- A shape that mixes a word in puts its contents on the left and the word on the right.
    return match layout.mixin with
      | none => root
      | some word => mixIn root word

/--
The leaves in a half-open range, as nodes, each nested value rooted here.

A range is asked for where a proof walks into one part of a tree.
Rooting a leaf only when it is reached keeps a proof from hashing the whole value.
-/
def layoutChunksAt (budget : Nat) (layout : MerkleLayout)
    (start : Nat := 0) (stop : Option Nat := none) : Except Err (Array Bytes) := do
  -- An absent end means every leaf the shape produced.
  let last := stop.getD layout.leaves.count
  match layout.leaves with
  -- Packed data is already nodes, so the range is taken from them directly.
  | .packed chunks => return chunks.extract start last
  | .nested values =>
    -- Slice first so a proof only roots values inside the requested interval.
    let slots := (values.drop start).take (last - start)
    let chunks ← slots.mapM fun slot =>
      match slot with
      -- A position carrying no value merkleizes as a zero leaf.
      | none => pure zeroChunk
      -- Present values contribute their own roots in declaration order.
      | some (shape, value) => hashTreeRootAt budget shape value
    return chunks.toArray

end

/--
The SSZ Merkle root of a value, read against the type it is meant to fit.

The budget is the type's own nesting, which is past what the rooting can spend.
-/
def hashTreeRoot (shape : Desc) (value : Value) : Except Err Bytes :=
  -- Each step of the rooting descends into one nested type.
  hashTreeRootAt shape.nesting shape value

end Ssz
