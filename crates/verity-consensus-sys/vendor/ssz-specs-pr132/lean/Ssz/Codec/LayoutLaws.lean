import Ssz.Codec.Layout
import Ssz.Codec.Admits

/-! Nested leaves always carry types shallower than their enclosing type. -/

namespace Ssz

private theorem nesting_le_deepest (shape : Desc) (fields : List Desc)
    (member : shape ∈ fields) : shape.nesting ≤ Desc.deepestNesting fields := by
  -- The maximum over fields bounds each individual field depth.
  induction fields with
  | nil => simp at member
  | cons first rest ih =>
    simp only [List.mem_cons] at member
    rcases member with rfl | member
    · exact Nat.le_max_left _ _
    · exact Nat.le_trans (ih member) (Nat.le_max_right _ _)

private theorem sequenceLayout_depth (element : Desc) (values : List Value)
    (positions : Option Nat) (mixin : Option Bytes) (layout : MerkleLayout)
    (success : sequenceLayout element values positions mixin = .ok layout)
    (slots : List (Option (Desc × Value))) (nested : layout.leaves = .nested slots)
    (child : Desc) (value : Value) (member : some (child, value) ∈ slots) :
    child = element := by
  -- Basic elements are packed bytes, while each composite slot has the element type.
  unfold sequenceLayout at success
  split at success
  · cases h : serializeEach element values <;>
      simp [h, Bind.bind, Except.bind, Pure.pure, Except.pure] at success
    subst layout
    simp [MerkleLayout.packing] at nested
  · simp [Pure.pure, Except.pure] at success
    subst layout
    simp [MerkleLayout.nesting] at nested
    subst slots
    obtain ⟨_, _, eq⟩ := List.mem_map.mp member
    exact (Option.some.inj eq |> Prod.mk.inj).1.symm

/-- Every present nested leaf has strictly smaller type depth than its enclosing value. -/
theorem merkleLayout_child_nesting (shape : Desc) (value : Value) (layout : MerkleLayout)
    (success : merkleLayout shape value = .ok layout)
    (slots : List (Option (Desc × Value))) (nested : layout.leaves = .nested slots)
    (child : Desc) (inner : Value) (member : some (child, inner) ∈ slots) :
    child.nesting < shape.nesting := by
  -- Each layout constructor either packs bytes or selects declared child types.
  cases shape <;> cases value <;>
    simp [merkleLayout, fixedLeaf, Bind.bind, Except.bind, Pure.pure, Except.pure,
        throw, throwThe, MonadExceptOf.throw] at success
  all_goals try solve
    | split at success <;> simp only [Except.ok.injEq, reduceCtorEq] at success
      subst layout
      cases nested
    | subst layout
      cases nested
  -- Every nested vector position has the declared element type, one type level below the vector.
  case vector.seq element length elements =>
    split at success <;> try simp only [reduceCtorEq] at success
    have eq := sequenceLayout_depth element elements (some length) none layout
      success slots nested child inner member
    simp [eq, Desc.nesting]
  -- A list's count word adds no nested value type.
  case list.seq element limit elements =>
    split at success <;> try simp only [reduceCtorEq] at success
    have eq := sequenceLayout_depth element elements (some limit) (some (lengthWord elements.length))
      layout success slots nested child inner member
    simp [eq, Desc.nesting]
  -- Progressive tree depth does not increase the nesting depth of element types.
  case progressiveList.seq element elements =>
    have eq := sequenceLayout_depth element elements none (some (lengthWord elements.length))
      layout success slots nested child inner member
    simp [eq, Desc.nesting]
  case container.seq names fields elements =>
    -- Pairing preserves field membership, so the maximum declared depth bounds the child.
    split at success <;> try simp only [reduceCtorEq, Except.ok.injEq] at success
    subst layout
    simp [MerkleLayout.nesting] at nested
    subst slots
    have paired : (child, inner) ∈ fields.zip elements := by simpa using member
    have bounded := nesting_le_deepest child fields (List.of_mem_zip paired).1
    simpa [Desc.nesting] using Nat.lt_succ_of_le bounded
  case progressiveContainer.seq active names fields elements =>
    -- EIP-7495 gaps add positions but never introduce a new field type.
    cases hs : layoutSlots active fields elements with
    | error e => simp [hs] at success
    | ok placed =>
      simp [hs] at success
      subst layout
      simp [MerkleLayout.nesting] at nested
      subst slots
      unfold layoutSlots at hs
      split at hs <;> try simp only [reduceCtorEq, Bind.bind, Except.bind,
        throw, throwThe, MonadExceptOf.throw] at hs
      split at hs <;> try simp only [reduceCtorEq,
         ] at hs
      have fieldsPlaced := placeSlots_fields hs
      have paired : (child, inner) ∈ fields.zip elements := by
        rw [← fieldsPlaced]
        exact List.mem_filterMap.mpr ⟨some (child, inner), member, rfl⟩
      have bounded := nesting_le_deepest child fields (List.of_mem_zip paired).1
      simpa [Desc.nesting] using Nat.lt_succ_of_le bounded
  case compatibleUnion.union selectors options selector data =>
    -- EIP-8016 selectors choose an existing option without changing its type.
    cases hl : lookupOption selectors options selector with
    | error e => simp [hl] at success
    | ok chosen =>
      cases hw : selectorWord selector with
      | error e => simp [hl, hw] at success
      | ok word =>
        simp [hl, hw] at success
        subst layout
        simp [MerkleLayout.nesting] at nested
        subst slots
        simp at member
        rcases member with ⟨rfl, rfl⟩
        have bounded := nesting_le_deepest child options
          (lookupOption_mem selectors options selector child hl)
        simpa [Desc.nesting] using Nat.lt_succ_of_le bounded

end Ssz
