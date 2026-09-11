import Ssz.Codec.BindingLeafShape
import Ssz.Codec.BindingDomain
import Ssz.Codec.RootDomain
import Ssz.Codec.LengthBinding
import Ssz.Codec.LayoutMixin

/-! A type and its exact mixing word determine the alignment of its Merkle leaves. -/

namespace Ssz

private theorem each_basic_bytes {element : Desc} {values : List Value} {parts : List Bytes}
    (basic : element.isBasic = true) (each : ∀ value ∈ values, Fits element value)
    (wrote : serializeEach element values = .ok parts) :
    (parts.foldl (fun total part => total ++ part) (#[] : Bytes)).size =
      values.length * element.itemLength := by
  -- Basic elements have a fixed byte width, so the packed stream length is element count times width.
  obtain ⟨expected, generated, sized⟩ := serializeEach_basic_size basic values each
  -- Deterministic serialization identifies the given parts with the parts of the size theorem.
  rw [wrote] at generated
  cases Except.ok.inj generated
  exact sized

/-- Equal-length sequences under the same element type have aligned leaf shapes. -/
theorem sequenceLayout_shapes {element : Desc} {left right : List Value}
    {positions : Option Nat} {leftMixin rightMixin : Option Bytes}
    {first second : MerkleLayout}
    (lengths : left.length = right.length)
    (leftFits : ∀ value ∈ left, Fits element value)
    (rightFits : ∀ value ∈ right, Fits element value)
    (laid : sequenceLayout element left positions leftMixin = .ok first)
    (otherLaid : sequenceLayout element right positions rightMixin = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Composite elements contribute one typed position each, while basic elements share packed nodes.
  cases basic : element.isBasic with
  | false =>
    simp only [sequenceLayout, basic, Bool.false_eq_true, if_false, pure, Except.pure,
      Except.ok.injEq] at laid otherLaid
    subst first
    subst second
    -- Equal element counts give the same repeated child-type sequence.
    simp [MerkleLayout.nesting, Leaves.shapes, List.map_map, Function.comp_def,
      List.map_const', lengths]
  | true =>
    cases a : serializeEach element left with
    | error fault => simp [sequenceLayout, basic, a, Bind.bind, Except.bind] at laid
    | ok parts =>
      cases b : serializeEach element right with
      | error fault => simp [sequenceLayout, basic, b, Bind.bind, Except.bind] at otherLaid
      | ok others =>
        simp only [sequenceLayout, basic, if_true, a, b, Bind.bind, Except.bind,
          pure, Except.pure, Except.ok.injEq] at laid otherLaid
        subst first
        subst second
        -- Equal element counts and byte widths produce equal packed-node counts and capacities.
        simp only [MerkleLayout.packing, Leaves.shapes, packElements, packBytes_size,
          each_basic_bytes basic leftFits a, each_basic_bytes basic rightFits b, lengths,
          and_self]

/-- Placing fields preserves their type projection independently of their values. -/
theorem placeSlots_shapes {active : List Bool} {left right : List (Desc × Value)}
    {first second : List (Option (Desc × Value))}
    (types : left.map Prod.fst = right.map Prod.fst)
    (placed : placeSlots active left = .ok first)
    (otherPlaced : placeSlots active right = .ok second) :
    first.map (Option.map Prod.fst) = second.map (Option.map Prod.fst) := by
  -- Use the common active-field mask to place both type sequences at the same positions.
  induction active generalizing left right first second with
  -- An exhausted mask requires that both field sequences are also exhausted.
  | nil =>
    cases left <;> simp [placeSlots] at placed
    cases right <;> simp [placeSlots] at otherPlaced
    subst first
    subst second
    rfl
  | cons bit active ih =>
    cases bit with
    -- An inactive position adds a gap to both layouts without consuming a field.
    | false =>
      cases a : placeSlots active left with
      | error fault => simp [placeSlots, a, Bind.bind, Except.bind] at placed
      | ok tail =>
        cases b : placeSlots active right with
        | error fault => simp [placeSlots, b, Bind.bind, Except.bind] at otherPlaced
        | ok rest =>
          simp only [placeSlots, a, b, Bind.bind, Except.bind, pure, Except.pure,
            Except.ok.injEq] at placed otherPlaced
          subst first
          subst second
          simp [ih types a b]
    -- An active position consumes one field from each sequence.
    | true =>
      cases left with
      | nil => simp [placeSlots] at placed
      | cons l left =>
        cases right with
        | nil => simp [placeSlots] at otherPlaced
        | cons r right =>
          -- The consumed fields have equal types, and their remaining type sequences still agree.
          obtain ⟨head, tails⟩ := List.cons.inj types
          cases a : placeSlots active left with
          | error fault => simp [placeSlots, a, Bind.bind, Except.bind] at placed
          | ok tail =>
            cases b : placeSlots active right with
            | error fault => simp [placeSlots, b, Bind.bind, Except.bind] at otherPlaced
            | ok rest =>
              simp only [placeSlots, a, b, Bind.bind, Except.bind, pure, Except.pure,
                Except.ok.injEq] at placed otherPlaced
              subst first
              subst second
              simp [head, ih tails a b]

private theorem merkleLayout_mixed {shape : Desc} {value : Value} {layout : MerkleLayout}
    (laid : merkleLayout shape value = .ok layout) :
    layout.mixin.isSome = (match shape with
      | .bool | .uint _ | .byteVector _ | .bitVector _ | .vector _ _ | .container _ _ => false
      | _ => true) := by
  -- Fixed-size leaves, vectors, and ordinary containers omit metadata mixing.
  -- Variable-size collections, progressive containers, and unions always include a word.
  cases shape <;> cases value <;>
    simp only [merkleLayout, fixedLeaf, sequenceLayout, Bind.bind, Except.bind,
      Pure.pure, Except.pure] at laid
  all_goals repeat first | split at laid | contradiction
  all_goals cases laid
  all_goals rfl

/-- The type alone determines whether a root includes a mixing word. -/
theorem merkleLayout_mixin_presence {shape : Desc} {left right : Value}
    {first second : MerkleLayout}
    (laid : merkleLayout shape left = .ok first)
    (otherLaid : merkleLayout shape right = .ok second) :
    first.mixin.isSome = second.mixin.isSome := by
  -- The common type makes the metadata-presence decision identically for both values.
  cases shape <;> exact (merkleLayout_mixed laid).trans (merkleLayout_mixed otherLaid).symm

private theorem fixedLeaf_shapes {shape : Desc} {left right : Value}
    {a b : Bytes} {first second : MerkleLayout}
    (leftBytes : serialize shape left = .ok a) (rightBytes : serialize shape right = .ok b)
    (sizes : a.size = b.size)
    (laid : fixedLeaf shape left = .ok first) (other : fixedLeaf shape right = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Equal encoded byte lengths round up to equal packed-node counts and hence equal capacities.
  simp only [fixedLeaf, leftBytes, rightBytes, Bind.bind, Except.bind, pure, Except.pure,
    Except.ok.injEq] at laid other
  subst first
  subst second
  simp [MerkleLayout.packing, Leaves.shapes, packBytes_size, sizes]

private theorem packedBits_size_eq {left right : Array Bool} (same : left.size = right.size) :
    (packedBits left).size = (packedBits right).size := by
  -- Packing first rounds the bit count to bytes and then rounds the byte count to whole nodes.
  simp [packedBits, packBytes_size, packBits, same]

private theorem zip_shapes {fields : List Desc} {left right : List Value}
    (first : fields.length = left.length) (second : fields.length = right.length) :
    (fields.zip left).map Prod.fst = (fields.zip right).map Prod.fst := by
  -- Pairing every field with one value preserves the complete field-type sequence.
  rw [List.map_fst_zip (by omega), List.map_fst_zip (by omega)]

private theorem bool_layout_shapes {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.bool) left) (rightFits : Fits (Desc.bool) right)
    (laid : merkleLayout (Desc.bool) left = .ok first)
    (other : merkleLayout (Desc.bool) right = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Either boolean occupies one encoded byte before zero padding.
  cases leftFits with
  | bool b =>
    cases rightFits with
    | bool c =>
      exact fixedLeaf_shapes (a := #[if b then 1 else 0]) (b := #[if c then 1 else 0])
        (by simp [serialize]) (by simp [serialize]) rfl laid other

private theorem uint_layout_shapes (width : Nat) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.uint width) left) (rightFits : Fits (Desc.uint width) right)
    (laid : merkleLayout (Desc.uint width) left = .ok first)
    (other : merkleLayout (Desc.uint width) right = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Every admissible integer uses exactly its declared byte width.
  cases leftFits with
  | @uint width n bound =>
    cases rightFits with
    | @uint _ m otherBound =>
      exact fixedLeaf_shapes (a := uintBytes width n) (b := uintBytes width m)
        (by simp [serialize, bound]) (by simp [serialize, otherBound])
        (by simp [uintBytes_size]) laid other

private theorem byteVector_layout_shapes (length : Nat) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.byteVector length) left) (rightFits : Fits (Desc.byteVector length) right)
    (laid : merkleLayout (Desc.byteVector length) left = .ok first)
    (other : merkleLayout (Desc.byteVector length) right = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Both payload lengths equal the fixed byte count declared by the type.
  cases leftFits with
  | byteVector count =>
    cases rightFits with
    | byteVector otherCount =>
      exact fixedLeaf_shapes (by simp [serialize, count]) (by simp [serialize, otherCount])
        (count.trans otherCount.symm) laid other

private theorem byteList_layout_shapes (limit : Nat) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.byteList limit) left) (rightFits : Fits (Desc.byteList limit) right)
    (leftSized : CommitmentSized (Desc.byteList limit) left) (rightSized : CommitmentSized (Desc.byteList limit) right)
    (laid : merkleLayout (Desc.byteList limit) left = .ok first)
    (other : merkleLayout (Desc.byteList limit) right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Exact 256-bit length words recover the byte counts needed to align packed nodes.
  cases leftFits with
  | byteList within =>
    cases rightFits with
    | byteList otherWithin =>
      cases leftSized with
      | byteList count =>
        cases rightSized with
        | byteList otherCount =>
          simp only [merkleLayout, Nat.not_lt.mpr within, Nat.not_lt.mpr otherWithin,
            if_false, pure, Except.pure, Except.ok.injEq] at laid other
          subst first
          subst second
          -- The count bounds exclude truncation when the equal 256-bit words are decoded.
          have sizes := lengthWord_injective count otherCount (Option.some.inj mixed)
          simp [MerkleLayout.packing, Leaves.shapes, packBytes_size, sizes]

private theorem bitVector_layout_shapes (length : Nat) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.bitVector length) left) (rightFits : Fits (Desc.bitVector length) right)
    (laid : merkleLayout (Desc.bitVector length) left = .ok first)
    (other : merkleLayout (Desc.bitVector length) right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Both bit counts equal the fixed count declared by the type.
  cases leftFits with
  | bitVector count =>
    cases rightFits with
    | bitVector otherCount =>
      simp only [merkleLayout, count, otherCount, bne_self_eq_false, Bool.false_eq_true,
        if_false, pure, Except.pure, Except.ok.injEq] at laid other
      subst first
      subst second
      exact ⟨rfl, congrArg Sum.inl (packedBits_size_eq (count.trans otherCount.symm))⟩

private theorem bitList_layout_shapes (limit : Nat) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.bitList limit) left) (rightFits : Fits (Desc.bitList limit) right)
    (leftSized : CommitmentSized (Desc.bitList limit) left) (rightSized : CommitmentSized (Desc.bitList limit) right)
    (laid : merkleLayout (Desc.bitList limit) left = .ok first)
    (other : merkleLayout (Desc.bitList limit) right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- The mixing word authenticates the bit count, including trailing zero bits.
  cases leftFits with
  | @bitList _ data within =>
    cases rightFits with
    | @bitList _ otherData otherWithin =>
      cases leftSized with
      | bitList count =>
        cases rightSized with
        | bitList otherCount =>
          simp only [merkleLayout, Nat.not_lt.mpr within, Nat.not_lt.mpr otherWithin,
            if_false, pure, Except.pure, Except.ok.injEq] at laid other
          subst first
          subst second
          -- The count bounds exclude truncation when the equal 256-bit words are decoded.
          have sizes := lengthWord_injective count otherCount (Option.some.inj mixed)
          simp [MerkleLayout.packing, Leaves.shapes, packedBits, packBytes_size, packBits, sizes]

private theorem progressiveBitList_layout_shapes {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.progressiveBitList) left) (rightFits : Fits (Desc.progressiveBitList) right)
    (leftSized : CommitmentSized (Desc.progressiveBitList) left) (rightSized : CommitmentSized (Desc.progressiveBitList) right)
    (laid : merkleLayout (Desc.progressiveBitList) left = .ok first)
    (other : merkleLayout (Desc.progressiveBitList) right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- An exact bit count fixes the progressive spine's stored node count even without a capacity.
  cases leftFits with
  | @progressiveBitList data =>
    cases rightFits with
    | @progressiveBitList otherData =>
      cases leftSized with
      | progressiveBitList count =>
        cases rightSized with
        | progressiveBitList otherCount =>
          cases laid
          cases other
          -- The count bounds exclude truncation when the equal 256-bit words are decoded.
          have sizes := lengthWord_injective count otherCount (Option.some.inj mixed)
          simp [MerkleLayout.packing, Leaves.shapes, packedBits, packBytes_size, packBits, sizes]

private theorem vector_layout_shapes (element : Desc) (length : Nat) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.vector element length) left) (rightFits : Fits (Desc.vector element length) right)
    (laid : merkleLayout (Desc.vector element length) left = .ok first)
    (other : merkleLayout (Desc.vector element length) right = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Both element counts equal the fixed count declared by the type.
  cases leftFits with
  | vector count each =>
    cases rightFits with
    | vector otherCount otherEach =>
      simp only [merkleLayout, count, otherCount, bne_self_eq_false, Bool.false_eq_true,
        if_false] at laid other
      exact sequenceLayout_shapes (count.trans otherCount.symm) each otherEach laid other

private theorem list_layout_shapes (element : Desc) (limit : Nat) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.list element limit) left) (rightFits : Fits (Desc.list element limit) right)
    (leftSized : CommitmentSized (Desc.list element limit) left) (rightSized : CommitmentSized (Desc.list element limit) right)
    (laid : merkleLayout (Desc.list element limit) left = .ok first)
    (other : merkleLayout (Desc.list element limit) right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- An exact element count determines the packing or typed positions beneath the list root.
  cases leftFits with
  | list within each =>
    cases rightFits with
    | list otherWithin otherEach =>
      simp only [merkleLayout, Nat.not_lt.mpr within, Nat.not_lt.mpr otherWithin, if_false] at laid other
      cases leftSized with
      | list count _ =>
        cases rightSized with
        | list otherCount _ =>
          -- The sequence layout keeps the supplied length word unchanged in its metadata.
          have words : lengthWord _ = lengthWord _ := Option.some.inj
            ((sequenceLayout_mixin laid).symm.trans (mixed.trans (sequenceLayout_mixin other)))
          -- Recover equal counts from the words before aligning packed or nested leaves.
          exact sequenceLayout_shapes (lengthWord_injective count otherCount words) each otherEach laid other

private theorem progressiveList_layout_shapes (element : Desc) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.progressiveList element) left) (rightFits : Fits (Desc.progressiveList element) right)
    (leftSized : CommitmentSized (Desc.progressiveList element) left) (rightSized : CommitmentSized (Desc.progressiveList element) right)
    (laid : merkleLayout (Desc.progressiveList element) left = .ok first)
    (other : merkleLayout (Desc.progressiveList element) right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- The mixing word aligns progressive element lists by their actual counts.
  cases leftFits with
  | progressiveList each =>
    cases rightFits with
    | progressiveList otherEach =>
      cases leftSized with
      | progressiveList count _ =>
        cases rightSized with
        | progressiveList otherCount _ =>
          -- The sequence layout keeps the supplied length word unchanged in its metadata.
          have words : lengthWord _ = lengthWord _ := Option.some.inj
            ((sequenceLayout_mixin laid).symm.trans (mixed.trans (sequenceLayout_mixin other)))
          -- Recover equal counts from the words before aligning packed or nested leaves.
          exact sequenceLayout_shapes (lengthWord_injective count otherCount words) each otherEach laid other

private theorem container_layout_shapes (names : List String) (fields : List Desc) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.container names fields) left) (rightFits : Fits (Desc.container names fields) right)
    (laid : merkleLayout (Desc.container names fields) left = .ok first)
    (other : merkleLayout (Desc.container names fields) right = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Ordinary containers retain exactly one position for every declared field.
  cases leftFits with
  | container paired _ =>
    cases rightFits with
    | container otherPaired _ =>
      simp only [merkleLayout, paired, bne_self_eq_false, Bool.false_eq_true,
        if_false, pure, Except.pure, Except.ok.injEq] at laid
      simp only [merkleLayout, otherPaired, bne_self_eq_false, Bool.false_eq_true,
        if_false, pure, Except.pure, Except.ok.injEq] at other
      subst first
      subst second
      -- Projecting paired fields back to their types removes the values without losing positions.
      exact ⟨congrArg some (paired.symm.trans otherPaired), congrArg Sum.inr (by
        simpa only [List.map_map, Function.comp_def, Option.map_some] using
          congrArg (List.map some) (zip_shapes paired otherPaired))⟩

private theorem progressiveContainer_layout_shapes (active : List Bool) (names : List String) (fields : List Desc) {left right : Value} {first second : MerkleLayout}
    (sound : (Desc.progressiveContainer active names fields).wellFormed = .ok ())
    (leftFits : Fits (Desc.progressiveContainer active names fields) left) (rightFits : Fits (Desc.progressiveContainer active names fields) right)
    (laid : merkleLayout (Desc.progressiveContainer active names fields) left = .ok first)
    (other : merkleLayout (Desc.progressiveContainer active names fields) right = .ok second) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- A well-formed active-field mask has exactly enough occupied positions for the declared fields.
  cases leftFits with
  | @progressiveContainer active names fields values paired _ =>
    cases rightFits with
    | @progressiveContainer _ _ _ others otherPaired _ =>
      -- Both values can be placed into the same active positions, including gaps between fields.
      obtain ⟨slots, placed⟩ := placeSlots_exists active (fields.zip values)
        (by simp [progressive_count sound, paired])
      obtain ⟨otherSlots, otherPlaced⟩ := placeSlots_exists active (fields.zip others)
        (by simp [progressive_count sound, otherPaired])
      simp [merkleLayout, layoutSlots, paired, progressive_count sound, placed,
        Bind.bind, Except.bind, pure, Except.pure] at laid
      simp [merkleLayout, layoutSlots, otherPaired, progressive_count sound, otherPlaced,
        Bind.bind, Except.bind, pure, Except.pure] at other
      subst first
      subst second
      -- The common mask and common field-type sequence determine identical nested leaf shapes.
      exact ⟨rfl, congrArg Sum.inr (placeSlots_shapes (zip_shapes paired otherPaired)
        placed otherPlaced)⟩

private theorem compatibleUnion_layout_shapes (selectors : List Nat) (options : List Desc) {left right : Value} {first second : MerkleLayout}
    (leftFits : Fits (Desc.compatibleUnion selectors options) left) (rightFits : Fits (Desc.compatibleUnion selectors options) right)
    (laid : merkleLayout (Desc.compatibleUnion selectors options) left = .ok first)
    (other : merkleLayout (Desc.compatibleUnion selectors options) right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- The authenticated selector identifies which child type occupies the union's sole position.
  cases leftFits with
  | @compatibleUnion selectors options selector option data bounded named _ =>
    cases rightFits with
    | @compatibleUnion _ _ otherSelector otherOption otherData otherBounded otherNamed _ =>
      simp [merkleLayout, named, selectorWord, show ¬ selector > 255 by omega,
        Bind.bind, Except.bind, pure, Except.pure] at laid
      simp [merkleLayout, otherNamed, selectorWord, show ¬ otherSelector > 255 by omega,
        Bind.bind, Except.bind, pure, Except.pure] at other
      subst first
      subst second
      -- A one-byte selector always fits in the 256-bit word, so equal words recover equal selectors.
      have selectorsEqual := lengthWord_injective
        (left := selector) (right := otherSelector) (by omega) (by omega) (Option.some.inj mixed)
      subst otherSelector
      -- The same selector lookup must return the same child type on both sides.
      rw [named] at otherNamed
      cases Except.ok.inj otherNamed
      exact ⟨rfl, rfl⟩

/--
A fixed type and equal mixing words align all packed or nested leaf positions.
Variable-size counts must fit in 256 bits so that equal words imply equal counts.
-/
theorem merkleLayout_shapes {shape : Desc} {left right : Value} {first second : MerkleLayout}
    (sound : shape.wellFormed = .ok ()) (leftFits : Fits shape left) (rightFits : Fits shape right)
    (leftSized : CommitmentSized shape left) (rightSized : CommitmentSized shape right)
    (laid : merkleLayout shape left = .ok first)
    (other : merkleLayout shape right = .ok second) (mixed : first.mixin = second.mixin) :
    first.limit = second.limit ∧ first.leaves.shapes = second.leaves.shapes := by
  -- Fixed counts come from the type, while variable counts and union selections come from exact mixing words.
  cases shape with
  | bool =>
    exact bool_layout_shapes leftFits rightFits laid other
  | uint width =>
    exact uint_layout_shapes width leftFits rightFits laid other
  | byteVector length =>
    exact byteVector_layout_shapes length leftFits rightFits laid other
  | byteList limit =>
    exact byteList_layout_shapes limit leftFits rightFits leftSized rightSized laid other mixed
  | bitVector length =>
    exact bitVector_layout_shapes length leftFits rightFits laid other mixed
  | bitList limit =>
    exact bitList_layout_shapes limit leftFits rightFits leftSized rightSized laid other mixed
  | progressiveBitList =>
    exact progressiveBitList_layout_shapes leftFits rightFits leftSized rightSized laid other mixed
  | vector element length =>
    exact vector_layout_shapes element length leftFits rightFits laid other
  | list element limit =>
    exact list_layout_shapes element limit leftFits rightFits leftSized rightSized laid other mixed
  | progressiveList element =>
    exact progressiveList_layout_shapes element leftFits rightFits leftSized rightSized laid other mixed
  | container names fields =>
    exact container_layout_shapes names fields leftFits rightFits laid other
  | progressiveContainer active names fields =>
    exact progressiveContainer_layout_shapes active names fields sound leftFits rightFits laid other
  | compatibleUnion selectors options =>
    exact compatibleUnion_layout_shapes selectors options leftFits rightFits laid other mixed

end Ssz
