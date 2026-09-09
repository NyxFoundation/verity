import Ssz.Codec.Admits
import Ssz.Type.DefaultLaws

/-! The executable value checker reflects declarative admissibility. -/

namespace Ssz

/-- A homogeneous sequence passes exactly when each element passes its own check. -/
theorem Value.allFit_eq_true (element : Desc) (values : List Value) :
    Value.allFit element values = true ↔ ∀ value ∈ values, Value.fits element value = true := by
  -- Each step checks the first element and then the remaining sequence.
  induction values with
  | nil => simp [Value.allFit]
  | cons value rest ih => simp [Value.allFit, ih]

/-- Field checking requires equal counts and a successful check for every paired field. -/
theorem Value.fieldsFit_eq_true (fields : List Desc) (values : List Value) :
    Value.fieldsFit fields values = true ↔ fields.length = values.length ∧
      ∀ pair ∈ fields.zip values, Value.fits pair.1 pair.2 = true := by
  -- Both lists must end together, so pairing cannot silently discard a field or value.
  induction fields generalizing values with
  | nil => cases values <;> simp [Value.fieldsFit]
  | cons field fields ih =>
    cases values <;> simp [Value.fieldsFit, ih, and_left_comm]

/-- Union checking follows exactly the option selected by the shared selector lookup. -/
theorem Value.optionFits_eq_true (selectors : List Nat) (options : List Desc)
    (selector : Nat) (data : Value) :
    Value.optionFits selectors options selector data = true ↔
      ∃ option, lookupOption selectors options selector = .ok option ∧ Value.fits option data = true := by
  -- An unmatched selector advances through both declaration lists together.
  induction selectors generalizing options with
  | nil => cases options <;> simp [Value.optionFits, lookupOption]
  | cons chosen selectors ih =>
    cases options with
    | nil => simp [Value.optionFits, lookupOption]
    | cons option options =>
      -- The first matching selector determines the payload type.
      -- Later options cannot override it.
      by_cases matched : chosen = selector
      · simp [Value.optionFits, lookupOption, matched]
      · simp [Value.optionFits, lookupOption, matched, ih]

/-- Every declaratively admitted value passes the executable checker. -/
theorem Fits.checked {shape : Desc} {value : Value} (fitted : Fits shape value) :
    Value.fits shape value = true := by
  -- Induction follows the admitted value, including each nested field and element.
  induction fitted with
  | bool => simp [Value.fits]
  | uint bound => simpa [Value.fits] using bound
  | byteVector exact => simpa [Value.fits] using exact
  | byteList within => simpa [Value.fits] using within
  | bitVector exact => simpa [Value.fits] using exact
  | bitList within => simpa [Value.fits] using within
  | progressiveBitList => simp [Value.fits]
  | vector count each ih => simpa [Value.fits, count, Value.allFit_eq_true] using ih
  | list within each ih => simpa [Value.fits, within, Value.allFit_eq_true] using ih
  | progressiveList each ih => simpa [Value.fits, Value.allFit_eq_true] using ih
  | container paired each ih => simpa [Value.fits, Value.fieldsFit_eq_true, paired] using ih
  | progressiveContainer paired each ih => simpa [Value.fits, Value.fieldsFit_eq_true, paired] using ih
  | compatibleUnion bounded named inner ih =>
    simpa only [Value.fits] using (Value.optionFits_eq_true _ _ _ _).mpr ⟨_, named, ih⟩

/-- For a real declaration, a successful executable check establishes admissibility. -/
theorem fits_of_checked (shape : Desc) (value : Value) (sound : shape.wellFormed = .ok ())
    (checked : Value.fits shape value = true) : Fits shape value := by
  -- Validity descends with the declaration, supplying each selected union option's bounds.
  induction shape using Desc.rec
    (motive_2 := fun fields => ∀ field ∈ fields, ∀ value,
      field.wellFormed = .ok () → Value.fits field value = true → Fits field value)
    generalizing value with
  | bool => cases value <;> simp [Value.fits] at checked; exact .bool _
  | uint width => cases value <;> simp [Value.fits] at checked; exact .uint checked
  | byteVector length => cases value <;> simp [Value.fits] at checked; exact .byteVector checked
  | byteList limit => cases value <;> simp [Value.fits] at checked; exact .byteList checked
  | bitVector length => cases value <;> simp [Value.fits] at checked; exact .bitVector checked
  | bitList limit => cases value <;> simp [Value.fits] at checked; exact .bitList checked
  | progressiveBitList => cases value <;> simp [Value.fits] at checked; exact .progressiveBitList
  -- Fixed and bounded sequences combine a count condition with every child’s admissibility.
  | vector element length ih =>
    cases value <;> simp [Value.fits, Value.allFit_eq_true] at checked
    exact .vector checked.1 (fun held member =>
      ih held (wellFormed_vector sound) (checked.2 held member))
  | list element limit ih =>
    cases value <;> simp [Value.fits, Value.allFit_eq_true] at checked
    exact .list checked.1 (fun held member =>
      ih held (wellFormed_list sound) (checked.2 held member))
  | progressiveList element ih =>
    cases value <;> simp [Value.fits, Value.allFit_eq_true] at checked
    exact .progressiveList (fun held member =>
      ih held (wellFormed_progressiveList sound) (checked held member))
  -- A paired field is both declared and successfully checked at the same ordinal.
  | container names fields ih =>
    cases value <;> simp [Value.fits, Value.fieldsFit_eq_true] at checked
    refine .container checked.1 ?_
    intro pair member
    have field := (List.of_mem_zip member).1
    exact ih pair.1 field pair.2
      (allWellFormed_mem fields (wellFormed_container sound) pair.1 field) (checked.2 pair.1 pair.2 member)
  -- Inactive tree positions do not add values to the field sequence.
  | progressiveContainer active names fields ih =>
    cases value <;> simp [Value.fits, Value.fieldsFit_eq_true] at checked
    refine .progressiveContainer checked.1 ?_
    intro pair member
    have field := (List.of_mem_zip member).1
    exact ih pair.1 field pair.2
      (allWellFormed_mem fields (wellFormed_progressiveContainer sound) pair.1 field) (checked.2 pair.1 pair.2 member)
  -- A valid declaration bounds the selected tag and validates its chosen option type.
  | compatibleUnion selectors options ih =>
    cases value <;> simp [Value.fits, Value.optionFits_eq_true] at checked
    obtain ⟨option, named, inner⟩ := checked
    -- Successful selector lookup places the chosen type among the validated options.
    have member := lookupOption_mem selectors options _ option named
    exact .compatibleUnion
      (wellFormed_union_selectors sound _ (lookupOption_selector selectors options _ option named))
      named (ih option member _ (allWellFormed_mem options (wellFormed_union sound) option member) inner)
  | nil => intros; contradiction
  | cons field fields ihHead ihTail =>
    rename_i held member value sound checked
    -- Membership selects either the current field’s proof or the remaining field-list proof.
    rcases List.mem_cons.mp member with here | later
    · subst held; exact ihHead value sound checked
    · exact ihTail held later value sound checked

/--
On well-formed declarations, executable checking is equivalent to declarative admissibility.

Declaration validity supplies the selector bound required by the union's one-byte encoding.
-/
theorem Value.fits_eq_true {shape : Desc} {value : Value} (sound : shape.wellFormed = .ok ()) :
    Value.fits shape value = true ↔ Fits shape value :=
  ⟨fits_of_checked shape value sound, Fits.checked⟩

/-- Every successfully constructed default passes the executable value checker. -/
theorem default_checked {shape : Desc} {value : Value} (made : shape.default = .ok value) :
    Value.fits shape value = true :=
  (default_fits shape value made).checked

end Ssz
