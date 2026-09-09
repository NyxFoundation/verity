import Ssz.Codec.RoundTripComposite

/-! Canonical sequence encodings split back into precisely their original element bytes. -/

namespace Ssz

/-- A fixed-width sequence splits into its original parts under an optional count limit. -/
theorem listSlices_concat_fixed {element : Desc} {width : Nat}
    (fixed : element.fixedSize = some width) (positive : 0 < width)
    (parts : List Bytes) (widths : ∀ part ∈ parts, part.size = width)
    (limit : Option Nat) (within : ∀ cap, limit = some cap → parts.length ≤ cap)
    (nameable : (concatParts parts).size < 2 ^ (8 * bytesPerOffset)) :
    listSlices element limit (concatParts parts) = .ok parts := by
  -- Equal element widths recover the count by division, including the empty sequence.
  cases parts with
  | nil => simp [listSlices, concatParts, Pure.pure, Except.pure]
  | cons part rest =>
    have sizeIs := concatParts_size width (part :: rest) widths
    -- A positive width makes division recover the element count exactly.
    have counted : (part :: rest).length * width / width = (part :: rest).length :=
      Nat.mul_div_cancel _ positive
    -- At least one positive-width element distinguishes this case from the empty encoding.
    have notEmpty : (part :: rest).length * width ≠ 0 := by
      have := Nat.mul_pos (by simp : 0 < (part :: rest).length) positive
      omega
    rw [sizeIs] at nameable
    -- Canonical fixed-width windows recover each concatenated part exactly.
    have extracted := map_extract_concatParts width (part :: rest) widths
    -- Adding a capacity check changes only whether the recovered count is permitted.
    cases limit with
    | none =>
      simpa only [listSlices, sizeIs, fixed, beq_iff_eq, bne_iff_ne,
        if_neg notEmpty, if_neg (Nat.ne_of_gt positive), if_neg (Nat.not_le.mpr nameable),
        Nat.mul_mod_left, ne_self_iff_false, counted, if_false, Bind.bind, Except.bind, Pure.pure, Except.pure] using congrArg Except.ok extracted
    | some cap =>
      have inLimit := within cap rfl
      simpa only [listSlices, sizeIs, fixed, beq_iff_eq, bne_iff_ne,
        if_neg notEmpty, if_neg (Nat.ne_of_gt positive), if_neg (Nat.not_le.mpr nameable),
        Nat.mul_mod_left, ne_self_iff_false, counted, if_false, Bind.bind, Except.bind, Pure.pure, Except.pure,
        if_neg (Nat.not_lt.mpr inLimit)]
        using congrArg Except.ok extracted

/-- A variable-width sequence splits at the offsets originally written for its parts. -/
theorem listSlices_offset_parts {element : Desc} (varying : element.fixedSize = none)
    (parts : List Bytes) (limit : Option Nat)
    (within : ∀ cap, limit = some cap → parts.length ≤ cap)
    (nameable : parts.length * bytesPerOffset + bodyWidth (offsetSlots parts) <
      2 ^ (8 * bytesPerOffset)) :
    listSlices element limit
      (headOf (parts.length * bytesPerOffset) (offsetSlots parts) ++ concatParts parts) = .ok parts := by
  -- With no elements, both the table and body are empty.
  cases parts with
  | nil => simp [listSlices, headOf, concatParts, Pure.pure, Except.pure]
  | cons part rest =>
    let parts := part :: rest
    have nameable : parts.length * bytesPerOffset + bodyWidth (offsetSlots parts) <
        2 ^ (8 * bytesPerOffset) := nameable
    have sizeIs := offset_encoding_size parts
    -- The header occupies exactly four bytes for each element.
    have frontIs : (headOf (parts.length * bytesPerOffset) (offsetSlots parts)).size =
        parts.length * bytesPerOffset := by rw [headOf_size, headWidth_offset]
    -- The first offset measures the whole table and therefore fixes the element count.
    have tableWidth : bytesPerOffset ≤ parts.length * bytesPerOffset := by
      simp only [parts, bytesPerOffset, List.length_cons]
      omega
    have firstIs : readUint (headOf (parts.length * bytesPerOffset) (offsetSlots parts)
        ++ concatParts parts) 0 bytesPerOffset = parts.length * bytesPerOffset := by
      have read := readUint_headOf_offset parts (parts.length * bytesPerOffset) #[]
        (concatParts parts) 0 (by simp [parts]) nameable
      simpa [parts, runningOffsets] using read
    -- Dividing the first offset by four therefore recovers the number of elements.
    have counted : parts.length * bytesPerOffset / bytesPerOffset = parts.length :=
      Nat.mul_div_cancel _ (by decide)
    have notEmpty : parts.length * bytesPerOffset + bodyWidth (offsetSlots parts) ≠ 0 := by
      change 4 ≤ parts.length * 4 at tableWidth
      change parts.length * 4 + bodyWidth (offsetSlots parts) ≠ 0
      omega
    -- Every stored offset names its original body segment, and the final span reaches the end.
    have extracted := map_extract_offsets parts (headOf (parts.length * bytesPerOffset) (offsetSlots parts))
    rw [frontIs] at extracted
    -- The nonempty header contains its first complete offset, and its end lies within the whole encoding.
    have scopeEnough : ¬ parts.length * bytesPerOffset + bodyWidth (offsetSlots parts) < bytesPerOffset := by omega
    have firstEnough : ¬ parts.length * bytesPerOffset < bytesPerOffset := by omega
    have inScope : ¬ parts.length * bytesPerOffset >
        parts.length * bytesPerOffset + bodyWidth (offsetSlots parts) := by omega
    change listSlices element limit (headOf (parts.length * bytesPerOffset) (offsetSlots parts)
      ++ concatParts parts) = .ok parts
    -- Both capacity choices use the same recovered offsets and body spans.
    cases limit with
    | none =>
      simpa only [listSlices, varying, sizeIs, firstIs, counted, if_neg (Nat.not_le.mpr nameable),
        beq_iff_eq, bne_iff_ne, if_neg notEmpty, if_neg scopeEnough, if_neg firstEnough,
        Nat.mul_mod_left, ne_self_iff_false, if_false, if_neg inScope,
        readOffsets_headOf parts _ _ nameable, offsetSpans_runningOffsets,
        Bind.bind, Except.bind, Pure.pure, Except.pure] using congrArg Except.ok extracted
    | some cap =>
      have inLimit : parts.length ≤ cap := within cap rfl
      simpa only [listSlices, varying, sizeIs, firstIs, counted, if_neg (Nat.not_le.mpr nameable),
        beq_iff_eq, bne_iff_ne, if_neg notEmpty, if_neg scopeEnough, if_neg firstEnough,
        Nat.mul_mod_left, ne_self_iff_false, if_false, if_neg inScope,
        readOffsets_headOf parts _ _ nameable, offsetSpans_runningOffsets,
        Bind.bind, Except.bind, Pure.pure, Except.pure, if_neg (Nat.not_lt.mpr inLimit)]
        using congrArg Except.ok extracted

end Ssz
