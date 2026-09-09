import Ssz.Codec.ProofBudget
import Ssz.Codec.ProofTree

/-! Crossing a composite leaf preserves the nested value's root and branch turns. -/

namespace Ssz

/-- Below a filled composite leaf, the bounded walker is exactly the nested value's walker. -/
theorem boundedNode_nested (layout : MerkleLayout) (budget : Nat)
    (slots : List (Option (Desc × Value))) (nested : layout.leaves = .nested slots)
    (child : Desc) (inner : Value) (enough : child.nesting ≤ budget)
    (index depth start capacity : Nat) (deeper : depthFor capacity < depth)
    (selected : slots[start + gindexBelow (index >>> (depth - depthFor capacity))
      (depthFor capacity)]? = some (some (child, inner))) :
    boundedNode budget layout index depth start capacity =
      nodeRoot child inner (gindexRebase index (depth - depthFor capacity)) := by
  -- The outer tree consumes its height, leaving the remaining turns inside the child.
  simp only [boundedNode, nextPow2, Nat.log2_two_pow, if_neg (Nat.not_le.mpr deeper), nested, selected]
  exact nodeRootAt_eq_nodeRoot child inner _ budget enough

/-- An enclosing type's sufficient budget also covers every child reached through its layout. -/
theorem boundedNode_nested_of_layout (shape : Desc) (value : Value) (layout : MerkleLayout)
    (laidOut : merkleLayout shape value = .ok layout) (budget : Nat)
    (enough : shape.nesting ≤ budget + 1)
    (slots : List (Option (Desc × Value))) (nested : layout.leaves = .nested slots)
    (child : Desc) (inner : Value) (index depth start capacity : Nat)
    (deeper : depthFor capacity < depth)
    (selected : slots[start + gindexBelow (index >>> (depth - depthFor capacity))
      (depthFor capacity)]? = some (some (child, inner))) :
    boundedNode budget layout index depth start capacity =
      nodeRoot child inner (gindexRebase index (depth - depthFor capacity)) := by
  -- A filled slot names a strictly shallower type, so entering it spends at most one level.
  have smaller := merkleLayout_child_nesting shape value layout laidOut slots nested
    child inner (List.mem_of_getElem? selected)
  exact boundedNode_nested layout budget slots nested child inner (by omega)
    index depth start capacity deeper selected

/-- At a filled composite leaf, the outer tree commits to exactly the child's own root. -/
theorem boundedNode_nested_leaf (layout : MerkleLayout) (budget : Nat)
    (slots : List (Option (Desc × Value))) (nested : layout.leaves = .nested slots)
    (child : Desc) (inner : Value) (enough : child.nesting ≤ budget)
    (index start capacity : Nat)
    (selected : slots[start + gindexBelow index (depthFor capacity)]? =
      some (some (child, inner))) :
    boundedNode budget layout index (depthFor capacity) start capacity = hashTreeRoot child inner := by
  -- A leaf interval has width one, so merkleization adds no extra hash around the child's root.
  obtain ⟨inside, atLeaf⟩ := List.getElem?_eq_some_iff.mp selected
  -- A one-position interval materializes exactly the selected child root.
  have one := layoutChunksAt_singleton budget layout slots nested
    (start + gindexBelow index (depthFor capacity)) inside
  simp only [atLeaf] at one
  simp only [boundedNode, nextPow2, Nat.log2_two_pow, Nat.le_refl, ↓reduceIte,
    Nat.shiftRight_eq_div_pow, Nat.div_self (Nat.two_pow_pos _), Nat.mul_one]
  rw [one, hashTreeRootAt_eq_hashTreeRoot child inner budget enough]
  -- The enclosing leaf preserves both the child’s successful root and any child-rooting error.
  cases root : hashTreeRoot child inner with
  | error e => rfl
  | ok bytes =>
    simp [merkleizeBounded, subtreeAt, depthFor, padded, Bind.bind, Except.bind,
      Pure.pure, Except.pure]

/-- Removing an outer path prefix preserves each left or right child turn. -/
theorem gindexRebase_children (index depth : Nat) :
    gindexRebase (2 * index) (depth + 1) = 2 * gindexRebase index depth ∧
    gindexRebase (2 * index + 1) (depth + 1) = 2 * gindexRebase index depth + 1 := by
  -- The new leading one moves left alongside the retained low branch bits.
  obtain ⟨left, right⟩ := gindexBelow_children index depth
  simp only [gindexRebase, left, right, Nat.pow_succ, Nat.mul_add]
  constructor <;> omega

/-- At or below a filled leaf, the outer walk reads the nested value at the rebased index. -/
theorem boundedNode_nested_at (layout : MerkleLayout) (budget : Nat)
    (slots : List (Option (Desc × Value))) (nested : layout.leaves = .nested slots)
    (child : Desc) (inner : Value) (enough : child.nesting ≤ budget)
    (index depth start capacity : Nat) (reached : depthFor capacity ≤ depth)
    (selected : slots[start + gindexBelow (index >>> (depth - depthFor capacity))
      (depthFor capacity)]? = some (some (child, inner))) :
    boundedNode budget layout index depth start capacity =
      nodeRoot child inner (gindexRebase index (depth - depthFor capacity)) := by
  -- At the boundary the remaining path is empty, naming the child's root at index one.
  by_cases boundary : depth = depthFor capacity
  · subst depth
    simpa only [Nat.sub_self, gindexRebase, gindexBelow, Nat.pow_zero, Nat.mod_one,
      Nat.add_zero, nodeRoot_root] using
      boundedNode_nested_leaf layout budget slots nested child inner enough index start capacity
        (by simpa using selected)
  · exact boundedNode_nested layout budget slots nested child inner enough index depth start
      capacity (by omega) selected

private theorem shift_children (index depth : Nat) :
    (2 * index) >>> (depth + 1) = index >>> depth ∧
    (2 * index + 1) >>> (depth + 1) = index >>> depth := by
  -- Dropping the new final turn first recovers the same original path.
  rw [Nat.add_comm depth 1, Nat.shiftRight_add, Nat.shiftRight_add]
  have left : (2 * index) >>> 1 = index := by simp [Nat.shiftRight_eq_div_pow]
  have right : (2 * index + 1) >>> 1 = index := by
    simp only [Nat.shiftRight_eq_div_pow]
    omega
  simp [left, right]

/-- Parent equations inside a nested value remain parent equations after an outer path prefix. -/
theorem boundedNode_nested_split (layout : MerkleLayout) (budget : Nat)
    (slots : List (Option (Desc × Value))) (nested : layout.leaves = .nested slots)
    (child : Desc) (inner : Value) (enough : child.nesting ≤ budget)
    (index depth start capacity : Nat) (reached : depthFor capacity ≤ depth)
    (selected : slots[start + gindexBelow (index >>> (depth - depthFor capacity))
      (depthFor capacity)]? = some (some (child, inner)))
    (parent : nodeRoot child inner (gindexRebase index (depth - depthFor capacity)) = (do
      let left ← nodeRoot child inner (2 * gindexRebase index (depth - depthFor capacity))
      let right ← nodeRoot child inner (2 * gindexRebase index (depth - depthFor capacity) + 1)
      pure (combine left right))) :
    boundedNode budget layout index depth start capacity = (do
      let left ← boundedNode budget layout (2 * index) (depth + 1) start capacity
      let right ← boundedNode budget layout (2 * index + 1) (depth + 1) start capacity
      pure (combine left right)) := by
  -- Both child turns stay in the same outer leaf and become the corresponding nested turns.
  have remaining : depth + 1 - depthFor capacity = (depth - depthFor capacity) + 1 := by omega
  obtain ⟨leftShift, rightShift⟩ := shift_children index (depth - depthFor capacity)
  -- Appending a child turn changes the nested address without selecting a different outer leaf.
  have leftSelected : slots[start + gindexBelow
      ((2 * index) >>> (depth + 1 - depthFor capacity)) (depthFor capacity)]? =
      some (some (child, inner)) := by simpa [remaining, leftShift] using selected
  -- The right child reaches that same outer leaf before taking its final nested turn.
  have rightSelected : slots[start + gindexBelow
      ((2 * index + 1) >>> (depth + 1 - depthFor capacity)) (depthFor capacity)]? =
      some (some (child, inner)) := by simpa [remaining, rightShift] using selected
  rw [boundedNode_nested_at layout budget slots nested child inner enough
    index depth start capacity reached selected]
  rw [boundedNode_nested_at layout budget slots nested child inner enough
    (2 * index) (depth + 1) start capacity (by omega) leftSelected]
  rw [boundedNode_nested_at layout budget slots nested child inner enough
    (2 * index + 1) (depth + 1) start capacity (by omega) rightSelected]
  -- Restoring a leading bit to the suffix preserves the order of its two nested children.
  obtain ⟨leftTurn, rightTurn⟩ := gindexRebase_children index (depth - depthFor capacity)
  simpa only [remaining, leftTurn, rightTurn] using parent

/-- Rebasing commutes with moving one level upward while a nested branch still has turns. -/
theorem gindexRebase_parent (index depth : Nat) :
    gindexRebase index (depth + 1) / 2 = gindexRebase (index / 2) depth := by
  -- Every index is either its parent's left child or its parent's right child.
  obtain ⟨left, right⟩ := gindexRebase_children (index / 2) depth
  -- The lowest retained turn identifies which of the common parent’s children is being rebased.
  rcases Nat.mod_two_eq_zero_or_one index with even | odd
  · have child : index = 2 * (index / 2) := by omega
    calc
      gindexRebase index (depth + 1) / 2 = (2 * gindexRebase (index / 2) depth) / 2 :=
        congrArg (· / 2) ((congrArg (gindexRebase · (depth + 1)) child).trans left)
      _ = _ := by omega
  · have child : index = 2 * (index / 2) + 1 := by omega
    calc
      gindexRebase index (depth + 1) / 2 = (2 * gindexRebase (index / 2) depth + 1) / 2 :=
        congrArg (· / 2) ((congrArg (gindexRebase · (depth + 1)) child).trans right)
      _ = _ := by omega

/-- Rebasing preserves sibling positions until the outer leaf boundary is reached. -/
theorem gindexRebase_sibling (index depth : Nat) :
    gindexRebase (gindexSibling index) (depth + 1) =
      gindexSibling (gindexRebase index (depth + 1)) := by
  -- The sibling operation changes only the last retained turn.
  obtain ⟨left, right⟩ := gindexRebase_children (index / 2) depth
  -- The lowest retained turn identifies which of the common parent’s children is being rebased.
  rcases Nat.mod_two_eq_zero_or_one index with even | odd
  · obtain ⟨child, sibling⟩ := gindexSibling_even even
    have rebased := (congrArg (gindexRebase · (depth + 1)) child).trans left
    rw [sibling, right, rebased]
    simpa using (gindexSibling_even (by omega : (2 * gindexRebase (index / 2) depth) % 2 = 0)).2.symm
  · obtain ⟨child, sibling⟩ := gindexSibling_odd odd
    have rebased := (congrArg (gindexRebase · (depth + 1)) child).trans right
    rw [sibling, left, rebased]
    have half : (2 * gindexRebase (index / 2) depth + 1) / 2 = gindexRebase (index / 2) depth := by omega
    simpa only [half] using
      (gindexSibling_odd (by omega : (2 * gindexRebase (index / 2) depth + 1) % 2 = 1)).2.symm

end Ssz
