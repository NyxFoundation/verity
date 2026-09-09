import Ssz.Codec.Fits
import Ssz.Type.Valid

/-! Every value a real type encodes is a value that type admits. -/

namespace Ssz

/-- Throwing a codec error returns that error without a successful result. -/
private theorem throw_error {α : Type} (fault : Err) :
    (throw fault : Except Err α) = .error fault := rfl

/-- A chain of checks that all passed had to pass its first one. -/
theorem firstCheckPassed {alpha beta} {step : Except Err alpha}
    {rest : alpha → Except Err beta} {answer : beta} (chain : step >>= rest = .ok answer) :
    ∃ carried, step = .ok carried := by
  -- A refusal at the first check is carried out of the chain, so the chain refuses too.
  cases step with
  | error _ => simp [Bind.bind, Except.bind] at chain
  | ok carried => exact ⟨carried, rfl⟩

/-- What a passed first check leaves is the rest of the chain, which passed as well. -/
theorem laterChecksPassed {alpha beta} {step : Except Err alpha} {carried : alpha}
    {rest : alpha → Except Err beta} {answer : beta} (first : step = .ok carried)
    (chain : step >>= rest = .ok answer) : rest carried = .ok answer := by
  -- With the first check known to have passed, the chain is the rest of itself.
  subst first
  exact chain

/-- The option a selector names is one of the options the union declared. -/
theorem lookupOption_mem :
    ∀ (selectors : List Nat) (options : List Desc) (selector : Nat) (option : Desc),
      lookupOption selectors options selector = .ok option → option ∈ options := by
  intro selectors
  induction selectors with
  | nil => intro options _ _ found; cases options <;> simp [lookupOption] at found
  | cons _ rest ih =>
    intro options selector option found
    cases options with
    | nil => simp [lookupOption] at found
    | cons candidate others =>
      simp only [lookupOption] at found
      -- Either this step answered, or the walk moved along and the rest answered.
      split at found
      · exact Except.ok.inj found ▸ List.mem_cons_self
      · exact List.mem_cons_of_mem _ (ih others selector option found)

/-- A selector that names an option is one of the selectors the union declared. -/
theorem lookupOption_selector :
    ∀ (selectors : List Nat) (options : List Desc) (selector : Nat) (option : Desc),
      lookupOption selectors options selector = .ok option → selector ∈ selectors := by
  intro selectors
  induction selectors with
  | nil => intro options _ _ found; cases options <;> simp [lookupOption] at found
  | cons candidate rest ih =>
    intro options selector option found
    cases options with
    | nil => simp [lookupOption] at found
    | cons _ others =>
      simp only [lookupOption] at found
      -- Either this step answered, in which case its own selector is the one asked for.
      split at found
      · rename_i matched
        simp only [beq_iff_eq] at matched
        exact matched ▸ List.mem_cons_self
      · exact List.mem_cons_of_mem _ (ih others selector option found)

/-- Every type in a list that names real types names a real type itself. -/
theorem allWellFormed_mem : ∀ (fields : List Desc), Desc.allWellFormed fields = .ok () →
    ∀ shape ∈ fields, shape.wellFormed = .ok () := by
  intro fields
  induction fields with
  | nil =>
    intro _ shape member
    simp at member
  | cons field rest ih =>
    intro sound shape member
    simp only [Desc.allWellFormed] at sound
    -- The head names a real type, and what that leaves is the same claim about the rest.
    cases head : field.wellFormed with
    | error _ =>
      rw [head] at sound
      simp [Bind.bind, Except.bind] at sound
    | ok _ =>
      rw [head] at sound
      simp only [Bind.bind, Except.bind] at sound
      rcases List.mem_cons.mp member with here | later
      · exact here ▸ head
      · exact ih sound shape later

/-- A vector of a real element type declares a real type below it. -/
theorem wellFormed_vector {element : Desc} {length : Nat}
    (sound : (Desc.vector element length).wellFormed = .ok ()) : element.wellFormed = .ok () := by
  -- A vector holds at least one element, and past that check only the element is left.
  rw [Desc.wellFormed] at sound
  split at sound
  · simp [Bind.bind, Except.bind] at sound
  · exact sound

/-- A list declares the type of the elements it holds, and nothing else. -/
theorem wellFormed_list {element : Desc} {limit : Nat}
    (sound : (Desc.list element limit).wellFormed = .ok ()) : element.wellFormed = .ok () := by
  rw [Desc.wellFormed] at sound
  exact sound

/-- A progressive list declares the type of the elements it holds, and nothing else. -/
theorem wellFormed_progressiveList {element : Desc}
    (sound : (Desc.progressiveList element).wellFormed = .ok ()) :
    element.wellFormed = .ok () := by
  rw [Desc.wellFormed] at sound
  exact sound

/-- A struct of real types declares real types for its fields. -/
theorem wellFormed_container {names : List String} {fields : List Desc}
    (sound : (Desc.container names fields).wellFormed = .ok ()) :
    Desc.allWellFormed fields = .ok () := by
  -- Declaration checks precede the field checks, so an invalid field cannot be hidden.
  cases inner : Desc.allWellFormed fields with
  | ok _ => rfl
  | error _ =>
    exfalso
    rw [Desc.wellFormed] at sound
    simp only [inner, throw_error, Bind.bind, Except.bind] at sound
    repeat' split at sound
    all_goals simp at sound

/-- A progressive struct of real types declares real types for its fields. -/
theorem wellFormed_progressiveContainer {active : List Bool} {names : List String}
    {fields : List Desc}
    (sound : (Desc.progressiveContainer active names fields).wellFormed = .ok ()) :
    Desc.allWellFormed fields = .ok () := by
  -- Whichever checks the declaration passed, the fields are the last thing it reads.
  cases inner : Desc.allWellFormed fields with
  | ok _ => rfl
  | error _ =>
    exfalso
    rw [Desc.wellFormed] at sound
    simp only [inner, throw_error, Bind.bind, Except.bind] at sound
    repeat' split at sound
    all_goals simp at sound

/-- A union of real types declares real types for its options. -/
theorem wellFormed_union {selectors : List Nat} {options : List Desc}
    (sound : (Desc.compatibleUnion selectors options).wellFormed = .ok ()) :
    Desc.allWellFormed options = .ok () := by
  -- Whichever checks the declaration passed, the options are read before the pairing is.
  cases inner : Desc.allWellFormed options with
  | ok _ => rfl
  | error _ =>
    exfalso
    rw [Desc.wellFormed] at sound
    simp only [inner, throw_error, Bind.bind, Except.bind] at sound
    repeat' split at sound
    all_goals simp at sound

/-- A union declares every one of its selectors inside the one byte that carries it. -/
theorem wellFormed_union_selectors {selectors : List Nat} {options : List Desc}
    (sound : (Desc.compatibleUnion selectors options).wellFormed = .ok ()) :
    ∀ selector ∈ selectors, selector < 256 := by
  intro selector member
  -- A selector outside the reserved range would have been found, and the search found none.
  cases searched : selectors.find? (fun candidate =>
      decide (candidate < minSelector) || decide (candidate > maxSelector)) with
  | some _ =>
    exfalso
    rw [Desc.wellFormed] at sound
    simp only [searched, Bind.bind, Except.bind] at sound
    repeat' split at sound
    all_goals simp_all
  | none =>
    have inside := List.find?_eq_none.mp searched selector member
    simp only [Bool.or_eq_true, decide_eq_true_eq, not_or, Nat.not_lt] at inside
    have top : maxSelector = 127 := rfl
    omega

/-- The second step of a chain that passed also passed. -/
theorem secondCheckPassed {alpha beta gamma} {first : Except Err alpha} {second : Except Err beta}
    {rest : alpha → beta → Except Err gamma} {answer : gamma}
    (chain : (do let carried ← first; let held ← second; rest carried held) = .ok answer) :
    ∃ held, second = .ok held := by
  -- A refusal at either step is carried out of the chain, so the chain refuses too.
  cases first with
  | error _ => simp [Bind.bind, Except.bind] at chain
  | ok _ =>
    simp only [Bind.bind, Except.bind] at chain
    cases second with
    | error _ => simp at chain
    | ok held => exact ⟨held, rfl⟩

/-- The rest of a list that names real types names real types itself. -/
theorem allWellFormed_tail {field : Desc} {fields : List Desc}
    (sound : Desc.allWellFormed (field :: fields) = .ok ()) :
    Desc.allWellFormed fields = .ok () := by
  -- The head is read first, and what it leaves is the same claim about the rest.
  simp only [Desc.allWellFormed] at sound
  cases head : field.wellFormed with
  | error _ =>
    rw [head] at sound
    simp [Bind.bind, Except.bind] at sound
  | ok _ =>
    rw [head] at sound
    simpa [Bind.bind, Except.bind] using sound

/--
A value the encoder wrote is a value its type admits.

So nothing has to be established of a value before the theorems below apply to it.
Writing it at all is enough.
-/
theorem fits_of_serialize : ∀ (shape : Desc) (value : Value),
    shape.wellFormed = .ok () → (∃ bytes, serialize shape value = .ok bytes) →
      Fits shape value := by
  intro shape value
  -- Follow encoder execution, proving field pairing and element admissibility alongside the enclosing value.
  induction shape, value using serialize.induct
    (motive2 := fun fields values => Desc.allWellFormed fields = .ok () →
      (∃ bytes, serializeStruct fields values = .ok bytes) →
        fields.length = values.length ∧ ∀ pair ∈ fields.zip values, Fits pair.1 pair.2)
    (motive3 := fun fields values => Desc.allWellFormed fields = .ok () →
      (∃ parts, serializeFields fields values = .ok parts) →
        fields.length = values.length ∧ ∀ pair ∈ fields.zip values, Fits pair.1 pair.2)
    (motive4 := fun element elements => element.wellFormed = .ok () →
      (∃ bytes, serializeSequence element elements = .ok bytes) →
        ∀ held ∈ elements, Fits element held)
    (motive5 := fun element elements => element.wellFormed = .ok () →
      (∃ parts, serializeEach element elements = .ok parts) →
        ∀ held ∈ elements, Fits element held) with
  -- Either boolean value is admissible.
  | case1 b =>
    intro _ _
    exact .bool b
  -- The accepted integer branch supplies its exact representable range.
  | case2 _ _ bound =>
    intro _ _
    exact .uint bound
  -- An out-of-range integer branch cannot have produced bytes.
  | case3 _ _ bad =>
    intro _ ⟨_, wrote⟩
    rw [serialize, if_neg bad] at wrote
    simp at wrote
  -- The fixed byte-array branch supplies equality with the declared length.
  | case4 _ _ exact_ =>
    intro _ _
    exact .byteVector (by simpa using exact_)
  -- A wrong byte count cannot have produced a successful fixed-array encoding.
  | case5 _ _ bad =>
    intro _ ⟨_, wrote⟩
    rw [serialize, if_neg bad] at wrote
    simp at wrote
  -- The bounded byte-array branch supplies its capacity inequality.
  | case6 _ _ within =>
    intro _ _
    exact .byteList within
  -- An exceeded byte capacity contradicts successful encoding.
  | case7 _ _ bad =>
    intro _ ⟨_, wrote⟩
    rw [serialize, if_neg bad] at wrote
    simp at wrote
  -- A fixed bitfield supplies the exact declared bit count.
  | case8 _ _ exact_ =>
    intro _ _
    exact .bitVector (by simpa using exact_)
  -- A wrong bit count contradicts successful fixed-bitfield encoding.
  | case9 _ _ bad =>
    intro _ ⟨_, wrote⟩
    rw [serialize, if_neg bad] at wrote
    simp at wrote
  -- An accepted bit list supplies its capacity inequality.
  | case10 _ _ within =>
    intro _ _
    exact .bitList within
  -- An exceeded bit capacity contradicts successful encoding.
  | case11 _ _ bad =>
    intro _ ⟨_, wrote⟩
    rw [serialize, if_neg bad] at wrote
    simp at wrote
  -- Progressive bit lists impose no declared capacity.
  | case12 _ =>
    intro _ _
    exact .progressiveBitList
  -- A vector needs both the exact element count and admissibility of every element.
  | case13 _ _ _ count ih =>
    intro sound ⟨bytes, wrote⟩
    rw [serialize, if_pos count] at wrote
    exact .vector (by simpa using count) (ih (wellFormed_vector sound) ⟨bytes, wrote⟩)
  -- The vector encoder cannot succeed with a mismatched element count.
  | case14 _ _ _ bad =>
    intro _ ⟨_, wrote⟩
    rw [serialize, if_neg bad] at wrote
    simp at wrote
  -- A bounded list combines its accepted count with the element induction.
  | case15 _ _ _ within ih =>
    intro sound ⟨bytes, wrote⟩
    rw [serialize, if_pos within] at wrote
    exact .list within (ih (wellFormed_list sound) ⟨bytes, wrote⟩)
  -- The list encoder cannot succeed after exceeding its capacity.
  | case16 _ _ _ bad =>
    intro _ ⟨_, wrote⟩
    rw [serialize, if_neg bad] at wrote
    simp at wrote
  -- A progressive list inherits admissibility from all of its elements.
  | case17 _ _ ih =>
    intro sound ⟨bytes, wrote⟩
    rw [serialize] at wrote
    exact .progressiveList (ih (wellFormed_progressiveList sound) ⟨bytes, wrote⟩)
  -- Ordinary containers require one admissible value per field.
  | case18 _ _ _ ih =>
    intro sound ⟨bytes, wrote⟩
    rw [serialize] at wrote
    obtain ⟨paired, each⟩ := ih (wellFormed_container sound) ⟨bytes, wrote⟩
    exact .container paired each
  -- Progressive containers have the same field-value pairing requirement.
  | case19 _ _ _ _ ih =>
    intro sound ⟨bytes, wrote⟩
    rw [serialize] at wrote
    obtain ⟨paired, each⟩ := ih (wellFormed_progressiveContainer sound) ⟨bytes, wrote⟩
    exact .progressiveContainer paired each
  | case20 selectors options selector _ ih =>
    intro sound ⟨_, wrote⟩
    rw [serialize] at wrote
    -- The selector names one of the declared options, and that option was written too.
    obtain ⟨option, found⟩ := firstCheckPassed wrote
    have chosen := laterChecksPassed found wrote
    obtain ⟨body, inner⟩ := firstCheckPassed chosen
    have declared := allWellFormed_mem options (wellFormed_union sound) option
      (lookupOption_mem selectors options selector option found)
    exact .compatibleUnion
      (wellFormed_union_selectors sound selector
        (lookupOption_selector selectors options selector option found))
      found (ih option declared ⟨body, inner⟩)
  | case21 shape value _ _ _ _ _ _ _ _ _ _ _ _ =>
    intro _ ⟨_, wrote⟩
    cases shape <;> cases value <;> simp_all [serialize]
    -- Each pairing the reduction leaves standing is excluded by a hypothesis of its own.
    all_goals (rename_i notMatch; exact absurd rfl (notMatch _ _ _ _ rfl rfl rfl))
  | case22 _ _ ih =>
    intros
    rename_i sound made
    obtain ⟨_, wrote⟩ := made
    -- A struct writes its fields before it lays them out, so the fields were written.
    rw [serializeStruct] at wrote
    obtain ⟨parts, made⟩ := firstCheckPassed wrote
    exact ih sound ⟨parts, made⟩
  | case23 =>
    intros
    exact ⟨rfl, by simp⟩
  -- Successful field collection proves both the first field and the remaining paired fields.
  | case24 field fields value values ihHead ihTail =>
    intros
    rename_i sound made
    obtain ⟨_, wrote⟩ := made
    rw [serializeFields] at wrote
    obtain ⟨head, wroteHead⟩ := firstCheckPassed wrote
    obtain ⟨tail, wroteTail⟩ := secondCheckPassed wrote
    obtain ⟨paired, each⟩ := ihTail (allWellFormed_tail sound) ⟨tail, wroteTail⟩
    refine ⟨by simp [paired], ?_⟩
    intro pair member
    -- The pairs are this field with its own value, and then the pairs of the rest.
    rw [List.zip_cons_cons] at member
    rcases List.mem_cons.mp member with here | later
    · exact here ▸ ihHead (allWellFormed_mem (field :: fields) sound field List.mem_cons_self)
        ⟨head, wroteHead⟩
    · exact each pair later
  | case25 fields values _ _ =>
    intros
    rename_i made
    obtain ⟨_, wrote⟩ := made
    cases fields <;> cases values <;> simp_all [serializeFields]
    -- A field list and a value list of different lengths pair with nothing.
    all_goals (rename_i notMatch; exact absurd rfl (notMatch _ _ _ _ rfl rfl rfl))
  | case26 _ _ ih =>
    intros
    rename_i sound made held member
    obtain ⟨_, wrote⟩ := made
    -- A sequence writes its elements before it lays them out, so the elements were written.
    rw [serializeSequence] at wrote
    obtain ⟨parts, encoded⟩ := firstCheckPassed wrote
    exact ih sound ⟨parts, encoded⟩ held member
  | case27 _ =>
    intros
    rename_i member
    simp at member
  -- Every collected sequence value is either the first element or belongs to the successfully encoded suffix.
  | case28 _ _ _ ihHead ihTail =>
    intros
    rename_i sound made held member
    obtain ⟨_, wrote⟩ := made
    rw [serializeEach] at wrote
    obtain ⟨head, wroteHead⟩ := firstCheckPassed wrote
    obtain ⟨tail, wroteTail⟩ := secondCheckPassed wrote
    rcases List.mem_cons.mp member with here | later
    · exact here ▸ ihHead sound ⟨head, wroteHead⟩
    · exact ihTail sound ⟨tail, wroteTail⟩ held later

end Ssz
