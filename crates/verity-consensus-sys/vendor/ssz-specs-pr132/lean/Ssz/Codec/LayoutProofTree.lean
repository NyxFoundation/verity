import Ssz.Codec.ProgressiveProofTree
import Ssz.Codec.ProofBudget

/-! A value's materialized leaves determine its top-level proof tree, including nested field roots. -/

namespace Ssz

/-- A bounded layout hashes its materialized leaves, then its optional mixing word. -/
theorem hashTreeRoot_bounded_layout {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {capacity : Nat}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (bounded : layout.limit = some capacity) (fits : chunks.size ≤ capacity) :
    hashTreeRoot shape value = .ok (match layout.mixin with
      | none => subtreeAt chunks (depthFor capacity) 0
      | some word => combine (subtreeAt chunks (depthFor capacity) 0) word) := by
  -- The type's positive nesting depth leaves one budget unit for its own layout.
  have positive := shape.nesting_pos
  unfold hashTreeRoot
  cases nesting : shape.nesting with
  | zero => omega
  | succ budget =>
    -- The enclosing type consumes one recursion level, matching the budget used for its materialized leaves.
    simp only [nesting, Nat.add_sub_cancel] at materialized
    simp [hashTreeRootAt, laid, materialized, bounded, merkleizeBounded, Nat.not_lt.mpr fits,
      mixIn, Bind.bind, Except.bind, Pure.pure, Except.pure]
    rfl

/-- A progressive layout hashes its materialized spine, then its optional mixing word. -/
theorem hashTreeRoot_progressive_layout {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (progressive : layout.limit = none) :
    hashTreeRoot shape value = .ok (match layout.mixin with
      | none => merkleizeProgressive chunks.toList
      | some word => combine (merkleizeProgressive chunks.toList) word) := by
  -- The suffix construction reads only the resulting leaf array, regardless of leaf types.
  have positive := shape.nesting_pos
  unfold hashTreeRoot
  cases nesting : shape.nesting with
  | zero => omega
  | succ budget =>
    -- The enclosing type consumes one recursion level, matching the budget used for its materialized leaves.
    simp only [nesting, Nat.add_sub_cancel] at materialized
    simp [hashTreeRootAt, laid, materialized, progressive, mixIn,
      Bind.bind, Except.bind, Pure.pure, Except.pure]
    rfl

/-- Every top-level node in an unmixed bounded layout is its corresponding binary subtree. -/
theorem nodeRoot_bounded_layout {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {capacity : Nat}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (bounded : layout.limit = some capacity) (unmixed : layout.mixin = none)
    (fits : chunks.size ≤ capacity) (index : Nat) (named : 1 ≤ index)
    (inside : Nat.log2 index ≤ depthFor capacity) :
    nodeRoot shape value index = .ok (boundedReading chunks (depthFor capacity) index) := by
  -- A sufficient walk budget can be replaced by the exact budget used to materialize the leaves.
  by_cases root : index = 1
  · subst index
    rw [nodeRoot_root, hashTreeRoot_bounded_layout laid materialized bounded fits, unmixed,
      boundedReading_root]
  · have positive := shape.nesting_pos
    have below : 2 ≤ index := by omega
    have deep : Nat.log2 index ≠ 0 := by
      have recursion := levelOf_parent below
      change Nat.log2 index = _ at recursion
      omega
    rw [← nodeRootAt_eq_nodeRoot shape value index shape.nesting (Nat.le_refl _)]
    cases nesting : shape.nesting with
    | zero => omega
    | succ budget =>
      -- The enclosing type consumes one recursion level, matching the budget used for its materialized leaves.
      simp only [nesting, Nat.add_sub_cancel] at materialized
      simp [nodeRootAt, laid, bounded, unmixed, Nat.not_lt.mpr named, root,
        gindexLength, gindexDepth, deep, Bind.bind, Except.bind]
      exact boundedNode_window (layoutChunksAt_window budget layout chunks materialized)
        index (Nat.log2 index) 0 capacity inside

/-- Every top-level node of a bounded mixed layout is its contents or its single mixing word. -/
theorem nodeRoot_mixed_layout {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {capacity : Nat} {word : Bytes}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (bounded : layout.limit = some capacity) (mixed : layout.mixin = some word)
    (fits : chunks.size ≤ capacity) (index : Nat)
    (readable : index = 1 ∨ index = 3 ∨
      (2 ≤ index ∧ gindexBit index (Nat.log2 index - 1) = false ∧
        Nat.log2 index - 1 ≤ depthFor capacity)) :
    nodeRoot shape value index = .ok (mixedReading chunks (depthFor capacity) word index) := by
  -- Mixed words end at depth one, while contents continue through their bounded intervals.
  rcases readable with root | wordIndex | ⟨named, leftSide, inside⟩
  · subst index
    rw [nodeRoot_root, hashTreeRoot_bounded_layout laid materialized bounded fits, mixed]
    simp [mixedReading]
  · subst index
    have depth : gindexLength 3 = .ok 1 := rfl
    simp [nodeRoot, nodeRootAt, laid, mixed, depth, gindexBit, mixedReading,
      Bind.bind, Except.bind, Pure.pure, Except.pure]
  · have positive := shape.nesting_pos
    have notRoot : index ≠ 1 := by omega
    -- The contents’ initial left turn excludes the right-hand word even at the same depth.
    have notWord : index ≠ 3 := by
      intro same
      subst index
      change true = false at leftSide
      contradiction
    have deep : Nat.log2 index ≠ 0 := by
      have recursion := levelOf_parent named
      change Nat.log2 index = _ at recursion
      omega
    rw [← nodeRootAt_eq_nodeRoot shape value index shape.nesting (Nat.le_refl _)]
    cases nesting : shape.nesting with
    | zero => omega
    | succ budget =>
      -- The enclosing type consumes one recursion level, matching the budget used for its materialized leaves.
      simp only [nesting, Nat.add_sub_cancel] at materialized
      simp [nodeRootAt, laid, bounded, mixed, show ¬index < 1 by omega, notRoot,
        gindexLength, gindexDepth, deep, leftSide, mixedReading, notWord,
        Bind.bind, Except.bind]
      exact boundedNode_window (layoutChunksAt_window budget layout chunks materialized)
        index (Nat.log2 index - 1) 0 capacity inside

/-- Every top-level progressive node is the corresponding spine position or mixing word. -/
theorem nodeRoot_progressive_layout {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {word : Bytes}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (progressive : layout.limit = none) (mixed : layout.mixin = some word)
    (index : Nat) (readable : index = 1 ∨ index = 3 ∨
      (2 ≤ index ∧ gindexBit index (Nat.log2 index - 1) = false ∧
        ProgressivePosition chunks.size index (Nat.log2 index - 1) 0 0)) :
    nodeRoot shape value index = .ok (progressiveMixedReading chunks word index) := by
  -- Nested values contribute their roots as leaves without changing the enclosing tree shape.
  rcases readable with root | wordIndex | ⟨named, leftSide, position⟩
  · subst index
    rw [nodeRoot_root, hashTreeRoot_progressive_layout laid materialized progressive, mixed]
    simp [progressiveMixedReading]
  · subst index
    have depth : gindexLength 3 = .ok 1 := rfl
    simp [nodeRoot, nodeRootAt, laid, mixed, depth, gindexBit, progressiveMixedReading,
      Bind.bind, Except.bind, Pure.pure, Except.pure]
  · have positive := shape.nesting_pos
    have notRoot : index ≠ 1 := by omega
    -- The contents’ initial left turn excludes the right-hand word even at the same depth.
    have notWord : index ≠ 3 := by
      intro same
      subst index
      change true = false at leftSide
      contradiction
    have deep : Nat.log2 index ≠ 0 := by
      have recursion := levelOf_parent named
      change Nat.log2 index = _ at recursion
      omega
    rw [← nodeRootAt_eq_nodeRoot shape value index shape.nesting (Nat.le_refl _)]
    cases nesting : shape.nesting with
    | zero => omega
    | succ budget =>
      -- The enclosing type consumes one recursion level, matching the budget used for its materialized leaves.
      simp only [nesting, Nat.add_sub_cancel] at materialized
      simp [nodeRootAt, laid, progressive, mixed, show ¬index < 1 by omega, notRoot,
        gindexLength, gindexDepth, deep, leftSide, progressiveMixedReading, notWord,
        Bind.bind, Except.bind]
      exact progressiveNode_read materialized index (Nat.log2 index - 1) 0 0 position

/-- Reading a reference tree's siblings transfers its parent equations to the constructed proof. -/
theorem buildProof_from_reading {shape : Desc} {value : Value} {node : Nat → Bytes}
    {index : Nat} {indices : List Nat} (indexed : getBranchIndices index = .ok indices)
    (parents : BranchConsistent node index)
    (reads : ∀ position ∈ indices, nodeRoot shape value position = .ok (node position))
    (root : hashTreeRoot shape value = .ok (node 1)) :
    buildProof shape value index = .ok (indices.map node) ∧
      calculateMerkleRoot (node index) (indices.map node) index = hashTreeRoot shape value := by
  -- The constructor reads exactly the named siblings, and the algebraic branch reaches its root.
  constructor
  · simp [buildProof, indexed, mapM_of_ok_on node _ indices reads, Bind.bind, Except.bind]
  · rw [branch_rebuilds_root_on_path parents indexed, root]

/-- Constructed branches within an unmixed layout rebuild the actual root from its materialized leaves. -/
theorem buildProof_bounded_layout_correct {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {capacity index : Nat} {indices : List Nat}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (bounded : layout.limit = some capacity) (unmixed : layout.mixin = none)
    (fits : chunks.size ≤ capacity) (indexed : getBranchIndices index = .ok indices)
    (inside : Nat.log2 index ≤ depthFor capacity) :
    buildProof shape value index = .ok (indices.map (boundedReading chunks (depthFor capacity))) ∧
      calculateMerkleRoot (boundedReading chunks (depthFor capacity) index)
        (indices.map (boundedReading chunks (depthFor capacity))) index = hashTreeRoot shape value := by
  -- Nested fields are already roots at this level, so the same binary branch laws apply.
  obtain ⟨spelled, named, _⟩ := getBranchIndices_eq indexed
  -- The concrete value walker supplies the same siblings as the finite tree whose parent equations are known.
  apply buildProof_from_reading indexed (boundedReading_branch chunks _ index named inside)
  · intro position member
    rw [spelled] at member
    obtain ⟨step, below, same⟩ := List.mem_map.mp member
    subst position
    have positive := path_shift_positive named (List.mem_range.mp below)
    have half := gindexSibling_half (index >>> step)
    apply nodeRoot_bounded_layout laid materialized bounded unmixed fits _ (by omega)
    rw [show Nat.log2 (gindexSibling (index >>> step)) = Nat.log2 (index >>> step) from
      levelOf_sibling positive]
    exact Nat.le_trans (levelOf_mono (by omega) (shiftRight_le _ _)) inside
  · rw [hashTreeRoot_bounded_layout laid materialized bounded fits, unmixed, boundedReading_root]

/-- Constructed branches in mixed bounded contents rebuild the actual root, including nested leaves. -/
theorem buildProof_mixed_layout_correct {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {capacity index : Nat} {word : Bytes} {indices : List Nat}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (bounded : layout.limit = some capacity) (mixed : layout.mixin = some word)
    (fits : chunks.size ≤ capacity) (indexed : getBranchIndices index = .ok indices)
    (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (inside : Nat.log2 index - 1 ≤ depthFor capacity) :
    buildProof shape value index = .ok (indices.map (mixedReading chunks (depthFor capacity) word)) ∧
      calculateMerkleRoot (mixedReading chunks (depthFor capacity) word index)
        (indices.map (mixedReading chunks (depthFor capacity) word)) index = hashTreeRoot shape value := by
  -- Every sibling is an enclosing bounded interval or the final mixing word.
  obtain ⟨spelled, named, nonzero⟩ := getBranchIndices_eq indexed
  -- A nonempty branch starts below the root, preserving a leading path turn for its contents.
  have positive : 2 ≤ index := two_le_of_level_positive (depth := Nat.log2 index) rfl (by omega)
  -- The concrete value walker supplies the same siblings as the finite tree whose parent equations are known.
  apply buildProof_from_reading indexed (mixedReading_branch_left chunks _ word index positive leftSide inside)
  · intro position member
    rw [spelled] at member
    obtain ⟨step, below, same⟩ := List.mem_map.mp member
    subst position
    apply nodeRoot_mixed_layout laid materialized bounded mixed fits _
    exact Or.inr (mixed_branch_sibling positive leftSide inside (List.mem_range.mp below))
  · rw [hashTreeRoot_bounded_layout laid materialized bounded fits, mixed]
    simp [mixedReading]

/-- Constructed branches within a progressive layout rebuild the actual root, including nested leaves. -/
theorem buildProof_progressive_layout_correct {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {word : Bytes} {index : Nat} {indices : List Nat}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (progressive : layout.limit = none) (mixed : layout.mixin = some word)
    (indexed : getBranchIndices index = .ok indices)
    (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (position : ProgressivePosition chunks.size index (Nat.log2 index - 1) 0 0) :
    buildProof shape value index = .ok (indices.map (progressiveMixedReading chunks word)) ∧
      calculateMerkleRoot (progressiveMixedReading chunks word index)
        (indices.map (progressiveMixedReading chunks word)) index = hashTreeRoot shape value := by
  -- Spine and bounded-level parent equations have already been proved from the materialized leaves.
  obtain ⟨spelled, named, nonzero⟩ := getBranchIndices_eq indexed
  -- A nonempty branch starts below the root, preserving a leading path turn for its contents.
  have positive : 2 ≤ index := two_le_of_level_positive (depth := Nat.log2 index) rfl (by omega)
  -- The concrete value walker supplies the same siblings as the finite tree whose parent equations are known.
  apply buildProof_from_reading indexed (progressiveMixedReading_branch_left chunks word index positive leftSide position)
  · intro at_ member
    rw [spelled] at member
    obtain ⟨step, below, same⟩ := List.mem_map.mp member
    subst at_
    apply nodeRoot_progressive_layout laid materialized progressive mixed _
    exact Or.inr (progressive_mixed_branch_sibling positive leftSide position (List.mem_range.mp below))
  · rw [hashTreeRoot_progressive_layout laid materialized progressive, mixed]
    simp [progressiveMixedReading]

/-- Reading the right child of any mixed layout returns its word. -/
theorem nodeRoot_mixin {shape : Desc} {value : Value} {layout : MerkleLayout} {word : Bytes}
    (laid : merkleLayout shape value = .ok layout) (mixed : layout.mixin = some word) :
    nodeRoot shape value 3 = .ok word := by
  -- The right child ends after the mixing turn, so its leaves and capacity are irrelevant.
  have depth : gindexLength 3 = .ok 1 := rfl
  simp [nodeRoot, nodeRootAt, laid, mixed, depth, gindexBit,
    Bind.bind, Except.bind, Pure.pure, Except.pure]

end Ssz
