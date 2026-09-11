import Ssz.Codec.Error

/-! Generalized indices: naming one node of a tree, and the nodes a proof of it needs. -/

namespace Ssz

/--
Levels an index sits below the root, which is none at all for the root itself.

The leading bit of an index carries its depth, so the depth is the bit width less one.
-/
def gindexDepth (index : Nat) : Except Err Nat :=
  -- Zero has no leading root bit, while every positive index has a well-defined binary depth.
  if index < 1 then .error (.notAGindex index) else .ok (Nat.log2 index)

/-- Depth of an index, which is also the number of nodes on its proof branch. -/
def gindexLength (index : Nat) : Except Err Nat := do
  let depth ← gindexDepth index
  -- The root sits on no branch, so it has no length to report.
  if depth == 0 then .error .rootHasNoBranch else .ok depth

/-- Whether the branch turns right at the given depth, counted from the leaf. -/
def gindexBit (index position : Nat) : Bool :=
  -- A low bit describes an upward turn before the higher bits are reached.
  index.testBit position

/-- The bottom turns of an index, as many as the depth, with everything above dropped. -/
def gindexBelow (index depth : Nat) : Nat :=
  -- Remainder modulo a power of two keeps exactly the requested low path bits.
  index % 2 ^ depth

/-- The node sharing a parent with this one. -/
def gindexSibling (index : Nat) : Nat :=
  -- Flipping the lowest bit changes the child side without changing its parent.
  index ^^^ 1

/-- One of the two nodes this one is the parent of. -/
def gindexChild (index : Nat) (rightSide : Bool) : Nat :=
  -- Appending one binary side bit selects the left or right child.
  index * 2 + (if rightSide then 1 else 0)

/-- The node this one is a child of. -/
def gindexParent (index : Nat) : Nat :=
  -- Integer division by two removes the final side bit.
  index / 2

/--
An index measured from one root, rebased onto a position in a larger tree.

An index carries its depth in its leading bit, so this splices rather than multiplies.

    outer 2, inner 24  ->  40, not 48
-/
def gindexConcat (outer inner : Nat) : Except Err Nat := do
  -- Both operands must name nodes, including the root onto which the path is spliced.
  let _ ← gindexDepth outer
  -- The inner depth determines how far the outer index shifts.
  let depth ← gindexDepth inner
  -- Dropping the inner leading bit is what stops the splice from being a multiplication.
  return outer * 2 ^ depth + (inner - 2 ^ depth)

/--
The bottom turns of an index, read as an index of a tree of their own.

Undoes a splice, handing the rest of an index to the subtree it lands in.

    splice 2 and 24  ->  40
    read 40 back at the depth of 24  ->  24
-/
def gindexRebase (index depth : Nat) : Nat :=
  -- Restore a leading root bit above the selected low path bits.
  2 ^ depth + gindexBelow index depth

/--
Position of one chunk on a progressive spine.

    level 1  holds chunk 0
    level 2  holds chunks 1 to 4
    level 3  holds chunks 5 to 20

A chunk keeps its place as the collection grows, so a proof of it outlives an append.
The index counts one level below the spine, the shape mixing a word into its root.
-/
def progressiveChunkGindex (chunk : Nat) (depth : Nat := 0) (spine : Nat := 2) : Nat :=
  let width := 2 ^ depth
  -- The subtree root is the spine node's left child, and the chunk sits below it.
  if chunk < width then spine * 2 * width + chunk
  -- Otherwise skip this level's chunks and widen the next one fourfold.
  else progressiveChunkGindex (chunk - width) (depth + 2) (spine * 2 + 1)
termination_by chunk
decreasing_by
  have : 0 < 2 ^ depth := Nat.pos_of_neZero (2 ^ depth)
  omega

/-- Nodes from the given one up to the root, excluding the root. -/
def getPathIndices (index : Nat) : Except Err (List Nat) := do
  -- One node per level below the root, which is what the depth counts.
  let length ← gindexLength index
  -- Shifting the index down by a level names the ancestor at that level.
  return (List.range length).map fun level => index >>> level

/-- Siblings along the path from the given node to the root, which is its proof branch. -/
def getBranchIndices (index : Nat) : Except Err (List Nat) := do
  -- A branch is what a verifier is not given, which is the sibling at every level.
  return (← getPathIndices index).map gindexSibling

/-- Refuse any claimed node that is a strict ancestor of the current claim. -/
def rejectAncestors (indices : List Nat) (claim : Nat) : List Nat → Except Err Unit
  | [] => .ok ()
  | ancestor :: rest => do
    -- An ancestor would replace a root already reconstructed from its descendants.
    if indices.contains ancestor then throw (.nestedIndex claim)
    rejectAncestors indices claim rest

/-- Validate every claim's strict ancestors in the original request order. -/
def rejectClaimPaths (indices : List Nat) : List Nat → Except Err Unit
  | [] => .ok ()
  | claim :: rest => do
    let path ← getPathIndices claim
    -- The first path node is the claim itself, not one of its strict ancestors.
    rejectAncestors indices claim (path.drop 1)
    rejectClaimPaths indices rest

/--
Refuse an index set that cannot be verified soundly.

An empty request claims nothing, so nothing about the root would be checked.
A repeated index keeps one value and drops the other without a word.
An ancestor claim can be overwritten when its descendants are folded upward.
-/
def rejectRelated (indices : List Nat) : Except Err Unit := do
  -- Empty, repeated, or ancestor-related claims would leave some requested statement unauthenticated.
  if indices.isEmpty then throw .emptyRequest
  if indices.eraseDups.length != indices.length then throw .repeatedIndex
  rejectClaimPaths indices indices

/-- The paths of several claims, flattened in the order the claims were supplied. -/
def collectPathIndices : List Nat → Except Err (List Nat)
  | [] => .ok []
  | index :: rest => do
    -- Each claim contributes its ancestors below the root.
    let path ← getPathIndices index
    let paths ← collectPathIndices rest
    return path ++ paths

/--
Nodes a proof must carry to authenticate all the given ones at once.

Every sibling on every branch, less the paths the verifier can rebuild.
Descending order fixes which proof value belongs to each helper index.
-/
def getHelperIndices (indices : List Nat) : Except Err (List Nat) := do
  rejectRelated indices
  -- Sharing the flattened paths also makes shared branches appear only once.
  let paths ← collectPathIndices indices
  let helpers := (paths.map gindexSibling).eraseDups.filter fun node => !paths.contains node
  return helpers.mergeSort fun a b => b ≤ a

/-- The two children of a node are the two numbers whose halving gives it back. -/
theorem gindexParent_child (index : Nat) (rightSide : Bool) :
    gindexParent (gindexChild index rightSide) = index := by
  -- A child is the parent doubled, with one added on the right, and halving drops that.
  cases rightSide <;> simp [gindexParent, gindexChild] <;> omega

/-- Siblings come in pairs, so naming one twice names the first again. -/
theorem gindexSibling_sibling (index : Nat) : gindexSibling (gindexSibling index) = index := by
  -- A sibling flips the lowest bit, and flipping it twice leaves it as it was.
  simp [gindexSibling, Nat.xor_assoc]

/-- An index sits at or above the power of two its depth names, and below the next. -/
theorem gindexDepth_bounds {index : Nat} (named : 1 ≤ index) :
    2 ^ Nat.log2 index ≤ index ∧ index < 2 ^ (Nat.log2 index + 1) :=
  -- The depth is where the leading bit sits, which is what brackets the index.
  ⟨Nat.log2_self_le (by omega), Nat.lt_log2_self⟩

/--
Splicing an index onto a position and reading it back gives the index unchanged.

An index carries its depth in its leading bit, so a splice is not a multiplication.
This round trip is what makes descending into a nested value's own tree sound.
-/
theorem gindexRebase_splice (outer index : Nat) (named : 1 ≤ index) :
    gindexRebase (outer * 2 ^ Nat.log2 index + (index - 2 ^ Nat.log2 index)) (Nat.log2 index)
      = index := by
  -- The index sits at or above its own leading bit, and below the next one up.
  obtain ⟨lower, upper⟩ := gindexDepth_bounds named
  -- So what the splice put below that bit is strictly narrower than the bit itself.
  have width : 2 ^ (Nat.log2 index + 1) = 2 ^ Nat.log2 index * 2 := by rw [Nat.pow_succ]
  -- Which means the outer position, shifted up past it, leaves no trace in the remainder.
  have below : (outer * 2 ^ Nat.log2 index + (index - 2 ^ Nat.log2 index)) % 2 ^ Nat.log2 index
      = index - 2 ^ Nat.log2 index := Nat.mul_add_mod_of_lt (by omega)
  simp only [gindexRebase, gindexBelow, below]
  -- Putting the leading bit back on what is left gives the index it was taken from.
  omega

/-- Siblings share a parent, which is the half both of them round down to. -/
theorem gindexSibling_half (index : Nat) : gindexSibling index / 2 = index / 2 := by
  -- Flipping the lowest bit leaves every bit above it alone, and those are the half.
  have shifted : (index ^^^ 1) >>> 1 = (index >>> 1) ^^^ (1 >>> 1) := Nat.shiftRight_xor_distrib
  simpa [gindexSibling, Nat.shiftRight_eq_div_pow] using shifted

/-- Siblings sit on opposite sides, so exactly one of the two is the odd one. -/
theorem gindexSibling_parity (index : Nat) : gindexSibling index % 2 = 1 - index % 2 := by
  -- The lowest bit is what a sibling flips, and the lowest bit is the parity.
  have flipped : (index ^^^ 1).testBit 0 = !index.testBit 0 := by
    rw [Nat.testBit_xor]
    simp
  simp only [Nat.testBit_zero] at flipped
  simp only [gindexSibling]
  rcases Nat.mod_two_eq_zero_or_one index with side | side <;>
    rcases Nat.mod_two_eq_zero_or_one (index ^^^ 1) with pair | pair <;>
      simp [side, pair] at flipped ⊢

/-- An even node and its sibling are the left and right children of the node above them. -/
theorem gindexSibling_even {index : Nat} (even : index % 2 = 0) :
    index = 2 * (index / 2) ∧ gindexSibling index = 2 * (index / 2) + 1 := by
  -- The two facts a sibling keeps and flips are its half and its parity.
  have half := gindexSibling_half index
  have parity := gindexSibling_parity index
  omega

/-- An odd node and its sibling are the right and left children of the node above them. -/
theorem gindexSibling_odd {index : Nat} (odd : index % 2 = 1) :
    index = 2 * (index / 2) + 1 ∧ gindexSibling index = 2 * (index / 2) := by
  -- The two facts a sibling keeps and flips are its half and its parity.
  have half := gindexSibling_half index
  have parity := gindexSibling_parity index
  omega

/-- Climbing one more level is halving what the level below reached. -/
theorem shiftRight_succ (index level : Nat) : index >>> (level + 1) = index >>> level / 2 := by
  -- Shifting by a sum is shifting twice, and the last shift by one is a halving.
  rw [Nat.shiftRight_add]
  simp [Nat.shiftRight_eq_div_pow]

/-- The turn taken at one level is the parity of what the walk has reached there. -/
theorem gindexBit_parity (index level : Nat) :
    gindexBit index level = decide (index >>> level % 2 = 1) := by
  -- Reading a bit of an index is reading the lowest bit of the index shifted to it.
  simp [gindexBit]

/-- Climbing every level an index carries arrives at the root. -/
theorem shiftRight_depth {index : Nat} (named : 1 ≤ index) : index >>> Nat.log2 index = 1 := by
  -- The index sits at or above its own leading bit, and below the next one up.
  obtain ⟨lower, upper⟩ := gindexDepth_bounds named
  have doubled : 2 ^ (Nat.log2 index + 1) = 2 ^ Nat.log2 index * 2 := by rw [Nat.pow_succ]
  -- One leading bit is therefore all that is left once everything below it is shifted away.
  rw [Nat.shiftRight_eq_div_pow]
  exact Nat.div_eq_of_lt_le (by omega) (by omega)

/-- The root sits on no branch, so a request naming it is refused. -/
theorem gindexLength_root : gindexLength 1 = .error .rootHasNoBranch :=
  -- The root needs no sibling, so it is not a valid single-branch request.
  rfl

/-- A number naming no node is refused before anything is measured of it. -/
theorem gindexDepth_zero : gindexDepth 0 = .error (.notAGindex 0) :=
  -- Zero cannot name a tree node because it carries no leading root bit.
  rfl

/-- A request naming no index is refused, since it would check nothing. -/
theorem rejectRelated_empty : rejectRelated [] = .error .emptyRequest :=
  -- With no claimed position, verification would establish nothing about the root.
  rfl

end Ssz
