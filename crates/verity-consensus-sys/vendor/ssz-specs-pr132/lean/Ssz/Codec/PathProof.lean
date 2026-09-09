import Ssz.Codec.LayoutProofTree
import Ssz.Codec.NestedProof
import Ssz.Type.PathLaws
import Ssz.Codec.ProgressiveIndex

/-! Type paths select the same positions that the value walker reads. -/

namespace Ssz

/-- A reserved path step selects the right child carrying that type's mixed-in word. -/
theorem getGeneralizedIndex_mixin (shape : Desc) (step : PathStep)
    (supported : shape.mixesIn step = true) :
    getGeneralizedIndex shape [step] = .ok 3 := by
  -- Only length, active-field, and selector steps are supported by the matching type families.
  cases shape <;> cases step <;> simp only [Desc.mixesIn, Bool.false_eq_true] at supported
  all_goals with_unfolding_all rfl

/-- A reserved type path and the value walker agree on the mixed-in node. -/
theorem reserved_path_reads {shape : Desc} {value : Value} {layout : MerkleLayout}
    {step : PathStep} {word : Bytes} (supported : shape.mixesIn step = true)
    (laid : merkleLayout shape value = .ok layout) (mixed : layout.mixin = some word) :
    getGeneralizedIndex shape [step] = .ok 3 ∧ nodeRoot shape value 3 = .ok word := by
  -- The path names position three, and the executable walker returns exactly the layout's word.
  exact ⟨getGeneralizedIndex_mixin shape step supported, nodeRoot_mixin laid mixed⟩

/-- Rebasing a generalized index at the root leaves the index unchanged. -/
theorem gindexConcat_one {index : Nat} (named : 1 ≤ index) :
    gindexConcat 1 index = .ok index := by
  -- Removing and restoring the leading bit preserves all branch turns.
  have lower := (gindexDepth_bounds named).1
  simp [gindexConcat, gindexDepth, Nat.not_lt.mpr named, Bind.bind, Except.bind,
    Pure.pure, Except.pure, Nat.add_sub_cancel' lower]

/-- A one-step container path names the leaf at its field's declared ordinal. -/
theorem getGeneralizedIndex_container_field (names : List String) (fields : List Desc)
    (ordinal : Nat) (child : Desc) (selected : fields[ordinal]? = some child) :
    getGeneralizedIndex (.container names fields) [.position ordinal] =
      .ok (nextPow2 fields.length + ordinal) := by
  -- Containers allocate one Merkle leaf per field in declaration order.
  have positive : 1 ≤ nextPow2 fields.length + ordinal := by
    have width := Nat.two_pow_pos (depthFor fields.length)
    unfold nextPow2
    omega
  simp [getGeneralizedIndex, Desc.resolveStep, Desc.chunkPosition, Desc.elementType, selected, Desc.chunkCount,
    Bind.bind, Except.bind, Pure.pure, Except.pure,
    gindexConcat_root_right positive]

/-- A field's generalized index has the bounded tree's leaf depth. -/
theorem bounded_leaf_depth {capacity ordinal : Nat} (selected : ordinal < capacity) :
    Nat.log2 (nextPow2 capacity + ordinal) = depthFor capacity := by
  -- Every leaf lies between the leading width bit and the next power of two.
  have room := le_two_pow_depthFor capacity
  have positive := Nat.two_pow_pos (depthFor capacity)
  apply (Nat.log2_eq_iff (by unfold nextPow2; omega)).mpr
  simp only [nextPow2, Nat.pow_succ]
  constructor <;> omega

/-- The low turns of a field's index recover its ordinal. -/
theorem bounded_leaf_position {capacity ordinal : Nat} (selected : ordinal < capacity) :
    gindexBelow (nextPow2 capacity + ordinal) (depthFor capacity) = ordinal := by
  -- The leading width bit is removed by reduction modulo that width.
  have room := le_two_pow_depthFor capacity
  simp only [gindexBelow, nextPow2, Nat.add_mod, Nat.mod_self, Nat.zero_add]
  rw [Nat.mod_mod, Nat.mod_eq_of_lt (by omega : ordinal < 2 ^ depthFor capacity)]

/-- A container field path reads the root of exactly the corresponding field value. -/
theorem container_field_path_reads (names : List String) (fields : List Desc) (values : List Value)
    (count : fields.length = values.length) (chunks : Array Bytes)
    (materialized : layoutChunksAt ((Desc.container names fields).nesting - 1)
      (.nesting ((fields.zip values).map some) (some fields.length)) = .ok chunks)
    (ordinal : Nat) (child : Desc) (inner : Value)
    (selectedType : fields[ordinal]? = some child) (selectedValue : values[ordinal]? = some inner) :
    getGeneralizedIndex (.container names fields) [.position ordinal] =
      .ok (nextPow2 fields.length + ordinal) ∧
    nodeRoot (.container names fields) (.seq values) (nextPow2 fields.length + ordinal) =
      hashTreeRoot child inner := by
  -- The type path and layout use the same ordinal in the same paired field/value lists.
  let layout := MerkleLayout.nesting ((fields.zip values).map some) (some fields.length)
  have laid : merkleLayout (.container names fields) (.seq values) = .ok layout := by
    simp [merkleLayout, count, layout, Pure.pure, Except.pure]
  have inside : ordinal < fields.length := (List.getElem?_eq_some_iff.mp selectedType).1
  have depth := bounded_leaf_depth inside
  have position := bounded_leaf_position inside
  have indexPositive : 1 ≤ nextPow2 fields.length + ordinal := by
    have := Nat.two_pow_pos (depthFor fields.length)
    unfold nextPow2
    omega
  -- The type lookup and value lookup share an ordinal, so their zipped entry is the selected child.
  have slots : ((fields.zip values).map some)[ordinal]? = some (some (child, inner)) := by
    have zipped : (fields.zip values)[ordinal]? = some (child, inner) :=
      List.getElem?_zip_eq_some.mpr ⟨selectedType, selectedValue⟩
    simp [List.getElem?_map, zipped]
  -- The enclosing declaration reserves a full recursion budget for every selected field type.
  have enough : child.nesting ≤ (Desc.container names fields).nesting - 1 := by
    have smaller := merkleLayout_child_nesting _ _ layout laid _ rfl child inner (List.mem_of_getElem? slots)
    omega
  have size := layoutChunksAt_size _ layout chunks materialized
  have fits : chunks.size ≤ fields.length := by
    simpa [layout, MerkleLayout.nesting, Leaves.count, List.length_zip, count] using Nat.le_of_eq size
  constructor
  · exact getGeneralizedIndex_container_field names fields ordinal child selectedType
  · have outer := nodeRoot_bounded_layout laid materialized rfl rfl fits
      (nextPow2 fields.length + ordinal) indexPositive (by omega)
    have interval := boundedNode_window (layoutChunksAt_window _ layout chunks materialized)
      (nextPow2 fields.length + ordinal) (depthFor fields.length) 0 fields.length (Nat.le_refl _)
    have nested := boundedNode_nested_leaf layout _ _ rfl child inner enough
      (nextPow2 fields.length + ordinal) 0 fields.length (by simpa [position] using slots)
    rw [outer, ← nested, interval]
    simp only [boundedReading, depth]

/-- A bounded leaf reads the corresponding materialized chunk, including zero padding. -/
theorem nodeRoot_bounded_leaf {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {capacity ordinal : Nat}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (bounded : layout.limit = some capacity) (unmixed : layout.mixin = none)
    (fits : chunks.size ≤ capacity) (inside : ordinal < capacity) :
    nodeRoot shape value (nextPow2 capacity + ordinal) = .ok (padded zeroChunk chunks ordinal) := by
  -- At leaf depth the selected interval has one node, so no parent hash is added.
  have positive : 1 ≤ nextPow2 capacity + ordinal := by
    have := Nat.two_pow_pos (depthFor capacity)
    unfold nextPow2
    omega
  rw [nodeRoot_bounded_layout laid materialized bounded unmixed fits _ positive
    (by simp only [bounded_leaf_depth inside, Nat.le_refl])]
  simp [boundedReading, boundedTreeNode, bounded_leaf_depth inside,
    bounded_leaf_position inside, subtreeAt_eq_subtreeRoot, subtreeRoot]

/-- A bounded contents leaf adds one leading left turn before its ordinal. -/
theorem mixed_leaf_index {capacity ordinal : Nat} (inside : ordinal < capacity) :
    Nat.log2 (2 * nextPow2 capacity + ordinal) = depthFor capacity + 1 ∧
    gindexBit (2 * nextPow2 capacity + ordinal) (depthFor capacity) = false ∧
    gindexBelow (2 * nextPow2 capacity + ordinal) (depthFor capacity) = ordinal := by
  -- The quotient by the leaf width is two, whose low bit is the contents turn.
  have room := le_two_pow_depthFor capacity
  have power := Nat.two_pow_pos (depthFor capacity)
  have small : ordinal < 2 ^ depthFor capacity := by omega
  -- The high quotient is binary ten, recording the root bit followed by the contents’ left turn.
  have quotient : (2 * nextPow2 capacity + ordinal) / 2 ^ depthFor capacity = 2 := by
    rw [nextPow2, Nat.add_div (Nat.two_pow_pos _), Nat.mul_div_left _ power,
      Nat.div_eq_of_lt small]
    simp [Nat.mod_eq_of_lt small, small]
  refine ⟨?_, ?_, ?_⟩
  · apply (Nat.log2_eq_iff (by unfold nextPow2; omega)).mpr
    simp only [nextPow2, Nat.pow_succ]
    constructor <;> omega
  · simp [gindexBit, Nat.testBit_eq_decide_div_mod_eq, quotient]
  · simp [gindexBelow, nextPow2, Nat.add_mod, Nat.mod_eq_of_lt small]

/-- A bounded mixed leaf reads the same packed chunk beneath its length or selector. -/
theorem nodeRoot_mixed_leaf {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {capacity ordinal : Nat} {word : Bytes}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (bounded : layout.limit = some capacity) (mixed : layout.mixin = some word)
    (fits : chunks.size ≤ capacity) (inside : ordinal < capacity) :
    nodeRoot shape value (2 * nextPow2 capacity + ordinal) =
      .ok (padded zeroChunk chunks ordinal) := by
  -- The right-hand word does not change which chunk a left-hand leaf contains.
  obtain ⟨depth, turn, position⟩ := mixed_leaf_index inside
  have positive : 2 ≤ 2 * nextPow2 capacity + ordinal := by
    have := Nat.two_pow_pos (depthFor capacity)
    unfold nextPow2
    omega
  -- A left-content leaf cannot be the right-hand mixing word at generalized index three.
  have notWord : 2 * nextPow2 capacity + ordinal ≠ 3 := by
    intro same
    have depthThree : Nat.log2 3 = 1 := rfl
    rw [same, depthThree] at depth
    have zero : depthFor capacity = 0 := by omega
    rw [same, zero] at turn
    contradiction
  rw [nodeRoot_mixed_layout laid materialized bounded mixed fits _
    (Or.inr (Or.inr ⟨positive, by simpa [depth] using turn, by simp [depth]⟩))]
  simp [mixedReading, show 2 * nextPow2 capacity + ordinal ≠ 1 by omega, notWord,
    boundedTreeNode, depth, position, subtreeAt_eq_subtreeRoot, subtreeRoot]

/-- An occupied progressive chunk reads exactly its materialized leaf, without an extra hash. -/
theorem nodeRoot_progressive_leaf {shape : Desc} {value : Value} {layout : MerkleLayout}
    {chunks : Array Bytes} {word : Bytes} {ordinal : Nat}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutChunksAt (shape.nesting - 1) layout = .ok chunks)
    (progressive : layout.limit = none) (mixed : layout.mixin = some word)
    (inside : ordinal < chunks.size) :
    nodeRoot shape value (progressiveChunkGindex ordinal) =
      .ok (padded zeroChunk chunks ordinal) := by
  -- The spine reaches an occupied segment, then the bounded walk ends at its selected leaf.
  have size := layoutChunksAt_size _ layout chunks materialized
  have present : ordinal < layout.leaves.count := by omega
  obtain ⟨stop, named, turn, walked, left, capacity, position, _⟩ :=
    progressiveChunkGindex_selection ordinal layout.leaves.count present
  -- The chosen progressive segment has a power-of-two capacity, giving its exact bounded-tree height.
  have height : depthFor stop.capacity = stop.depth := by
    rw [capacity, depthFor_pow]
  have depthPositive : Nat.log2 (progressiveChunkGindex ordinal) ≠ 0 := by
    have recursion := levelOf_parent named
    change Nat.log2 (progressiveChunkGindex ordinal) = _ at recursion
    omega
  have nesting := shape.nesting_pos
  rw [← nodeRootAt_eq_nodeRoot shape value _ shape.nesting (Nat.le_refl _)]
  cases depth : shape.nesting with
  | zero => omega
  | succ budget =>
    simp only [depth, Nat.add_sub_cancel] at materialized
    simp [nodeRootAt, show ¬progressiveChunkGindex ordinal < 1 by omega,
      show progressiveChunkGindex ordinal ≠ 1 by omega, laid, mixed, progressive,
      gindexLength, gindexDepth, depthPositive, turn, progressiveNode, walked, left,
      Bind.bind, Except.bind]
    rw [boundedNode_window (layoutChunksAt_window budget layout chunks materialized)
      _ stop.depth stop.leavesFrom stop.capacity (by omega)]
    simp [boundedTreeNode, height, position, subtreeAt_eq_subtreeRoot, subtreeRoot]

end Ssz
