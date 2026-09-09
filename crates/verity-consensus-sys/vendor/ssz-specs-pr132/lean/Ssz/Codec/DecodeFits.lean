import Ssz.Codec.Admits
import Ssz.Codec.Deserialize

/-! Successful decoding establishes the value's declared bounds and shape. -/

namespace Ssz

private theorem throw_error {α : Type} (fault : Err) :
    (throw fault : Except Err α) = .error fault := rfl

/-- Reading a fixed number of bytes always yields an integer within that width. -/
theorem readUint_lt (data : Bytes) (start width : Nat) :
    readUint data start width < 2 ^ (8 * width) := by
  induction width generalizing start with
  | zero => simp [readUint]
  | succ width ih =>
    -- A byte contributes at most 255, and each remaining digit has weight 256.
    have low := UInt8.toNat_lt (data[start]?.getD 0)
    have tail := ih (start + 1)
    have power : 2 ^ (8 * (width + 1)) = 2 ^ (8 * width) * 256 := by
      rw [Nat.mul_succ, Nat.pow_add]
    simp only [readUint, power]
    omega

/-- Unpacking produces exactly the requested number of bits. -/
@[simp] theorem unpackBits_size (data : Bytes) (count : Nat) :
    (unpackBits data count).size = count := by
  simp [unpackBits]

/-- A successful offset table gives one span per body. -/
theorem offsetSpans_length {offsets spans : List Nat} {scope : Nat}
    (read : offsetSpans offsets scope = .ok spans) : spans.length = offsets.length := by
  -- Each accepted start offset contributes one span, including a zero-length span.
  induction offsets, scope using offsetSpans.induct generalizing spans with
  | case1 => cases read; rfl
  | case2 start scope bad => simp [offsetSpans, bad] at read
  | case3 start scope good =>
    simp [offsetSpans, good] at read
    subst spans
    rfl
  | case4 start next rest scope bad => simp [offsetSpans, bad] at read
  | case5 start next rest scope good ih =>
    -- A rejected suffix cannot occur inside a successfully decoded table.
    cases tail : offsetSpans (next :: rest) scope with
    | error fault => simp [offsetSpans, good, tail, Bind.bind, Except.bind] at read
    | ok more =>
      simp [offsetSpans, good, tail, Bind.bind, Except.bind, pure, Except.pure] at read
      subst spans
      -- Prepending the current span increases both counts by one.
      simp [ih tail]

/-- Cutting a vector produces one slice per declared element. -/
theorem vectorSlices_length {element : Desc} {count : Nat} {data : Bytes}
    {slices : List Bytes} (read : vectorSlices element count data = .ok slices) :
    slices.length = count := by
  -- The composite size restriction applies before either slicing strategy can succeed.
  by_cases huge : data.size ≥ 2 ^ (8 * bytesPerOffset)
  · cases fixed : element.fixedSize <;>
      simp [vectorSlices, huge, throw_error, Bind.bind, Except.bind] at read
  · simp only [vectorSlices, if_neg huge] at read
    cases fixed : element.fixedSize with
    -- Fixed-size slicing creates one equal-width interval for each declared element.
    | some width =>
      simp only [fixed] at read
      split at read
      · simp [Bind.bind, Except.bind] at read
      · simp [pure, Except.pure] at read
        subst slices
        simp
    -- Variable-size slicing pairs each of the declared offsets with exactly one body span.
    | none =>
      simp only [fixed] at read
      split at read
      · simp [Bind.bind, Except.bind] at read
      · split at read
        · simp only [beq_iff_eq] at *
          simp [pure, Except.pure] at read
          subst slices
          simp_all
        · split at read
          · simp [Bind.bind, Except.bind] at read
          · cases spans : offsetSpans (readOffsets data count) data.size with
            | error fault => simp [spans, Bind.bind, Except.bind] at read
            | ok widths =>
              simp [spans, Bind.bind, Except.bind, pure, Except.pure] at read
              subst slices
              -- Span validation preserves the number of offset entries.
              have sized := offsetSpans_length spans
              simp [readOffsets] at sized ⊢
              omega

/-- A delimited bit sequence respects the capacity checked by its decoder. -/
theorem unpackDelimited_bound {limit : Nat} {data : Bytes} {bits : Array Bool}
    (read : unpackDelimited (some limit) data = .ok bits) : bits.size ≤ limit := by
  -- Acceptance retains the comparison between the recovered bit count and the declared capacity.
  simp only [unpackDelimited] at read
  repeat' split at read
  all_goals simp_all [throw_error, Bind.bind, Except.bind, pure, Except.pure]
  -- The recovered array has exactly that bit count.
  all_goals subst bits; simp_all

/-- Cutting a bounded list never produces more slices than its declared capacity. -/
theorem listSlices_bound {element : Desc} {limit : Nat} {data : Bytes}
    {slices : List Bytes} (read : listSlices element (some limit) data = .ok slices) :
    slices.length ≤ limit := by
  -- For fixed elements the count comes from byte length, and for variable elements it comes from the first offset.
  simp [listSlices, throw_error, Bind.bind, Except.bind, pure, Except.pure] at read
  repeat' split at read
  all_goals simp_all
  -- Every surviving decoder path has checked that its recovered count fits the capacity.
  all_goals try (subst slices; simp_all [readOffsets])
  all_goals omega

/-- Decoding a sequence preserves its count and establishes each element's domain. -/
private theorem deserializeEach_fits {element : Desc}
    (inner : ∀ data value, deserialize element data = .ok value → Fits element value) :
    ∀ slices values, deserializeEach element slices = .ok values →
      values.length = slices.length ∧ ∀ value ∈ values, Fits element value := by
  intro slices
  -- One accepted slice produces one admissible value, preserving the sequence length.
  induction slices with
  | nil =>
    intro values read
    simp [deserializeEach] at read
    subst values
    simp
  | cons slice slices ih =>
    intro values read
    cases head : deserialize element slice with
    | error fault => simp [deserializeEach, head, Bind.bind, Except.bind] at read
    | ok value =>
      -- The remaining slices must decode successfully because errors propagate.
      cases tail : deserializeEach element slices with
      | error fault => simp [deserializeEach, head, tail, Bind.bind, Except.bind] at read
      | ok rest =>
        simp [deserializeEach, head, tail, Bind.bind, Except.bind, pure, Except.pure] at read
        subst values
        -- Combine admissibility of the first value with the inductive result for the suffix.
        obtain ⟨sized, each⟩ := ih rest tail
        exact ⟨by simp [sized], by simpa using And.intro (inner slice value head) each⟩

/-- Decoding fields establishes both one-to-one pairing and each field's domain. -/
private theorem deserializeFields_fits : ∀ fields slices values,
    (∀ field ∈ fields, ∀ data value, deserialize field data = .ok value → Fits field value) →
    deserializeFields fields slices = .ok values →
      fields.length = values.length ∧ ∀ pair ∈ fields.zip values, Fits pair.1 pair.2 := by
  intro fields
  -- Keep field declarations paired with their slices and resulting values.
  induction fields with
  | nil =>
    intro slices values _ read
    cases slices <;> simp [deserializeFields] at read
    subst values
    simp
  | cons field fields ih =>
    intro slices values inner read
    cases slices with
    | nil => simp [deserializeFields] at read
    | cons slice slices =>
      -- The current field uses its own type-specific decoder.
      cases head : deserialize field slice with
      | error fault => simp [deserializeFields, head, Bind.bind, Except.bind] at read
      | ok value =>
        cases tail : deserializeFields fields slices with
        | error fault => simp [deserializeFields, head, tail, Bind.bind, Except.bind] at read
        | ok rest =>
          simp [deserializeFields, head, tail, Bind.bind, Except.bind, pure, Except.pure] at read
          subst values
          -- The remaining fields provide their pairing and admissibility guarantees recursively.
          obtain ⟨sized, each⟩ := ih slices rest
            (fun held member => inner held (List.mem_cons_of_mem _ member)) tail
          refine ⟨by simp [sized], ?_⟩
          intro pair member
          -- Every field-value pair belongs either to the current position or to the proven suffix.
          rcases List.mem_cons.mp member with first | later
          · exact first ▸ inner field List.mem_cons_self slice value head
          · exact each pair later

/-- A decoded union names an actual option and a value admissible for that option. -/
private theorem deserializeOption_fits : ∀ selectors options selector data value,
    (∀ option ∈ options, ∀ data value, deserialize option data = .ok value → Fits option value) →
    deserializeOption selectors options selector data = .ok value →
    ∃ option held, value = .union selector held ∧
      lookupOption selectors options selector = .ok option ∧ Fits option held := by
  intro selectors
  -- Search selectors and option types together so the selected type remains identified.
  induction selectors with
  | nil =>
    intro options selector data value _ read
    cases options <;> simp [deserializeOption] at read
  | cons chosen selectors ih =>
    intro options selector data value inner read
    cases options with
    | nil => simp [deserializeOption] at read
    | cons option options =>
      -- A matching selector provides both a successful lookup and an admissible decoded payload.
      by_cases same : chosen == selector
      · cases body : deserialize option data with
        | error fault => simp [deserializeOption, same, body, Bind.bind, Except.bind] at read
        | ok held =>
          simp [deserializeOption, same, body, Bind.bind, Except.bind, pure, Except.pure] at read
          subst value
          exact ⟨option, held, rfl, by simp [lookupOption, same],
            inner option List.mem_cons_self data held body⟩
      -- Otherwise continue with the remaining options without changing the payload bytes.
      · obtain ⟨heldOption, held, eq, named, fits⟩ := ih options selector data value
          (fun held member => inner held (List.mem_cons_of_mem _ member))
          (by simpa [deserializeOption, same] using read)
        exact ⟨heldOption, held, eq, by simpa [lookupOption, same] using named, fits⟩

/-- Every decoded value belongs to the domain described by its type. -/
theorem fits_of_deserialize (shape : Desc) :
    ∀ data value, deserialize shape data = .ok value → Fits shape value := by
  -- Each decoder branch establishes the corresponding value-domain constructor.
  induction shape using Desc.rec
    (motive_2 := fun fields => ∀ field ∈ fields, ∀ data value,
      deserialize field data = .ok value → Fits field value) with
  | bool =>
    intro data value read
    simp only [deserialize, throw_error, Bind.bind, Except.bind] at read
    repeat' split at read
    all_goals simp_all [pure, Except.pure]
    all_goals subst value; exact .bool _
  -- Reading exactly the declared byte width automatically bounds the recovered integer.
  | uint width =>
    intro data value read
    simp only [deserialize, throw_error, Bind.bind, Except.bind] at read
    split at read
    · cases read
    · simp [pure, Except.pure] at read
      subst value
      exact .uint (readUint_lt data 0 width)
  -- The accepted byte count supplies the fixed-size requirement.
  | byteVector length =>
    intro data value read
    simp only [deserialize, throw_error, Bind.bind, Except.bind] at read
    split at read
    · cases read
    · simp [pure, Except.pure] at read
      subst value
      exact .byteVector (by simp_all)
  -- The decoder capacity comparison supplies the variable-size requirement.
  | byteList limit =>
    intro data value read
    simp only [deserialize, throw_error, Bind.bind, Except.bind] at read
    split at read
    · cases read
    · simp [pure, Except.pure] at read
      subst value
      exact .byteList (by omega)
  -- Unpacking the declared count creates exactly that many bits.
  | bitVector length =>
    intro data value read
    simp only [deserialize, throw_error, Bind.bind, Except.bind] at read
    repeat' split at read
    all_goals simp_all [pure, Except.pure]
    all_goals subst value; exact .bitVector (unpackBits_size data length)
  -- The delimiter determines the bit count, which must fit the declared capacity.
  | bitList limit =>
    intro data value read
    cases bits : unpackDelimited (some limit) data with
    | error fault => simp [deserialize, bits, Bind.bind, Except.bind] at read
    | ok held =>
      simp [deserialize, bits, Bind.bind, Except.bind, pure, Except.pure] at read
      subst value
      exact .bitList (unpackDelimited_bound bits)
  -- Progressive bit sequences require valid delimiter decoding but no declared capacity.
  | progressiveBitList =>
    intro data value read
    cases bits : unpackDelimited none data with
    | error fault => simp [deserialize, bits, Bind.bind, Except.bind] at read
    | ok held =>
      simp [deserialize, bits, Bind.bind, Except.bind, pure, Except.pure] at read
      subst value
      exact .progressiveBitList
  -- Slice count and element admissibility together establish the fixed sequence shape.
  | vector element count ih =>
    intro data value read
    cases parts : vectorSlices element count data with
    | error fault => simp [deserialize, parts, Bind.bind, Except.bind] at read
    | ok slices =>
      cases decoded : deserializeEach element slices with
      | error fault => simp [deserialize, parts, decoded, Bind.bind, Except.bind] at read
      | ok values =>
        simp [deserialize, parts, decoded, Bind.bind, Except.bind, pure, Except.pure] at read
        subst value
        obtain ⟨sized, each⟩ := deserializeEach_fits ih slices values decoded
        exact .vector (sized.trans (vectorSlices_length parts)) each
  -- The accepted slice count is bounded, and every recovered element fits its own type.
  | list element limit ih =>
    intro data value read
    cases parts : listSlices element (some limit) data with
    | error fault => simp [deserialize, parts, Bind.bind, Except.bind] at read
    | ok slices =>
      cases decoded : deserializeEach element slices with
      | error fault => simp [deserialize, parts, decoded, Bind.bind, Except.bind] at read
      | ok values =>
        simp [deserialize, parts, decoded, Bind.bind, Except.bind, pure, Except.pure] at read
        subst value
        obtain ⟨sized, each⟩ := deserializeEach_fits ih slices values decoded
        exact .list (sized ▸ listSlices_bound parts) each
  -- Without a fixed capacity, only the admissibility of each recovered element remains.
  | progressiveList element ih =>
    intro data value read
    cases parts : listSlices element none data with
    | error fault => simp [deserialize, parts, Bind.bind, Except.bind] at read
    | ok slices =>
      cases decoded : deserializeEach element slices with
      | error fault => simp [deserialize, parts, decoded, Bind.bind, Except.bind] at read
      | ok values =>
        simp [deserialize, parts, decoded, Bind.bind, Except.bind, pure, Except.pure] at read
        subst value
        exact .progressiveList (deserializeEach_fits ih slices values decoded).2
  -- Both container families decode one admissible value per declared field.
  | container names fields ih | progressiveContainer active names fields ih =>
    intro data value read
    cases parts : structSlices fields data with
    | error fault => simp [deserialize, parts, Bind.bind, Except.bind] at read
    | ok slices =>
      cases decoded : deserializeFields fields slices with
      | error fault => simp [deserialize, parts, decoded, Bind.bind, Except.bind] at read
      | ok values =>
        simp [deserialize, parts, decoded, Bind.bind, Except.bind, pure, Except.pure] at read
        subst value
        obtain ⟨sized, each⟩ := deserializeFields_fits fields slices values ih decoded
        first | exact .container sized each | exact .progressiveContainer sized each
  -- The first byte supplies a bounded selector, and the matching option supplies payload admissibility.
  | compatibleUnion selectors options ih =>
    intro data value read
    simp only [deserialize, throw_error, Bind.bind, Except.bind] at read
    split at read
    · cases read
    · obtain ⟨option, held, eq, named, fits⟩ :=
        deserializeOption_fits selectors options _ _ value ih read
      subst value
      exact .compatibleUnion (UInt8.toNat_lt _) named fits
  | nil => rename_i held member data value read; simp at member
  | cons field fields ihHead ihTail =>
    rename_i held member data value read
    rcases List.mem_cons.mp member with first | later
    · subst held
      exact ihHead data value read
    · exact ihTail held later data value read

/-- A container budget outside the offset range is rejected before any field is read. -/
theorem structSlices_overflow (fields : List Desc) (data : Bytes)
    (large : data.size ≥ 2 ^ (8 * bytesPerOffset)) :
    structSlices fields data = .error (.offsetOverflow data.size) := by
  -- The total composite byte count is checked before any offsets or field payloads are interpreted.
  simp [structSlices, large, throw_error, Bind.bind, Except.bind]

/-- A vector budget outside the offset range is rejected even for fixed-size elements. -/
theorem vectorSlices_overflow (element : Desc) (count : Nat) (data : Bytes)
    (large : data.size ≥ 2 ^ (8 * bytesPerOffset)) :
    vectorSlices element count data = .error (.offsetOverflow data.size) := by
  -- Both fixed and variable element layouts enforce the same four-byte offset ceiling.
  cases element.fixedSize <;>
    simp [vectorSlices, large, throw_error, Bind.bind, Except.bind]

/-- A list budget outside the offset range is rejected independently of its capacity. -/
theorem listSlices_overflow (element : Desc) (limit : Option Nat) (data : Bytes)
    (large : data.size ≥ 2 ^ (8 * bytesPerOffset)) :
    listSlices element limit data = .error (.offsetOverflow data.size) := by
  -- Even an unbounded element capacity does not enlarge the composite byte-offset range.
  simp [listSlices, large, throw_error, Bind.bind, Except.bind]

end Ssz
