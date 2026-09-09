import Ssz.Codec.Proof
import Ssz.Merkle.HelperFrontier
import Ssz.Type.Equality

/-! A bounded proof walk reads the same binary subtrees that merkleization hashes. -/

namespace Ssz

/-- A binary subtree selected by the remaining turns of a generalized index. -/
def boundedTreeNode (chunks : Array Bytes) (height index depth start : Nat) : Bytes :=
  -- Each remaining turn halves the leaf interval and the low path bits select its start.
  subtreeAt chunks (height - depth) (start + gindexBelow index depth * 2 ^ (height - depth))

/-- Walking within a bounded tree returns the root of the selected leaf interval. -/
theorem boundedNode_window {budget : Nat} {layout : MerkleLayout} {chunks : Array Bytes}
    (windows : ∀ start stop, layoutChunksAt budget layout start (some stop) =
      .ok (chunks.extract start stop))
    (index depth start capacity : Nat) (inside : depth ≤ depthFor capacity) :
    boundedNode budget layout index depth start capacity =
      .ok (boundedTreeNode chunks (depthFor capacity) index depth start) := by
  -- The remaining width is a power of two, so interval merkleization is exactly a subtree.
  have width : nextPow2 capacity >>> depth = 2 ^ (depthFor capacity - depth) := by
    rw [nextPow2, Nat.shiftRight_eq_div_pow, Nat.pow_div inside (by decide)]
  simp only [boundedNode, nextPow2, Nat.log2_two_pow, if_pos inside]
  rw [show 2 ^ depthFor capacity >>> depth = 2 ^ (depthFor capacity - depth) from width]
  simp only [windows, Bind.bind, Except.bind]
  exact merkleizeBounded_window chunks _

/-- Packed leaves already are hash nodes, so every bounded read is a direct interval read. -/
theorem boundedNode_packed (budget : Nat) (chunks : Array Bytes) (limit : Option Nat)
    (mixin : Option Bytes) (index depth start capacity : Nat)
    (inside : depth ≤ depthFor capacity) :
    boundedNode budget (.packing chunks limit mixin) index depth start capacity =
      .ok (boundedTreeNode chunks (depthFor capacity) index depth start) := by
  -- Packed layouts do not consume any nested-root budget.
  apply boundedNode_window (chunks := chunks) _ index depth start capacity inside
  intro first last
  simp [layoutChunksAt, MerkleLayout.packing, Pure.pure, Except.pure]

/-- Appending a left or right turn doubles the old position and adds that turn. -/
theorem gindexBelow_children (index depth : Nat) :
    gindexBelow (2 * index) (depth + 1) = 2 * gindexBelow index depth ∧
    gindexBelow (2 * index + 1) (depth + 1) = 2 * gindexBelow index depth + 1 := by
  -- Reducing modulo the enlarged width retains precisely the old turns and the new bit.
  -- A positive binary width bounds the retained remainder before a child turn is appended.
  have positive : 0 < 2 ^ depth := Nat.two_pow_pos depth
  have remainder : index % 2 ^ depth < 2 ^ depth := Nat.mod_lt _ positive
  have left : 2 * index % (2 * 2 ^ depth) = 2 * (index % 2 ^ depth) :=
    Nat.mul_mod_mul_left 2 index (2 ^ depth)
  constructor
  · simpa [gindexBelow, Nat.pow_succ, Nat.mul_comm] using left
  · simp only [gindexBelow, Nat.pow_succ]
    rw [Nat.mul_comm (2 ^ depth) 2, Nat.add_mod, left,
      Nat.mod_eq_of_lt (by omega : 1 < 2 * 2 ^ depth), Nat.mod_eq_of_lt (by omega)]

/-- An internal bounded node is the hash of its two child intervals. -/
theorem boundedTreeNode_split (chunks : Array Bytes) (height index depth start : Nat)
    (inside : depth < height) :
    boundedTreeNode chunks height index depth start =
      combine (boundedTreeNode chunks height (2 * index) (depth + 1) start)
        (boundedTreeNode chunks height (2 * index + 1) (depth + 1) start) := by
  -- One more turn halves the interval and places its right half immediately after the left.
  obtain ⟨left, right⟩ := gindexBelow_children index depth
  -- An internal node has one more remaining level than either child interval.
  have remaining : height - depth = height - (depth + 1) + 1 := by omega
  unfold boundedTreeNode
  rw [remaining, subtreeAt_split, left, right]
  simp [Nat.pow_succ, Nat.mul_add, Nat.add_assoc, Nat.mul_assoc, Nat.mul_left_comm, Nat.mul_comm]

/-- The actual bounded walker obeys the parent equation whenever it remains inside the tree. -/
theorem boundedNode_split {budget : Nat} {layout : MerkleLayout} {chunks : Array Bytes}
    (windows : ∀ start stop, layoutChunksAt budget layout start (some stop) =
      .ok (chunks.extract start stop))
    (index depth start capacity : Nat) (inside : depth < depthFor capacity) :
    boundedNode budget layout index depth start capacity = (do
      let left ← boundedNode budget layout (2 * index) (depth + 1) start capacity
      let right ← boundedNode budget layout (2 * index + 1) (depth + 1) start capacity
      return combine left right) := by
  -- All three executable reads are the intervals in the binary split theorem.
  rw [boundedNode_window windows index depth start capacity (by omega),
    boundedNode_window windows (2 * index) (depth + 1) start capacity (by omega),
    boundedNode_window windows (2 * index + 1) (depth + 1) start capacity (by omega)]
  simp only [Bind.bind, Except.bind, Pure.pure, Except.pure]
  exact congrArg Except.ok (boundedTreeNode_split chunks _ _ _ _ inside)

/-- The nodes of one bounded binary tree, indexed from its root. -/
def boundedReading (chunks : Array Bytes) (height index : Nat) : Bytes :=
  -- The generalized index’s bit depth supplies exactly the turns below the tree’s root bit.
  boundedTreeNode chunks height index (Nat.log2 index) 0

/-- The left and right children of an internal bounded node combine to that node. -/
theorem boundedReading_parent (chunks : Array Bytes) (height index : Nat)
    (named : 1 ≤ index) (internal : Nat.log2 index < height) :
    boundedReading chunks height index =
      combine (boundedReading chunks height (2 * index))
        (boundedReading chunks height (2 * index + 1)) := by
  -- Each child has one additional branch bit, and both halve back to their parent.
  have left : Nat.log2 (2 * index) = Nat.log2 index + 1 := Nat.log2_two_mul (by omega)
  have right : Nat.log2 (2 * index + 1) = Nat.log2 index + 1 := by
    have parent := levelOf_parent (index := 2 * index + 1) (by omega)
    have half : (2 * index + 1) / 2 = index := by omega
    simpa only [levelOf, half] using parent
  simp only [boundedReading, left, right]
  exact boundedTreeNode_split chunks height index (Nat.log2 index) 0 internal

/-- The root of a bounded packed layout is its padded binary tree. -/
theorem hashTreeRoot_packed {shape : Desc} {value : Value} {chunks : Array Bytes} {capacity : Nat}
    (layout : merkleLayout shape value = .ok (.packing chunks (some capacity) none))
    (fits : chunks.size ≤ capacity) :
    hashTreeRoot shape value = .ok (subtreeAt chunks (depthFor capacity) 0) := by
  -- Packed leaves need no recursive rooting, regardless of the type's nesting budget.
  have positive := shape.nesting_pos
  unfold hashTreeRoot
  cases nesting : shape.nesting with
  | zero => omega
  | succ budget =>
    simp [hashTreeRootAt, layout, layoutChunksAt, MerkleLayout.packing,
      merkleizeBounded, Leaves.count, Nat.not_lt.mpr fits, Bind.bind, Except.bind, Pure.pure, Except.pure]

/-- At index one, the bounded node reading is the entire padded tree. -/
theorem boundedReading_root (chunks : Array Bytes) (height : Nat) :
    boundedReading chunks height 1 = subtreeAt chunks height 0 := by
  -- The root consumes no turns and starts at the first leaf.
  have depth : Nat.log2 1 = 0 := rfl
  simp [boundedReading, boundedTreeNode, depth, gindexBelow]

/-- The actual value walker reads every node of a bounded packed tree. -/
theorem nodeRoot_packed {shape : Desc} {value : Value} {chunks : Array Bytes} {capacity : Nat}
    (layout : merkleLayout shape value = .ok (.packing chunks (some capacity) none))
    (fits : chunks.size ≤ capacity) (index : Nat) (named : 1 ≤ index)
    (inside : Nat.log2 index ≤ depthFor capacity) :
    nodeRoot shape value index = .ok (boundedReading chunks (depthFor capacity) index) := by
  -- Index one reads the whole value, while deeper indices select bounded intervals.
  by_cases root : index = 1
  · subst index
    rw [nodeRoot_root, hashTreeRoot_packed layout fits, boundedReading_root]
  · have below : 2 ≤ index := by omega
    have deep : Nat.log2 index ≠ 0 := by
      have recursion := levelOf_parent below
      change Nat.log2 index = _ at recursion
      omega
    cases index with
    | zero => omega
    | succ previous =>
      simp only [nodeRoot, Nat.add_succ, nodeRootAt]
      simp only [Nat.not_lt.mpr named, if_false, beq_iff_eq, root]
      simp [layout, gindexLength, gindexDepth, Nat.not_lt.mpr named, deep,
        MerkleLayout.packing, Bind.bind, Except.bind]
      exact boundedNode_packed _ chunks (some capacity) none (previous + 1)
        (Nat.log2 (previous + 1)) 0 capacity inside

/-- A branch entirely within a bounded binary tree satisfies every required parent equation. -/
theorem boundedReading_branch (chunks : Array Bytes) (height index : Nat)
    (named : 1 ≤ index) (inside : Nat.log2 index ≤ height) :
    BranchConsistent (boundedReading chunks height) index := by
  -- The current ancestor and its sibling are precisely the two children of the next ancestor.
  intro level below
  have positive := path_shift_positive named below
  have bounded : levelOf (index >>> level) ≤ height :=
    Nat.le_trans (levelOf_mono (by omega) (shiftRight_le _ _)) inside
  have parentDepth := levelOf_parent positive
  have equation := boundedReading_parent chunks height ((index >>> level) / 2)
    (by omega) (by change levelOf _ < height; omega)
  rw [shiftRight_succ, gindexBit_parity]
  simp only [decide_eq_true_eq]
  rcases Nat.mod_two_eq_zero_or_one (index >>> level) with even | odd
  · rw [if_neg (by omega)]
    obtain ⟨left, right⟩ := gindexSibling_even even
    rw [right]
    exact (congrArg (fun at_ => combine (boundedReading chunks height at_)
      (boundedReading chunks height (2 * ((index >>> level) / 2) + 1))) left).trans equation.symm
  · rw [if_pos odd]
    obtain ⟨right, left⟩ := gindexSibling_odd odd
    rw [left]
    exact (congrArg (fun at_ => combine (boundedReading chunks height (2 * ((index >>> level) / 2)))
      (boundedReading chunks height at_)) right).trans equation.symm

/-- A proof built inside a bounded packed value rebuilds the value's root. -/
theorem buildProof_packed_rebuilds_root {shape : Desc} {value : Value}
    {chunks : Array Bytes} {capacity index : Nat} {indices : List Nat}
    (layout : merkleLayout shape value = .ok (.packing chunks (some capacity) none))
    (fits : chunks.size ≤ capacity)
    (indexed : getBranchIndices index = .ok indices)
    (inside : Nat.log2 index ≤ depthFor capacity) :
    buildProof shape value index =
      .ok (indices.map (boundedReading chunks (depthFor capacity))) ∧
    calculateMerkleRoot (boundedReading chunks (depthFor capacity) index)
      (indices.map (boundedReading chunks (depthFor capacity))) index = hashTreeRoot shape value := by
  -- Every sibling remains at the same depth as its corresponding ancestor.
  obtain ⟨spelled, named, _⟩ := getBranchIndices_eq indexed
  -- Every requested sibling stays within the already-established readable tree positions.
  have reads : ∀ position ∈ indices, nodeRoot shape value position =
      .ok (boundedReading chunks (depthFor capacity) position) := by
    intro position member
    rw [spelled] at member
    obtain ⟨step, below, same⟩ := List.mem_map.mp member
    subst position
    have positive := path_shift_positive named (List.mem_range.mp below)
    have half := gindexSibling_half (index >>> step)
    apply nodeRoot_packed layout fits _ (by omega)
    rw [show Nat.log2 (gindexSibling (index >>> step)) = Nat.log2 (index >>> step) from
      levelOf_sibling positive]
    exact Nat.le_trans (levelOf_mono (by omega) (shiftRight_le _ _)) inside
  constructor
  · simp [buildProof, indexed, mapM_of_ok_on _ _ indices reads, Bind.bind, Except.bind]
  · rw [branch_rebuilds_root_on_path (boundedReading_branch chunks _ index named inside) indexed,
      hashTreeRoot_packed layout fits, boundedReading_root]

/-- A bounded tree with one word mixed in as its right child. -/
def mixedReading (chunks : Array Bytes) (height : Nat) (word : Bytes) (index : Nat) : Bytes :=
  -- The root combines contents and word, while index three names the word directly.
  if index = 1 then combine (subtreeAt chunks height 0) word
  else if index = 3 then word
  else boundedTreeNode chunks height index (Nat.log2 index - 1) 0

/-- The root of a packed layout with a mixed-in word combines its contents and that word. -/
theorem hashTreeRoot_packed_mixed {shape : Desc} {value : Value} {chunks : Array Bytes}
    {capacity : Nat} {word : Bytes}
    (layout : merkleLayout shape value = .ok (.packing chunks (some capacity) (some word)))
    (fits : chunks.size ≤ capacity) :
    hashTreeRoot shape value = .ok (combine (subtreeAt chunks (depthFor capacity) 0) word) := by
  -- The word occupies its own right child after the bounded contents have been hashed.
  have positive := shape.nesting_pos
  unfold hashTreeRoot
  cases nesting : shape.nesting with
  | zero => omega
  | succ budget =>
    simp [hashTreeRootAt, layout, layoutChunksAt, MerkleLayout.packing, mixIn,
      merkleizeBounded, Leaves.count, Nat.not_lt.mpr fits, Bind.bind, Except.bind,
      Pure.pure, Except.pure]

/-- The mixed root is exactly its bounded contents and its single right-hand word. -/
theorem mixedReading_root (chunks : Array Bytes) (height : Nat) (word : Bytes) :
    mixedReading chunks height word 1 =
      combine (mixedReading chunks height word 2) (mixedReading chunks height word 3) := by
  -- Position two consumes the mixing turn but no turn inside the contents.
  have depth : Nat.log2 2 = 1 := rfl
  simp [mixedReading, boundedTreeNode, depth, gindexBelow]

/-- A child keeps the parent's higher branch turns. -/
theorem gindexBit_children (index level : Nat) :
    gindexBit (2 * index) (level + 1) = gindexBit index level ∧
    gindexBit (2 * index + 1) (level + 1) = gindexBit index level := by
  -- Reading above the lowest bit is reading the halved index.
  have left : 2 * index / 2 = index := by omega
  have right : (2 * index + 1) / 2 = index := by omega
  simp [gindexBit, Nat.testBit_add_one, left, right]

/-- Internal nodes of the mixed tree's left subtree obey the ordinary binary parent equation. -/
theorem mixedReading_parent (chunks : Array Bytes) (height : Nat) (word : Bytes) (index : Nat)
    (named : 2 ≤ index) (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (internal : Nat.log2 index - 1 < height) :
    mixedReading chunks height word index =
      combine (mixedReading chunks height word (2 * index))
        (mixedReading chunks height word (2 * index + 1)) := by
  -- The left subtree uses the same intervals after its initial mixing turn is removed.
  have positive : 0 < Nat.log2 index := by
    have recursion := levelOf_parent named
    change Nat.log2 index = _ at recursion
    omega
  -- A node whose first turn goes left cannot name the right-hand mixing word.
  have notWord : index ≠ 3 := by
    intro wordIndex
    subst index
    change true = false at leftSide
    contradiction
  have left : Nat.log2 (2 * index) = Nat.log2 index + 1 := Nat.log2_two_mul (by omega)
  have right : Nat.log2 (2 * index + 1) = Nat.log2 index + 1 := by
    have recursion := levelOf_parent (index := 2 * index + 1) (by omega)
    have half : (2 * index + 1) / 2 = index := by omega
    simpa only [levelOf, half] using recursion
  have depth : Nat.log2 index = Nat.log2 index - 1 + 1 := by omega
  simp only [mixedReading, if_neg (by omega : index ≠ 1), if_neg notWord,
    if_neg (by omega : 2 * index ≠ 1), if_neg (by omega : 2 * index ≠ 3),
    if_neg (by omega : 2 * index + 1 ≠ 1), if_neg (by omega : 2 * index + 1 ≠ 3),
    left, right, Nat.add_sub_cancel]
  rw [boundedTreeNode_split chunks height index (Nat.log2 index - 1) 0 internal]
  rw [← depth]

/-- The actual walker reads a mixed-in word or any node in the bounded contents. -/
theorem nodeRoot_packed_mixed {shape : Desc} {value : Value} {chunks : Array Bytes}
    {capacity : Nat} {word : Bytes}
    (layout : merkleLayout shape value = .ok (.packing chunks (some capacity) (some word)))
    (fits : chunks.size ≤ capacity) (index : Nat)
    (readable : index = 1 ∨ index = 3 ∨
      (2 ≤ index ∧ gindexBit index (Nat.log2 index - 1) = false ∧
        Nat.log2 index - 1 ≤ depthFor capacity)) :
    nodeRoot shape value index = .ok (mixedReading chunks (depthFor capacity) word index) := by
  -- The right child is a leaf, while the left child opens the bounded content tree.
  rcases readable with root | wordIndex | ⟨named, leftSide, inside⟩
  · subst index
    rw [nodeRoot_root, hashTreeRoot_packed_mixed layout fits]
    simp [mixedReading]
  · subst index
    have depth : gindexLength 3 = .ok 1 := rfl
    simp [nodeRoot, nodeRootAt, layout, depth, MerkleLayout.packing,
      gindexBit, mixedReading, Bind.bind, Except.bind, Pure.pure, Except.pure]
  · have notRoot : index ≠ 1 := by omega
    -- A node whose first turn goes left cannot name the right-hand mixing word.
    have notWord : index ≠ 3 := by
      intro same
      subst index
      change true = false at leftSide
      contradiction
    have positive : Nat.log2 index ≠ 0 := by
      have recursion := levelOf_parent named
      change Nat.log2 index = _ at recursion
      omega
    cases index with
    | zero => omega
    | succ previous =>
      simp only [nodeRoot, Nat.add_succ, nodeRootAt]
      simp only [show ¬previous + 1 < 1 by omega, if_false, beq_iff_eq, notRoot]
      simp [layout, gindexLength, gindexDepth, show ¬previous + 1 < 1 by omega,
        positive, leftSide, MerkleLayout.packing, Bind.bind, Except.bind,
        mixedReading, show previous ≠ 0 by omega, notWord]
      exact boundedNode_packed _ chunks (some capacity) (some word) (previous + 1)
        (Nat.log2 (previous + 1) - 1) 0 capacity inside

/-- Removing a branch turn decreases the depth by one until the root is reached. -/
theorem levelOf_shift {index : Nat} (named : 1 ≤ index) :
    ∀ step, step ≤ Nat.log2 index → levelOf (index >>> step) + step = Nat.log2 index := by
  -- Every intermediate ancestor is below the root until the final shift.
  intro step
  induction step with
  | zero => simp [levelOf]
  | succ step ih =>
    intro bounded
    have previous := ih (by omega)
    have parent := levelOf_parent (path_shift_positive named (by omega : step < Nat.log2 index))
    rw [shiftRight_succ]
    omega

/-- An ancestor below the root keeps the first turn of the original branch. -/
theorem gindexBit_shift_top {index step : Nat} (named : 1 ≤ index)
    (below : step < Nat.log2 index) :
    gindexBit (index >>> step) (Nat.log2 (index >>> step) - 1) =
      gindexBit index (Nat.log2 index - 1) := by
  -- Shifting drops lower turns, so the top turn moves down by the same number of places.
  have depth := levelOf_shift named step (by omega)
  change Nat.log2 (index >>> step) + step = Nat.log2 index at depth
  have position : step + (Nat.log2 (index >>> step) - 1) = Nat.log2 index - 1 := by omega
  simp [gindexBit, Nat.testBit_shiftRight, position]

/-- A branch in the bounded contents of a mixed tree satisfies every parent equation. -/
theorem mixedReading_branch_left (chunks : Array Bytes) (height : Nat) (word : Bytes) (index : Nat)
    (named : 2 ≤ index) (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (inside : Nat.log2 index - 1 ≤ height) :
    BranchConsistent (mixedReading chunks height word) index := by
  -- Below the mixing root, parent equations come from the bounded contents.
  intro level below
  have positive := path_shift_positive (by omega : 1 ≤ index) below
  have currentDepth := levelOf_shift (by omega : 1 ≤ index) level (by omega)
  have parentDepth := levelOf_parent positive
  have equation : mixedReading chunks height word ((index >>> level) / 2) =
      combine (mixedReading chunks height word (2 * ((index >>> level) / 2)))
        (mixedReading chunks height word (2 * ((index >>> level) / 2) + 1)) := by
    by_cases root : (index >>> level) / 2 = 1
    · simpa only [root] using mixedReading_root chunks height word
    · have parentPositive : 2 ≤ (index >>> level) / 2 := by omega
      -- Before the root boundary, the next ancestor retains the original first contents turn.
      have stillBelow : level + 1 < Nat.log2 index := by
        have above := levelOf_parent parentPositive
        omega
      have turn := gindexBit_shift_top (by omega : 1 ≤ index) stillBelow
      rw [shiftRight_succ] at turn
      exact mixedReading_parent chunks height word _ parentPositive
        (turn.trans leftSide) (by change levelOf _ - 1 < height; omega)
  -- Parity determines which of the two children is supplied as the branch sibling.
  rw [shiftRight_succ, gindexBit_parity]
  simp only [decide_eq_true_eq]
  rcases Nat.mod_two_eq_zero_or_one (index >>> level) with even | odd
  · rw [if_neg (by omega)]
    obtain ⟨left, right⟩ := gindexSibling_even even
    rw [right]
    exact (congrArg (fun at_ => combine (mixedReading chunks height word at_)
      (mixedReading chunks height word (2 * ((index >>> level) / 2) + 1))) left).trans equation.symm
  · rw [if_pos odd]
    obtain ⟨right, left⟩ := gindexSibling_odd odd
    rw [left]
    exact (congrArg (fun at_ => combine (mixedReading chunks height word (2 * ((index >>> level) / 2)))
      (mixedReading chunks height word at_)) right).trans equation.symm

/-- Swapping siblings changes only the lowest turn of their indices. -/
theorem gindexBit_sibling_high (index position : Nat) (above : 0 < position) :
    gindexBit (gindexSibling index) position = gindexBit index position := by
  -- Any higher bit is read from the common parent.
  cases position with
  | zero => omega
  | succ position => simp [gindexBit, Nat.testBit_add_one, gindexSibling_half]

/-- Siblings along a branch in the left subtree are readable contents or the mixed-in word. -/
theorem mixed_branch_sibling {index step height : Nat}
    (named : 2 ≤ index) (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (inside : Nat.log2 index - 1 ≤ height) (below : step < Nat.log2 index) :
    let sibling := gindexSibling (index >>> step)
    sibling = 3 ∨ (2 ≤ sibling ∧ gindexBit sibling (Nat.log2 sibling - 1) = false ∧
      Nat.log2 sibling - 1 ≤ height) := by
  -- Only the last sibling is outside the left subtree, at position three.
  dsimp only
  have positive := path_shift_positive (by omega : 1 ≤ index) below
  have half := gindexSibling_half (index >>> step)
  have depth := levelOf_shift (by omega : 1 ≤ index) step (by omega)
  have siblingDepth := levelOf_sibling positive
  have turn := (gindexBit_shift_top (by omega : 1 ≤ index) below).trans leftSide
  -- The only sibling outside the contents subtree is its final right-hand mixing word.
  by_cases word : gindexSibling (index >>> step) = 3
  · exact Or.inl word
  · right
    have high : 0 < Nat.log2 (index >>> step) - 1 := by
      have positiveDepth := levelOf_parent positive
      by_cases low : Nat.log2 (index >>> step) = 1
      · have bounds := gindexDepth_bounds (by omega : 1 ≤ index >>> step)
        have upper : index >>> step < 4 := by simpa [low] using bounds.2
        have cases : index >>> step = 2 ∨ index >>> step = 3 := by omega
        rcases cases with two | three
        · have sibling : gindexSibling (index >>> step) = 3 := by rw [two]; rfl
          exact False.elim (word sibling)
        · rw [three] at turn
          change true = false at turn
          contradiction
      · change Nat.log2 (index >>> step) = _ at positiveDepth
        omega
    refine ⟨by omega, ?_, ?_⟩
    · change Nat.log2 (gindexSibling (index >>> step)) = Nat.log2 (index >>> step) at siblingDepth
      rw [siblingDepth, gindexBit_sibling_high _ _ high]
      exact turn
    · change levelOf (gindexSibling (index >>> step)) - 1 ≤ height
      omega

/-- Constructed branches inside packed bounded contents rebuild the mixed value's root. -/
theorem buildProof_packed_mixed_rebuilds_root {shape : Desc} {value : Value}
    {chunks : Array Bytes} {capacity index : Nat} {word : Bytes} {indices : List Nat}
    (layout : merkleLayout shape value = .ok (.packing chunks (some capacity) (some word)))
    (fits : chunks.size ≤ capacity)
    (indexed : getBranchIndices index = .ok indices)
    (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (inside : Nat.log2 index - 1 ≤ depthFor capacity) :
    buildProof shape value index =
      .ok (indices.map (mixedReading chunks (depthFor capacity) word)) ∧
    calculateMerkleRoot (mixedReading chunks (depthFor capacity) word index)
      (indices.map (mixedReading chunks (depthFor capacity) word)) index = hashTreeRoot shape value := by
  -- Each sibling is either another bounded interval or the one final mixing word.
  obtain ⟨spelled, named, nonzero⟩ := getBranchIndices_eq indexed
  have positive : 2 ≤ index := two_le_of_level_positive (depth := Nat.log2 index) rfl (by omega)
  -- Every requested sibling stays within the already-established readable tree positions.
  have reads : ∀ position ∈ indices, nodeRoot shape value position =
      .ok (mixedReading chunks (depthFor capacity) word position) := by
    intro position member
    rw [spelled] at member
    obtain ⟨step, below, same⟩ := List.mem_map.mp member
    subst position
    apply nodeRoot_packed_mixed layout fits _
    exact Or.inr (mixed_branch_sibling positive leftSide inside (List.mem_range.mp below))
  constructor
  · simp [buildProof, indexed, mapM_of_ok_on _ _ indices reads, Bind.bind, Except.bind]
  · rw [branch_rebuilds_root_on_path
      (mixedReading_branch_left chunks _ word index positive leftSide inside) indexed,
      hashTreeRoot_packed_mixed layout fits]
    simp [mixedReading]

/-- A length or selector word is authenticated by the bounded contents beside it. -/
theorem buildProof_packed_word_rebuilds_root {shape : Desc} {value : Value}
    {chunks : Array Bytes} {capacity : Nat} {word : Bytes}
    (layout : merkleLayout shape value = .ok (.packing chunks (some capacity) (some word)))
    (fits : chunks.size ≤ capacity) :
    buildProof shape value 3 = .ok [subtreeAt chunks (depthFor capacity) 0] ∧
    calculateMerkleRoot word [subtreeAt chunks (depthFor capacity) 0] 3 = hashTreeRoot shape value := by
  -- Position three's only sibling is position two, the root of the contents.
  have depth : Nat.log2 2 = 1 := rfl
  have read := nodeRoot_packed_mixed layout fits 2
    (Or.inr (Or.inr ⟨by decide, rfl, by simp [depth]⟩))
  have branch : getBranchIndices 3 = .ok [2] := by with_unfolding_all rfl
  constructor
  · simp [buildProof, branch, read, mixedReading, boundedTreeNode, depth, gindexBelow,
      Bind.bind, Except.bind, Pure.pure, Except.pure]
  · rw [hashTreeRoot_packed_mixed layout fits]
    rfl

end Ssz
