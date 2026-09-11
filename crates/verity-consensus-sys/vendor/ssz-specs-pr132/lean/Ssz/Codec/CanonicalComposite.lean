import Ssz.Codec.CanonicalTable

/-! Canonicality is preserved when decoded elements are assembled into composites. -/

namespace Ssz

/-- Re-encoding individually canonical elements recovers every original slice in order. -/
theorem serializeEach_of_deserializeEach {element : Desc}
    (canonical : ∀ data value, deserialize element data = .ok value →
      serialize element value = .ok data) :
    ∀ slices values, deserializeEach element slices = .ok values →
      serializeEach element values = .ok slices := by
  intro slices
  -- Advance through the encoded slices in the same order as their element declarations.
  induction slices with
  | nil =>
    intro values read
    simp [deserializeEach] at read
    subst values
    simp [serializeEach]
  | cons slice slices ih =>
    intro values read
    cases head : deserialize element slice with
    | error fault => simp [deserializeEach, head, Bind.bind, Except.bind] at read
    | ok value =>
      -- A successful composite decode requires the remaining slices to decode successfully too.
      cases tail : deserializeEach element slices with
      | error fault => simp [deserializeEach, head, tail, Bind.bind, Except.bind] at read
      | ok rest =>
        simp [deserializeEach, head, tail, Bind.bind, Except.bind, pure, Except.pure] at read
        -- Re-encode the first value and the remaining values using their individual canonicality proofs.
        subst values
        simp [serializeEach, canonical slice value head, ih rest tail,
          Bind.bind, Except.bind, pure, Except.pure]

/-- Re-encoding canonical field values recovers each field's original slice. -/
theorem serializeFields_of_deserializeFields : ∀ fields slices values,
    (∀ field ∈ fields, ∀ data value, deserialize field data = .ok value →
      serialize field value = .ok data) →
    deserializeFields fields slices = .ok values →
    serializeFields fields values = .ok slices := by
  intro fields
  -- Advance through the encoded slices in the same order as their element declarations.
  induction fields with
  | nil =>
    intro slices values _ read
    cases slices <;> simp [deserializeFields] at read
    subst values
    simp [serializeFields]
  | cons field fields ih =>
    intro slices values canonical read
    cases slices with
    | nil => simp [deserializeFields] at read
    | cons slice slices =>
      cases head : deserialize field slice with
      | error fault => simp [deserializeFields, head, Bind.bind, Except.bind] at read
      | ok value =>
        -- A successful composite decode requires the remaining slices to decode successfully too.
        cases tail : deserializeFields fields slices with
        | error fault => simp [deserializeFields, head, tail, Bind.bind, Except.bind] at read
        | ok rest =>
          simp [deserializeFields, head, tail, Bind.bind, Except.bind, pure, Except.pure] at read
          -- Re-encode the first value and the remaining values using their individual canonicality proofs.
          subst values
          have later := ih slices rest
            (fun item member => canonical item (List.mem_cons_of_mem _ member)) tail
          simp [serializeFields, canonical field List.mem_cons_self slice value head, later,
            Bind.bind, Except.bind, pure, Except.pure]

/-- Decoding a union retains its selector and reconstructs its option's exact encoding. -/
theorem canonical_option : ∀ selectors options selector data value,
    (∀ option ∈ options, ∀ data value, deserialize option data = .ok value →
      serialize option value = .ok data) →
    deserializeOption selectors options selector data = .ok value →
    ∃ option held, value = .union selector held ∧
      lookupOption selectors options selector = .ok option ∧ serialize option held = .ok data := by
  intro selectors
  -- Walk the selector table alongside the option declarations.
  induction selectors with
  | nil =>
    intro options selector data value _ read
    cases options <;> simp [deserializeOption] at read
  | cons chosen selectors ih =>
    intro options selector data value canonical read
    cases options with
    | nil => simp [deserializeOption] at read
    | cons option options =>
      -- A matching selector fixes which type decodes and re-encodes the payload.
      by_cases same : chosen == selector
      · cases body : deserialize option data with
        | error fault => simp [deserializeOption, same, body, Bind.bind, Except.bind] at read
        | ok held =>
          simp [deserializeOption, same, body, Bind.bind, Except.bind, pure, Except.pure] at read
          subst value
          exact ⟨option, held, rfl, by simp [lookupOption, same],
            canonical option List.mem_cons_self data held body⟩
      -- A nonmatching entry leaves the payload unchanged while searching the remaining options.
      · obtain ⟨heldOption, held, eq, named, encoded⟩ := ih options selector data value
          (fun item member => canonical item (List.mem_cons_of_mem _ member))
          (by simpa [deserializeOption, same] using read)
        exact ⟨heldOption, held, eq, by simpa [lookupOption, same] using named, encoded⟩

/-- A nonempty byte sequence is its first byte followed by its remaining bytes. -/
theorem firstByte_append_tail (data : Bytes) (nonempty : 0 < data.size) :
    #[data[0]!] ++ data.extract 1 data.size = data := by
  -- The first one-byte slice contains exactly the selector byte.
  have first : data.extract 0 1 = #[data[0]!] := by
    have encoded := uintBytes_readUint_slice data 0 1 (by omega)
    simpa [uintBytes, readUint, getElem!_pos, nonempty, Array.getElem?_eq_getElem nonempty]
      using encoded.symm
  -- The selector slice and payload slice are adjacent and cover the whole message.
  rw [← first, extract_join data (by omega) (by omega) (by omega), Array.extract_size]

/-- Canonical option encodings give a canonical union encoding, including its selector. -/
theorem canonical_union {selectors : List Nat} {options : List Desc} {data : Bytes} {value : Value}
    (canonical : ∀ option ∈ options, ∀ data value, deserialize option data = .ok value →
      serialize option value = .ok data)
    (read : deserialize (.compatibleUnion selectors options) data = .ok value) :
    serialize (.compatibleUnion selectors options) value = .ok data := by
  -- A union encoding must contain a selector before its payload.
  simp only [deserialize] at read
  split at read
  · simp [Bind.bind, Except.bind] at read
  · rename_i nonempty
    -- Recover the chosen option together with its canonical payload encoding.
    obtain ⟨option, held, eq, named, encoded⟩ :=
      canonical_option selectors options _ _ value canonical read
    subst value
    -- Prepending the unchanged selector reconstructs the entire input.
    simp [serialize, named, encoded, Bind.bind, Except.bind, pure, Except.pure,
      firstByte_append_tail data (by omega)]

/-- A vector of canonical elements has a canonical encoding. -/
theorem canonical_vector {element : Desc} {count : Nat} {data : Bytes} {value : Value}
    (positive : 0 < count)
    (canonical : ∀ data value, deserialize element data = .ok value →
      serialize element value = .ok data)
    (read : deserialize (.vector element count) data = .ok value) :
    serialize (.vector element count) value = .ok data := by
  have domain := fits_of_deserialize _ _ _ read
  -- Recover the element boundaries before considering the values inside those boundaries.
  cases parts : vectorSlices element count data with
  | error fault => simp [deserialize, parts, Bind.bind, Except.bind] at read
  | ok slices =>
    -- Each accepted slice supplies one decoded element in the original order.
    cases decoded : deserializeEach element slices with
    | error fault => simp [deserialize, parts, decoded, Bind.bind, Except.bind] at read
    | ok values =>
      simp [deserialize, parts, decoded, Bind.bind, Except.bind, pure, Except.pure] at read
      -- Re-encoding each element recovers its slice, and canonical assembly recovers the whole byte string.
      subst value
      cases domain with
      | vector sized _ =>
        simp [serialize, sized, serializeSequence,
          serializeEach_of_deserializeEach canonical slices values decoded,
          Bind.bind, Except.bind, vectorSlices_assemble positive parts]

/-- A bounded list of canonical elements has a canonical encoding. -/
theorem canonical_list {element : Desc} {limit : Nat} {data : Bytes} {value : Value}
    (canonical : ∀ data value, deserialize element data = .ok value →
      serialize element value = .ok data)
    (read : deserialize (.list element limit) data = .ok value) :
    serialize (.list element limit) value = .ok data := by
  have domain := fits_of_deserialize _ _ _ read
  -- Recover the element boundaries before considering the values inside those boundaries.
  cases parts : listSlices element (some limit) data with
  | error fault => simp [deserialize, parts, Bind.bind, Except.bind] at read
  | ok slices =>
    -- Each accepted slice supplies one decoded element in the original order.
    cases decoded : deserializeEach element slices with
    | error fault => simp [deserialize, parts, decoded, Bind.bind, Except.bind] at read
    | ok values =>
      simp [deserialize, parts, decoded, Bind.bind, Except.bind, pure, Except.pure] at read
      -- Re-encoding each element recovers its slice, and canonical assembly recovers the whole byte string.
      subst value
      cases domain with
      | list within _ =>
        simp [serialize, within, serializeSequence,
          serializeEach_of_deserializeEach canonical slices values decoded,
          Bind.bind, Except.bind, listSlices_assemble parts]

/-- An unbounded list of canonical elements has a canonical encoding. -/
theorem canonical_progressiveList {element : Desc} {data : Bytes} {value : Value}
    (canonical : ∀ data value, deserialize element data = .ok value →
      serialize element value = .ok data)
    (read : deserialize (.progressiveList element) data = .ok value) :
    serialize (.progressiveList element) value = .ok data := by
  -- Recover the element boundaries before considering the values inside those boundaries.
  cases parts : listSlices element none data with
  | error fault => simp [deserialize, parts, Bind.bind, Except.bind] at read
  | ok slices =>
    -- Each accepted slice supplies one decoded element in the original order.
    cases decoded : deserializeEach element slices with
    | error fault => simp [deserialize, parts, decoded, Bind.bind, Except.bind] at read
    | ok values =>
      simp [deserialize, parts, decoded, Bind.bind, Except.bind, pure, Except.pure] at read
      -- Re-encoding each element recovers its slice, and canonical assembly recovers the whole byte string.
      subst value
      simp [serialize, serializeSequence,
        serializeEach_of_deserializeEach canonical slices values decoded,
        Bind.bind, Except.bind, listSlices_assemble parts]

end Ssz
