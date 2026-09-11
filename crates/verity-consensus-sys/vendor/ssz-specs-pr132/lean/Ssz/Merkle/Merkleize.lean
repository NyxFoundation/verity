import Ssz.Merkle.Chunk

/-! The root of an SSZ Merkle tree, and the facts an implementation of it leans on. -/

namespace Ssz

variable {α : Type}

/--
Root of the perfect binary tree of a given depth.

Leaves arrive as a function from position to node, not as a list.
A tree wider than its data therefore needs no padding to describe.

    depth 2:

    parent hash
     +-- left parent hash
     |    +-- leaf 0
     |    `-- leaf 1
     `-- right parent hash
          +-- leaf 2
          `-- leaf 3

Nothing here reads a node, so the account holds for any node type at all.
-/
def subtreeRoot (combine : α → α → α) : (depth : Nat) → (leaves : Nat → α) → α
  -- A tree of depth zero is a single leaf, which is already its own root.
  | 0, leaves => leaves 0
  -- Deeper trees fold the two half-width subtrees below them.
  --
  -- The right half sees the same leaves shifted past the left half's width.
  | depth + 1, leaves =>
      combine
        (subtreeRoot combine depth leaves)
        (subtreeRoot combine depth fun i => leaves (i + 2 ^ depth))

/--
Root of the perfect binary tree of a given depth whose every leaf is zero.

Implementations precompute it.
A value with a large capacity is mostly padding.
-/
def zeroRoot (combine : α → α → α) (zero : α) : Nat → α
  -- The bottom of an empty subtree is the zero node itself.
  | 0 => zero
  -- One level up, both children are the empty subtree of the level below.
  | depth + 1 => combine (zeroRoot combine zero depth) (zeroRoot combine zero depth)

/-- Smallest depth whose perfect tree holds the given number of leaves. -/
def depthFor (leafCount : Nat) : Nat :=
  -- One position covers an empty input and a lone leaf alike.
  if leafCount ≤ 1 then 0
  -- Otherwise pair the leaves up and ask again one level higher.
  --
  -- Rounding up is what stops an odd count from losing its last leaf.
  else depthFor ((leafCount + 1) / 2) + 1
-- Two or more leaves halve to strictly fewer, so the recursion runs out.
decreasing_by omega

/--
Leaves of a tree, taken from an array of nodes.

Positions past the end of the array read as zero.
-/
def padded (zero : α) (chunks : Array α) : Nat → α :=
  -- Past the end is absent rather than out of range, so the tree may outsize its data.
  fun position => chunks[position]?.getD zero

/-- A spine of perfect trees with capacities 1, 4, 16, and so on, per [EIP-7916](https://eips.ethereum.org/EIPS/eip-7916). -/
def progressiveRoot (combine : α → α → α) (zero : α) (chunks : List α)
    (level : Nat := 0) : α :=
  -- EIP-7916 closes an empty suffix with a single zero node.
  if chunks.isEmpty then zero
  else
    let width := 4 ^ level
    -- The left subtree holds this level's leaves, padded to its fixed capacity.
    let left := subtreeRoot combine (depthFor width) (padded zero (chunks.take width).toArray)
    -- Remaining leaves continue on the right at four times the capacity.
    combine left (progressiveRoot combine zero (chunks.drop width) (level + 1))
termination_by chunks.length
decreasing_by
  have positive : 0 < 4 ^ level := Nat.pos_of_neZero _
  have nonempty : 0 < chunks.length := by cases chunks <;> simp_all
  simp only [List.length_drop]
  omega

/--
The chosen depth really does hold the requested number of leaves.

Without this the capacity check names a number unrelated to the tree built.
-/
theorem le_two_pow_depthFor (leafCount : Nat) : leafCount ≤ 2 ^ depthFor leafCount := by
  -- Recursing the proof the way the definition recurses keeps the two in step.
  induction leafCount using depthFor.induct with
  | case1 leafCount small =>
    -- Zero and one both fit the single position of a depth-zero tree.
    rw [depthFor]
    simp [small]
  | case2 leafCount large ih =>
    -- Widths double one level up, and the halved count already fits the level below.
    rw [depthFor]
    -- Unfolding the doubling turns the hypothesis into plain linear arithmetic.
    simp only [if_neg large, Nat.pow_succ]
    omega

/--
No shallower tree holds them, so the depth chosen is the least one that does.

Together with the bound above this pins the depth exactly.
That is what ties it to the specification's next power of two.
-/
theorem depthFor_le_of_le_two_pow {leafCount depth : Nat} (holds : leafCount ≤ 2 ^ depth) :
    depthFor leafCount ≤ depth := by
  induction leafCount using depthFor.induct generalizing depth with
  | case1 leafCount small =>
    -- A depth-zero tree is already the shallowest there is.
    rw [depthFor]
    simp [small]
  | case2 leafCount large ih =>
    -- Two or more leaves need a level, so the bound must have come from one too.
    rw [depthFor, if_neg large]
    match depth with
    | 0 => simp at holds; omega
    | shallower + 1 =>
      have : (leafCount + 1) / 2 ≤ 2 ^ shallower := by
        rw [Nat.pow_succ] at holds
        omega
      have := ih this
      omega

/-- A leaf count that is already a power of two needs exactly the depth it names. -/
theorem depthFor_pow (depth : Nat) : depthFor (2 ^ depth) = depth := by
  -- No shallower tree holds that many leaves, and no deeper one is chosen.
  have below : depthFor (2 ^ depth) ≤ depth := depthFor_le_of_le_two_pow (Nat.le_refl _)
  have above : 2 ^ depth ≤ 2 ^ depthFor (2 ^ depth) := le_two_pow_depthFor (2 ^ depth)
  have widened : depth ≤ depthFor (2 ^ depth) :=
    (Nat.pow_le_pow_iff_right (by omega)).mp above
  omega

/--
Two leaf supplies that agree everywhere the tree reads give the same root.

The tree reads positions below its width and nothing else.
Every claim that a rearrangement leaves the root alone reduces to this one.
-/
theorem subtreeRoot_congr (combine : α → α → α) (depth : Nat) {f g : Nat → α}
    (agree : ∀ position, position < 2 ^ depth → f position = g position) :
    subtreeRoot combine depth f = subtreeRoot combine depth g := by
  -- The leaf supplies change on the way down, so they stay quantified over.
  induction depth generalizing f g with
  | zero =>
    -- A depth-zero tree reads position 0 alone, which the hypothesis covers.
    exact agree 0 (by decide)
  | succ depth ih =>
    -- Left half: positions below half the width, covered by the hypothesis.
    have left : subtreeRoot combine depth f = subtreeRoot combine depth g :=
      ih fun position bound => agree position (by omega)
    -- Right half: the same positions shifted up, still inside the full width.
    --
    --     full width = 2 ^ (depth + 1) = 2 ^ depth + 2 ^ depth
    --     shifted    = position + 2 ^ depth, with position < 2 ^ depth
    have right :
        subtreeRoot combine depth (fun i => f (i + 2 ^ depth))
          = subtreeRoot combine depth (fun i => g (i + 2 ^ depth)) :=
      ih fun position bound =>
        agree (position + 2 ^ depth) (by rw [Nat.pow_succ]; omega)
    simp [subtreeRoot, left, right]

/--
A tree of zero leaves folds to the table entry for its depth.

An empty subtree can then be answered by lookup rather than by hashing it out.
-/
theorem subtreeRoot_const_zero (combine : α → α → α) (zero : α) (depth : Nat) :
    subtreeRoot combine depth (fun _ => zero) = zeroRoot combine zero depth := by
  -- The leaf supply is the same constant at every level, so plain induction suffices.
  induction depth with
  | zero => rfl
  | succ depth ih => simp [subtreeRoot, zeroRoot, ih]

/--
Appending zero nodes to the data changes no leaf.

An implementation may fill its buffer out to the tree width, or leave it short.
Nothing above the leaves can tell the two apart.
-/
theorem padded_append_zeros (zero : α) (chunks : Array α) (count : Nat) :
    padded zero (chunks ++ Array.replicate count zero) = padded zero chunks := by
  -- Leaf supplies are functions, so they are equal once they agree pointwise.
  funext position
  simp only [padded, Array.getElem?_append]
  -- Inside the data both sides read the same node.
  split
  · rfl
  -- Past the data the original array has nothing there.
  · rename_i past_data
    have absent : chunks[position]? = none := Array.getElem?_eq_none (by omega)
    -- What remains is the padding, which reads zero whether or not it was written.
    simp only [absent, Option.getD_none, Array.getElem?_replicate]
    split <;> rfl

/--
Materializing the padding does not change the root.

Cashed in whenever an implementation hashes a short buffer instead of a full one.
-/
theorem subtreeRoot_padded_append_zeros
    (combine : α → α → α) (zero : α) (depth : Nat) (chunks : Array α) (count : Nat) :
    subtreeRoot combine depth (padded zero (chunks ++ Array.replicate count zero))
      = subtreeRoot combine depth (padded zero chunks) := by
  -- The leaf supplies are equal outright, so the roots follow without induction.
  rw [padded_append_zeros]

/-- A tree whose data begins past the end of the array is the empty tree of its depth. -/
theorem subtreeRoot_padded_past_data (combine : α → α → α) (zero : α) (depth : Nat)
    (chunks : Array α) {start : Nat} (past : chunks.size ≤ start) :
    subtreeRoot combine depth (fun i => padded zero chunks (start + i))
      = zeroRoot combine zero depth := by
  -- Every position the tree reads lies past the data, so every leaf is zero.
  rw [← subtreeRoot_const_zero combine zero depth]
  refine subtreeRoot_congr combine depth fun position _ => ?_
  simp only [padded]
  rw [Array.getElem?_eq_none (by omega)]
  rfl

end Ssz
