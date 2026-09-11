import Ssz.Codec.Layout

/-! Progressive field ordinals and materialized layout positions preserve the same values. -/

namespace Ssz

/-- Selecting a field ordinal locates that same field in the layout, skipping every gap. -/
theorem placeSlots_position {active : List Bool} {fields : List (Desc × Value)}
    {slots : List (Option (Desc × Value))}
    (placed : placeSlots active fields = .ok slots) (ordinal : Nat) (field : Desc × Value)
    (selected : fields[ordinal]? = some field) :
    ∃ position, activePosition active ordinal = some position ∧
      slots[position]? = some (some field) := by
  -- A gap shifts the position, while an occupied slot also consumes one field ordinal.
  induction active generalizing fields slots ordinal with
  | nil =>
    cases fields <;> simp [placeSlots] at placed
    simp at selected
  | cons bit active ih =>
    cases bit with
    -- An inactive position shifts the final tree index without consuming the field ordinal.
    | false =>
      cases tail : placeSlots active fields with
      | error e => simp [placeSlots, tail, Bind.bind, Except.bind] at placed
      | ok rest =>
        simp [placeSlots, tail, Bind.bind, Except.bind, Pure.pure, Except.pure] at placed
        subst slots
        obtain ⟨position, found, same⟩ := ih tail ordinal selected
        exact ⟨position + 1, by simp [activePosition, found], by simpa using same⟩
    -- An active position consumes one field and one tree position together.
    | true =>
      cases fields with
      | nil => simp at selected
      | cons first fields =>
        cases tail : placeSlots active fields with
        | error e => simp [placeSlots, tail, Bind.bind, Except.bind] at placed
        | ok rest =>
          simp [placeSlots, tail, Bind.bind, Except.bind, Pure.pure, Except.pure] at placed
          subst slots
          cases ordinal with
          -- The requested first field is the occupied position currently being visited.
          | zero =>
            simp at selected
            subst field
            exact ⟨0, rfl, rfl⟩
          -- A later field is found in the suffix and shifted past this occupied position.
          | succ ordinal =>
            obtain ⟨position, found, same⟩ := ih tail ordinal (by simpa using selected)
            exact ⟨position + 1, by simp [activePosition, found], by simpa using same⟩

/-- A successful progressive field path reads the paired type and value at its active position. -/
theorem layoutSlots_position {active : List Bool} {fields : List Desc} {values : List Value}
    {slots : List (Option (Desc × Value))}
    (placed : layoutSlots active fields values = .ok slots)
    (ordinal : Nat) (child : Desc) (inner : Value)
    (selectedType : fields[ordinal]? = some child) (selectedValue : values[ordinal]? = some inner) :
    ∃ position, layoutPosition active ordinal = .ok position ∧
      slots[position]? = some (some (child, inner)) := by
  -- Pairing declarations and values preserves the ordinal before gaps are inserted.
  unfold layoutSlots at placed
  split at placed
  · contradiction
  · dsimp only at placed
    split at placed
    · contradiction
    · obtain ⟨position, found, same⟩ := placeSlots_position placed ordinal (child, inner)
        (List.getElem?_zip_eq_some.mpr ⟨selectedType, selectedValue⟩)
      exact ⟨position, by simp [layoutPosition, found], same⟩

end Ssz
