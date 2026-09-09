import Ssz.Codec.Deserialize

/-! Reading back the offset table a struct or a sequence was laid out with. -/

namespace Ssz

/-- An integer takes exactly the width it was written at. -/
theorem uintBytes_size (width value : Nat) : (uintBytes width value).size = width := by
  -- The value shrinks by a factor of 256 at every step, so it stays quantified over.
  induction width generalizing value with
  -- No bytes were asked for, and none were written.
  | zero => rfl
  -- One byte was written and the rest is the same claim one width down.
  | succ width ih => simp only [uintBytes, Array.size_append, Array.size_singleton, ih]; omega

/-- Reading past a byte put on the front is reading the rest from one place earlier. -/
theorem readUint_shift (data : Bytes) (byte : UInt8) (start width : Nat) :
    readUint (#[byte] ++ data) (start + 1) width = readUint data start width := by
  -- The read moves along as the width shrinks, so the position stays quantified over.
  induction width generalizing start with
  -- A read of no bytes is zero wherever it starts.
  | zero => rfl
  | succ width ih =>
    -- The tail of the read is the same claim one width down, which the hypothesis gives.
    simp only [readUint, ih]
    -- What is left is the one byte this step reads.
    congr 2
    -- Past the single byte on the front, every position is one place along.
    simp [Array.getElem?_append]

/-- Reading past a run of bytes on the front is reading the rest from where it starts. -/
theorem readUint_append (front back : Bytes) (start width : Nat) :
    readUint (front ++ back) (front.size + start) width = readUint back start width := by
  -- The read moves along as the width shrinks, so the position stays quantified over.
  induction width generalizing start with
  | zero => rfl
  | succ width ih =>
    simp only [readUint]
    -- The one byte this step reads lies past the front.
    have byteIs : (front ++ back)[front.size + start]? = back[start]? := by
      rw [Array.getElem?_append_right (by omega), Nat.add_sub_cancel_left]
    -- The tail of the read is the same claim one width down, one position along.
    have along : front.size + start + 1 = front.size + (start + 1) := by omega
    rw [byteIs, along, ih]

/--
An integer written and read back at the same width is the integer written.

Stated with bytes allowed to follow, since an offset is read out of a longer encoding.
-/
theorem readUint_uintBytes_prefix (back : Bytes) (width value : Nat)
    (bound : value < 2 ^ (8 * width)) :
    readUint (uintBytes width value ++ back) 0 width = value := by
  induction width generalizing value back with
  | zero =>
    -- A width of no bytes holds only the value zero.
    simp only [Nat.mul_zero, Nat.pow_zero] at bound
    simp only [readUint]
    omega
  | succ width ih =>
    -- One more byte of width is one more factor of 256 of range.
    have step : 2 ^ (8 * (width + 1)) = 2 ^ (8 * width) * 256 := by
      rw [Nat.mul_succ, Nat.pow_add]
    -- So what is left after the low byte still fits the width that is left.
    have tail : value / 256 < 2 ^ (8 * width) := by omega
    -- The first byte of the encoding is the one this step wrote.
    have head : (uintBytes (width + 1) value ++ back)[0]?.getD 0
        = UInt8.ofNat (value % 256) := by
      simp [uintBytes, Array.getElem?_append]
    have low : (UInt8.ofNat (value % 256)).toNat = value % 256 := by simp
    simp only [readUint, head, low]
    -- Past that byte the read continues into what the rest of the value wrote.
    have shifted : readUint (uintBytes (width + 1) value ++ back) 1 width
        = readUint (uintBytes width (value / 256) ++ back) 0 width := by
      have := readUint_append #[UInt8.ofNat (value % 256)]
        (uintBytes width (value / 256) ++ back) 0 width
      simpa [uintBytes, Array.append_assoc] using this
    rw [shifted, ih back (value / 256) tail]
    -- A number is its low byte plus 256 times the rest.
    omega

/--
An integer written and read back at the same width is the integer written.

The bound is the type's own.
An integer at or above what its width holds is no value of that type.
-/
theorem readUint_uintBytes (width value : Nat) (bound : value < 2 ^ (8 * width)) :
    readUint (uintBytes width value) 0 width = value := by
  induction width generalizing value with
  | zero =>
    -- A width of no bytes holds only the value zero.
    simp only [Nat.mul_zero, Nat.pow_zero] at bound
    simp only [readUint]
    omega
  | succ width ih =>
    -- One more byte of width is one more factor of 256 of range.
    have step : 2 ^ (8 * (width + 1)) = 2 ^ (8 * width) * 256 := by
      rw [Nat.mul_succ, Nat.pow_add]
    -- So what is left after the low byte still fits the width that is left.
    have tail : value / 256 < 2 ^ (8 * width) := by omega
    -- The first byte of the encoding is the one this step wrote.
    have head : (#[UInt8.ofNat (value % 256)] ++ uintBytes width (value / 256))[0]?.getD 0
        = UInt8.ofNat (value % 256) := by simp [Array.getElem?_append]
    -- And it carries the low byte exactly, being already below 256.
    have low : (UInt8.ofNat (value % 256)).toNat = value % 256 := by simp
    simp only [uintBytes, readUint, head, low]
    -- The rest of the read is the same claim about the value with its low byte dropped.
    rw [readUint_shift, ih (value / 256) tail]
    -- A number is its low byte plus 256 times the rest.
    omega

/-- A run of bytes read back out of the middle of what it was placed in. -/
theorem extract_middle (front part tail : Bytes) :
    (front ++ (part ++ tail)).extract front.size (front.size + part.size) = part := by
  -- The requested interval skips the prefix and ends before the suffix.
  apply Array.ext
  · simp
  -- Every position in that interval addresses the corresponding byte of the middle payload.
  · intro position _ inPart
    simp only [Array.getElem_extract,
      Array.getElem_append_right (by simp : front.size ≤ front.size + position),
      Nat.add_sub_cancel_left, Array.getElem_append_left inPart]

/-- The slots a struct's fields and parts stand for. -/
def slotsOf (fields : List Desc) (parts : List Bytes) : List (Bool × Bytes) :=
  (fields.map Desc.isFixed).zip parts

/-- What reading the fixed part is meant to answer with. -/
def expectedSlots (bodyStart : Nat) : List Desc → List Bytes → List Slot
  | [], _ => []
  | _ :: _, [] => []
  | field :: fields, part :: parts =>
    -- Fixed fields retain their bytes, while variable fields record the next free body position.
    if field.isFixed then .inline part :: expectedSlots bodyStart fields parts
    else .body bodyStart :: expectedSlots (bodyStart + part.size) fields parts

/-- One field and its part put one slot on the front of both. -/
theorem slotsOf_cons (field : Desc) (fields : List Desc) (part : Bytes) (parts : List Bytes) :
    slotsOf (field :: fields) (part :: parts)
      = (field.isFixed, part) :: slotsOf fields parts := rfl

/--
The fixed part reads back the slots that wrote it.

The front is what already lies behind the read, which is how the induction moves along without the bounds it checks against changing.
-/
theorem readSlots_headOf :
    ∀ (fields : List Desc) (parts : List Bytes) (front bodies : Bytes) (bodyStart : Nat),
      fields.length = parts.length →
      (∀ pair ∈ fields.zip parts, ∀ width, pair.1.fixedSize = some width → pair.2.size = width) →
      bodyStart + bodyWidth (slotsOf fields parts) < 2 ^ (8 * bytesPerOffset) →
      readSlots fields (front ++ (headOf bodyStart (slotsOf fields parts) ++ bodies))
          front.size
        = .ok (expectedSlots bodyStart fields parts,
            front.size + headWidth (slotsOf fields parts)) := by
  intro fields
  induction fields with
  | nil =>
    intro parts front bodies bodyStart _ _ _
    simp [readSlots, slotsOf, expectedSlots, headWidth]
  | cons field fields ih =>
    intro parts front bodies bodyStart paired widths nameable
    match parts with
    | [] => simp at paired
    | part :: parts =>
      rw [slotsOf_cons] at nameable ⊢
      -- Either the field is written in place, or an offset stands for it.
      cases fixed : field.fixedSize with
      | some width =>
        have isFixed : field.isFixed = true := by simp [Desc.isFixed, fixed]
        have partWidth : part.size = width :=
          widths (field, part) (by simp) width fixed
        simp only [headOf, bodyWidth, headWidth, if_pos, expectedSlots,
          Desc.isFixed, fixed, Option.isSome_some] at nameable ⊢
        -- The bytes of this field sit here in full, and everything else follows them.
        have reassoc : front ++ (part ++ headOf bodyStart (slotsOf fields parts) ++ bodies)
            = (front ++ part) ++ (headOf bodyStart (slotsOf fields parts) ++ bodies) := by
          simp [Array.append_assoc]
        have narrowed : ∀ pair ∈ fields.zip parts, ∀ w, pair.1.fixedSize = some w →
            pair.2.size = w := fun pair member w named =>
          widths pair (List.mem_cons_of_mem _ member) w named
        have inner := ih parts (front ++ part) bodies bodyStart
          (by simpa using paired) narrowed nameable
        -- The rest of the read is the same claim with this field behind it.
        rw [← reassoc] at inner
        simp only [Array.size_append, partWidth] at inner
        simp only [readSlots, fixed]
        rw [if_neg (by simp [partWidth])]
        simp only [inner]
        -- What was cut out is the whole of this field's own bytes.
        simp [← partWidth, Nat.add_assoc]
        rfl
      | none =>
        have notFixed : field.isFixed = false := by simp [Desc.isFixed, fixed]
        simp only [notFixed, headOf, bodyWidth, headWidth, if_neg, expectedSlots,
          Bool.false_eq_true, not_false_eq_true] at nameable ⊢
        -- The offset stands in for the body, and the body follows the fixed part.
        have offsetWidth : (uintBytes bytesPerOffset bodyStart).size = bytesPerOffset :=
          uintBytes_size bytesPerOffset bodyStart
        have reassoc :
            front ++ (uintBytes bytesPerOffset bodyStart
              ++ headOf (bodyStart + part.size) (slotsOf fields parts) ++ bodies)
              = (front ++ uintBytes bytesPerOffset bodyStart)
                ++ (headOf (bodyStart + part.size) (slotsOf fields parts) ++ bodies) := by
          simp [Array.append_assoc]
        have narrowed : ∀ pair ∈ fields.zip parts, ∀ w, pair.1.fixedSize = some w →
            pair.2.size = w := fun pair member w named =>
          widths pair (List.mem_cons_of_mem _ member) w named
        have inner := ih parts (front ++ uintBytes bytesPerOffset bodyStart) bodies
          (bodyStart + part.size) (by simpa using paired) narrowed (by omega)
        rw [← reassoc] at inner
        simp only [Array.size_append, offsetWidth] at inner
        -- The offset written here is the one read back.
        have offsetIs : readUint (front ++ (uintBytes bytesPerOffset bodyStart
            ++ headOf (bodyStart + part.size) (slotsOf fields parts) ++ bodies))
            front.size bytesPerOffset = bodyStart := by
          have shaped : front ++ (uintBytes bytesPerOffset bodyStart
              ++ headOf (bodyStart + part.size) (slotsOf fields parts) ++ bodies)
              = front ++ (uintBytes bytesPerOffset bodyStart
                ++ (headOf (bodyStart + part.size) (slotsOf fields parts) ++ bodies)) := by
            simp [Array.append_assoc]
          rw [shaped]
          have past := readUint_append front (uintBytes bytesPerOffset bodyStart
            ++ (headOf (bodyStart + part.size) (slotsOf fields parts) ++ bodies)) 0 bytesPerOffset
          simp only [Nat.add_zero] at past
          rw [past, readUint_uintBytes_prefix _ _ _ (by omega)]
        simp only [readSlots, fixed]
        rw [if_neg (by simp [offsetWidth]), offsetIs]
        simp only [inner]
        simp [Nat.add_assoc]
        rfl

/-- Where each body begins, in the order the table names them. -/
def bodyOffsets (bodyStart : Nat) : List Desc → List Bytes → List Nat
  | [], _ => []
  | _ :: _, [] => []
  | field :: fields, part :: parts =>
    -- Only variable fields advance the body cursor and contribute an offset.
    if field.isFixed then bodyOffsets bodyStart fields parts
    else bodyStart :: bodyOffsets (bodyStart + part.size) fields parts

/-- How wide each body is, in the same order. -/
def bodySizes : List Desc → List Bytes → List Nat
  | [], _ => []
  | _ :: _, [] => []
  | field :: fields, part :: parts =>
    if field.isFixed then bodySizes fields parts else part.size :: bodySizes fields parts

/-- The offsets read out of the slots are the offsets the layout named. -/
theorem bodyStarts_expectedSlots :
    ∀ (fields : List Desc) (parts : List Bytes) (bodyStart : Nat),
      bodyStarts (expectedSlots bodyStart fields parts) = bodyOffsets bodyStart fields parts := by
  intro fields
  induction fields with
  | nil => intro parts _; simp [expectedSlots, bodyStarts, bodyOffsets]
  | cons field fields ih =>
    intro parts bodyStart
    match parts with
    | [] => simp [expectedSlots, bodyStarts, bodyOffsets]
    | part :: parts =>
      -- A field written in place names no offset, and any other names exactly one.
      by_cases fixed : field.isFixed
      · simp [expectedSlots, bodyStarts, bodyOffsets, fixed, ih]
      · simp [expectedSlots, bodyStarts, bodyOffsets, fixed, ih]

/-- The first body a layout names begins where the layout says the bodies begin. -/
theorem bodyOffsets_head :
    ∀ (fields : List Desc) (parts : List Bytes) (bodyStart next : Nat) (rest : List Nat),
      bodyOffsets bodyStart fields parts = next :: rest → next = bodyStart := by
  intro fields
  induction fields with
  | nil => intro parts _ _ _ named; simp [bodyOffsets] at named
  | cons field fields ih =>
    intro parts bodyStart next rest named
    match parts with
    | [] => simp [bodyOffsets] at named
    | part :: parts =>
      by_cases fixed : field.isFixed
      · -- A field written in place names nothing, so the first name comes from later.
        rw [bodyOffsets, if_pos fixed] at named
        exact ih parts bodyStart next rest named
      · rw [bodyOffsets, if_neg fixed] at named
        exact (List.cons.inj named).1.symm

/-- A layout that names no more bodies has no more body bytes, and no more widths. -/
theorem bodySizes_of_no_offsets :
    ∀ (fields : List Desc) (parts : List Bytes) (bodyStart : Nat),
      bodyOffsets bodyStart fields parts = [] →
        bodySizes fields parts = [] ∧ bodyWidth (slotsOf fields parts) = 0 := by
  intro fields
  -- Without any body offsets, every paired field must be stored inline.
  induction fields with
  | nil => intro parts _ _; simp [bodySizes, slotsOf, bodyWidth]
  | cons field fields ih =>
    intro parts bodyStart named
    match parts with
    | [] => simp [bodySizes, slotsOf, bodyWidth]
    | part :: parts =>
      rw [slotsOf_cons]
      by_cases fixed : field.isFixed
      · rw [bodyOffsets, if_pos fixed] at named
        simpa [bodySizes, fixed, bodyWidth] using ih parts bodyStart named
      -- A variable field would contribute an offset, contradicting the empty offset list.
      · rw [bodyOffsets, if_neg fixed] at named
        simp at named

/--
The spans between consecutive offsets are the widths of the bodies.

The last body is closed by the budget, which is where the whole encoding ends.
-/
theorem offsetSpans_bodyOffsets :
    ∀ (fields : List Desc) (parts : List Bytes) (bodyStart : Nat),
      offsetSpans (bodyOffsets bodyStart fields parts)
          (bodyStart + bodyWidth (slotsOf fields parts))
        = .ok (bodySizes fields parts) := by
  intro fields
  induction fields with
  | nil => intro parts _; simp [bodyOffsets, bodySizes, offsetSpans]
  | cons field fields ih =>
    intro parts bodyStart
    match parts with
    | [] => simp [bodyOffsets, bodySizes, offsetSpans]
    | part :: parts =>
      rw [slotsOf_cons]
      by_cases fixed : field.isFixed
      · -- A field written in place opens no body, so nothing between offsets changes.
        simp only [bodyOffsets, bodySizes, fixed, if_pos, bodyWidth]
        exact ih parts bodyStart
      · -- This field opens a body, which ends where the next one begins.
        simp only [bodyOffsets, bodySizes, fixed, if_neg, bodyWidth, Bool.false_eq_true,
          not_false_eq_true]
        cases later : bodyOffsets (bodyStart + part.size) fields parts with
        | nil =>
          -- Nothing follows, so the budget closes this body.
          obtain ⟨noSizes, noWidth⟩ := bodySizes_of_no_offsets fields parts _ later
          rw [noSizes, noWidth, offsetSpans, if_neg (by omega)]
          simp
        | cons next others =>
          have headIs : next = bodyStart + part.size :=
            bodyOffsets_head fields parts _ next others later
          have onward := ih parts (bodyStart + part.size)
          rw [later] at onward
          subst headIs
          rw [offsetSpans, if_neg (by omega),
            show bodyStart + part.size - bodyStart = part.size by omega,
            show bodyStart + (part.size + bodyWidth (slotsOf fields parts))
              = bodyStart + part.size + bodyWidth (slotsOf fields parts) by omega,
            onward]
          rfl

/--
The parts come back out of the bodies the layout put them in.

The front is the fixed part, so its width is where the first body begins.
-/
theorem takeSlots_expectedSlots :
    ∀ (fields : List Desc) (parts : List Bytes) (front : Bytes),
      fields.length = parts.length →
      takeSlots (front ++ bodiesOf (slotsOf fields parts))
          (expectedSlots front.size fields parts) (bodySizes fields parts)
        = .ok parts := by
  intro fields
  induction fields with
  | nil =>
    intro parts _ paired
    match parts with
    | [] => simp [takeSlots, expectedSlots]
  | cons field fields ih =>
    intro parts front paired
    match parts with
    | [] => simp at paired
    | part :: parts =>
      rw [slotsOf_cons]
      by_cases fixed : field.isFixed
      · -- A field written in place is already held, and no body was opened for it.
        simp only [expectedSlots, bodySizes, fixed, if_pos, bodiesOf, takeSlots]
        rw [ih parts front (by simpa using paired)]
        rfl
      · -- This field's body is the run the offset opened, and the rest follow it.
        simp only [expectedSlots, bodySizes, fixed, if_neg, bodiesOf, takeSlots,
          Bool.false_eq_true, not_false_eq_true]
        have shaped : front ++ (part ++ bodiesOf (slotsOf fields parts))
            = (front ++ part) ++ bodiesOf (slotsOf fields parts) := by
          simp [Array.append_assoc]
        have onward := ih parts (front ++ part) (by simpa using paired)
        rw [← shaped] at onward
        simp only [Array.size_append] at onward
        rw [extract_middle, onward]
        rfl

/-- The fixed part is as wide as the slots say it is. -/
theorem headOf_size : ∀ (slots : List (Bool × Bytes)) (start : Nat),
    (headOf start slots).size = headWidth slots := by
  intro slots
  -- Sum full payload widths for inline fields and four-byte widths for body references.
  induction slots with
  | nil => intro _; simp [headOf, headWidth]
  | cons slot rest ih =>
    intro start
    match slot with
    | (true, part) => simp [headOf, headWidth, ih]
    | (false, part) => simp [headOf, headWidth, ih, uintBytes_size]

/-- The bodies are as wide as the slots say they are. -/
theorem bodiesOf_size : ∀ (slots : List (Bool × Bytes)),
    (bodiesOf slots).size = bodyWidth slots := by
  intro slots
  -- Only variable-field payloads contribute bytes after the header.
  induction slots with
  | nil => simp [bodiesOf, bodyWidth]
  | cons slot rest ih =>
    match slot with
    | (true, part) => simp [bodiesOf, bodyWidth, ih]
    | (false, part) => simp [bodiesOf, bodyWidth, ih]

/-- With no bodies, the slots hold every part in place. -/
theorem map_expectedSlots_of_no_offsets :
    ∀ (fields : List Desc) (parts : List Bytes) (bodyStart : Nat),
      fields.length = parts.length →
      bodyOffsets bodyStart fields parts = [] →
      (expectedSlots bodyStart fields parts).map Slot.held = parts := by
  intro fields
  -- Every surviving slot must hold its field bytes directly.
  induction fields with
  | nil => intro parts _ paired _; match parts with | [] => simp [expectedSlots]
  | cons field fields ih =>
    intro parts bodyStart paired named
    match parts with
    | [] => simp at paired
    | part :: parts =>
      by_cases fixed : field.isFixed
      · rw [bodyOffsets, if_pos fixed] at named
        simp only [expectedSlots, fixed, if_pos, List.map_cons, Slot.held]
        rw [ih parts bodyStart (by simpa using paired) named]
      -- A body reference is impossible when the complete offset list is empty.
      · rw [bodyOffsets, if_neg fixed] at named
        simp at named

/--
A struct's parts are cut back out of the encoding they were laid into.

This is the offset table read back: what the encoder wrote, the decoder finds.
-/
theorem structSlices_assemble :
    ∀ (fields : List Desc) (parts : List Bytes),
      fields.length = parts.length →
      (∀ pair ∈ fields.zip parts, ∀ width, pair.1.fixedSize = some width → pair.2.size = width) →
      headWidth (slotsOf fields parts) + bodyWidth (slotsOf fields parts)
        < 2 ^ (8 * bytesPerOffset) →
      assemble (fields.map Desc.isFixed) parts >>= structSlices fields = .ok parts := by
  intro fields parts paired widths nameable
  have shape : (fields.map Desc.isFixed).zip parts = slotsOf fields parts := rfl
  -- The encoder writes the fixed part and then the bodies, and refuses neither.
  simp only [assemble, shape]
  rw [if_neg (by omega)]
  show structSlices fields (headOf (headWidth (slotsOf fields parts)) (slotsOf fields parts)
    ++ bodiesOf (slotsOf fields parts)) = .ok parts
  -- The fixed part reads back the slots that wrote it.
  have read := readSlots_headOf fields parts #[] (bodiesOf (slotsOf fields parts))
    (headWidth (slotsOf fields parts)) paired widths (by omega)
  simp only [Array.size_empty, Array.empty_append, Nat.zero_add] at read
  have widthIs : (headOf (headWidth (slotsOf fields parts)) (slotsOf fields parts)
      ++ bodiesOf (slotsOf fields parts)).size
      = headWidth (slotsOf fields parts) + bodyWidth (slotsOf fields parts) := by
    simp [headOf_size, bodiesOf_size]
  simp only [structSlices, read, bodyStarts_expectedSlots, widthIs,
    if_neg (Nat.not_le.mpr nameable), Bind.bind, Except.bind]
  -- Either the layout opened no body at all, or the first one starts where the slots end.
  by_cases empty : (bodyOffsets (headWidth (slotsOf fields parts)) fields parts).isEmpty = true
  · have named : bodyOffsets (headWidth (slotsOf fields parts)) fields parts = [] :=
      List.isEmpty_iff.mp empty
    obtain ⟨_, noWidth⟩ := bodySizes_of_no_offsets fields parts _ named
    rw [if_pos empty, noWidth, if_neg (by simp)]
    rw [map_expectedSlots_of_no_offsets fields parts _ paired named]
    rfl
  · rw [if_neg empty]
    -- The first body begins exactly where the fixed part ends.
    have headIs : (bodyOffsets (headWidth (slotsOf fields parts)) fields parts)[0]!
        = headWidth (slotsOf fields parts) := by
      cases named : bodyOffsets (headWidth (slotsOf fields parts)) fields parts with
      | nil => simp [named] at empty
      | cons next others =>
        simp [bodyOffsets_head fields parts _ next others named]
    rw [headIs, if_neg (by simp), offsetSpans_bodyOffsets fields parts]
    have taken := takeSlots_expectedSlots fields parts
      (headOf (headWidth (slotsOf fields parts)) (slotsOf fields parts)) paired
    rw [headOf_size] at taken
    exact taken

/-- An encoding that was written is one whose offsets four bytes can name. -/
theorem assemble_nameable (inline : List Bool) (parts : List Bytes) (bytes : Bytes)
    (wrote : assemble inline parts = .ok bytes) :
    headWidth (inline.zip parts) + bodyWidth (inline.zip parts)
      < 2 ^ (8 * bytesPerOffset) := by
  simp only [assemble] at wrote
  -- The encoder refuses anything wider, so what it wrote is inside the bound.
  by_cases wide : headWidth (inline.zip parts) + bodyWidth (inline.zip parts)
      ≥ 2 ^ (8 * bytesPerOffset)
  · rw [if_pos wide] at wrote
    simp [Functor.map, Except.map, throw, throwThe, MonadExceptOf.throw] at wrote
  · omega

/-- What was written is as wide as the fixed part and the bodies together. -/
theorem assemble_size (inline : List Bool) (parts : List Bytes) (bytes : Bytes)
    (wrote : assemble inline parts = .ok bytes) :
    bytes.size = headWidth (inline.zip parts) + bodyWidth (inline.zip parts) := by
  -- Successful assembly has already established that its total width is representable.
  have nameable := assemble_nameable inline parts bytes wrote
  simp only [assemble, if_neg (show ¬ headWidth (inline.zip parts)
    + bodyWidth (inline.zip parts) ≥ 2 ^ (8 * bytesPerOffset) by omega),
    Pure.pure, Except.pure] at wrote
  -- Identify the returned bytes as the header followed by the bodies, then add their widths.
  rw [← Except.ok.inj wrote]
  simp [headOf_size, bodiesOf_size]

/-- A struct whose fields all have a width is that wide, and opens no body. -/
theorem fieldsFixedSize_headWidth :
    ∀ (fields : List Desc) (parts : List Bytes) (width : Nat),
      fields.length = parts.length →
      (∀ pair ∈ fields.zip parts, ∀ w, pair.1.fixedSize = some w → pair.2.size = w) →
      Desc.fieldsFixedSize fields = some width →
      headWidth (slotsOf fields parts) = width ∧ bodyWidth (slotsOf fields parts) = 0 := by
  intro fields
  induction fields with
  | nil =>
    intro parts width paired _ fixed
    match parts with
    | [] => simp [slotsOf, headWidth, bodyWidth]; simpa [Desc.fieldsFixedSize] using fixed
  | cons field fields ih =>
    intro parts width paired widths fixed
    match parts with
    | [] => simp at paired
    | part :: parts =>
      rw [slotsOf_cons]
      -- A struct is fixed only when every field is, so this one is too.
      simp only [Desc.fieldsFixedSize] at fixed
      split at fixed
      · rename_i here rest fieldWidth remaining fieldIs restIs
        simp only [Option.some.injEq] at fixed
        have partWidth : part.size = fieldWidth :=
          widths (field, part) (by simp) fieldWidth fieldIs
        have onward := ih parts remaining (by simpa using paired)
          (fun pair member => widths pair (List.mem_cons_of_mem _ member)) restIs
        simp only [headWidth, bodyWidth, Desc.isFixed, fieldIs, Option.isSome_some, if_pos]
        rw [partWidth, onward.1, onward.2]
        exact ⟨by omega, rfl⟩
      · simp at fixed

/-- Parts laid end to end, which is what a sequence of fixed elements encodes to. -/
def concatParts : List Bytes → Bytes
  | [] => #[]
  | part :: rest => part ++ concatParts rest

/-- Extracting past a run on the front is extracting from what follows it. -/
theorem extract_append_shift (front back : Bytes) (start stop : Nat) :
    (front ++ back).extract (front.size + start) (front.size + stop)
      = back.extract start stop := by
  -- Subtracting the common prefix translates both extraction boundaries into the suffix.
  apply Array.ext
  · simp [Array.size_extract]
  · intro position inLeft _
    simp only [Array.getElem_extract,
      Array.getElem_append_right (by omega : front.size ≤ front.size + start + position)]
    -- The translated byte position is unchanged after cancelling the prefix length.
    congr 1
    omega

/-- Every element of a sequence of fixed elements sits in its own window. -/
theorem extract_concatParts (width : Nat) :
    ∀ (parts : List Bytes), (∀ part ∈ parts, part.size = width) →
      ∀ index, index < parts.length →
        (concatParts parts).extract (index * width) ((index + 1) * width) = parts[index]! := by
  intro parts
  induction parts with
  | nil => intro _ _ inside; simp at inside
  | cons part rest ih =>
    intro widths index inside
    have partWidth : part.size = width := widths part (by simp)
    match index with
    | 0 =>
      -- The first element begins where the whole run does.
      simp only [concatParts, Nat.zero_mul, Nat.zero_add, Nat.one_mul, ← partWidth]
      rw [Array.extract_append_left, Array.extract_size]
      simp
    | index + 1 =>
      -- Every later one lies past the first, at the same offset one element in.
      have shifted : (index + 1) * width = part.size + index * width := by
        rw [partWidth, Nat.succ_mul]
        omega
      have onward : (index + 2) * width = part.size + (index + 1) * width := by
        rw [partWidth, show index + 2 = index + 1 + 1 from rfl, Nat.succ_mul]
        omega
      rw [concatParts, shifted, onward, extract_append_shift]
      simpa using ih (fun p member => widths p (List.mem_cons_of_mem _ member)) index
        (by simpa using inside)

/-- Slots for a sequence of fixed elements, which are all written in place. -/
abbrev inlineSlots (parts : List Bytes) : List (Bool × Bytes) :=
  (parts.map fun _ => true).zip parts

/-- Every such element sits in the fixed part, one after another. -/
theorem headOf_inline : ∀ (parts : List Bytes) (start : Nat),
    headOf start (inlineSlots parts) = concatParts parts := by
  intro parts
  -- Every payload is included in the header in its original order.
  induction parts with
  | nil => intro _; simp [headOf, concatParts, inlineSlots]
  | cons part rest ih => intro start; simp only [inlineSlots, List.map_cons, List.zip_cons_cons,
      headOf, concatParts, ih]

/-- No body follows them, so the fixed part is the whole encoding. -/
theorem bodyWidth_inline : ∀ (parts : List Bytes), bodyWidth (inlineSlots parts) = 0 := by
  intro parts
  -- No inline payload contributes to the separate body region.
  induction parts with
  | nil => rfl
  | cons part rest ih => simp only [inlineSlots, List.map_cons, List.zip_cons_cons,
      bodyWidth, ih]

/-- The bodies of such a sequence are empty. -/
theorem bodiesOf_inline : ∀ (parts : List Bytes), bodiesOf (inlineSlots parts) = #[] := by
  intro parts
  -- Skipping every inline payload leaves an empty body byte string.
  induction parts with
  | nil => rfl
  | cons part rest ih => simp only [inlineSlots, List.map_cons, List.zip_cons_cons,
      bodiesOf, ih]

/-- A sequence of fixed elements encodes to its elements laid end to end. -/
theorem assemble_inline (parts : List Bytes) (bytes : Bytes)
    (wrote : assemble (parts.map fun _ => true) parts = .ok bytes) :
    bytes = concatParts parts := by
  -- Successful assembly passes the composite size bound even when no offsets are stored.
  have nameable := assemble_nameable (parts.map fun _ => true) parts bytes wrote
  simp only [assemble, if_neg (show ¬ headWidth ((parts.map fun _ => true).zip parts)
    + bodyWidth ((parts.map fun _ => true).zip parts) ≥ 2 ^ (8 * bytesPerOffset) by omega),
    Pure.pure, Except.pure] at wrote
  rw [← Except.ok.inj wrote]
  show headOf _ (inlineSlots parts) ++ bodiesOf (inlineSlots parts) = _
  -- The header is the concatenated payload and the separate body is empty.
  rw [headOf_inline, bodiesOf_inline]
  simp

/-- A run of equal-width elements is as wide as their count says. -/
theorem concatParts_size (width : Nat) : ∀ (parts : List Bytes),
    (∀ part ∈ parts, part.size = width) →
      (concatParts parts).size = parts.length * width := by
  intro parts
  -- Each appended element adds exactly one common element width.
  induction parts with
  | nil => intro _; simp [concatParts]
  | cons part rest ih =>
    intro widths
    simp only [concatParts, Array.size_append, List.length_cons,
      widths part (by simp), ih (fun p member => widths p (List.mem_cons_of_mem _ member)),
      Nat.succ_mul]
    omega

/-- Cutting a run of equal-width elements into windows gives the elements back. -/
theorem map_extract_concatParts (width : Nat) (parts : List Bytes)
    (widths : ∀ part ∈ parts, part.size = width) :
    (List.range parts.length).map
        (fun index => (concatParts parts).extract (index * width) ((index + 1) * width))
      = parts := by
  -- Recover one interval per element and compare each recovered interval with its original payload.
  apply List.ext_getElem
  · simp
  · intro index inRange inParts
    simp only [List.getElem_map, List.getElem_range]
    rw [extract_concatParts width parts widths index inParts]
    simp [getElem!_pos, inParts]

/-- Slots for a sequence of variable elements, each reached through an offset. -/
abbrev offsetSlots (parts : List Bytes) : List (Bool × Bytes) :=
  (parts.map fun _ => false).zip parts

/-- One element puts one offset slot on the front of both. -/
theorem offsetSlots_cons (part : Bytes) (rest : List Bytes) :
    offsetSlots (part :: rest) = (false, part) :: offsetSlots rest := rfl

/-- Where each body of such a sequence begins. -/
def runningOffsets (start : Nat) : List Bytes → List Nat
  | [] => []
  -- A body starts at the current cursor, and its byte length determines the next cursor.
  | part :: rest => start :: runningOffsets (start + part.size) rest

/-- The table is four bytes per element, and nothing else. -/
theorem headWidth_offset : ∀ (parts : List Bytes),
    headWidth (offsetSlots parts) = parts.length * bytesPerOffset := by
  intro parts
  -- Each variable-size element contributes exactly four header bytes, independently of payload length.
  induction parts with
  | nil => rfl
  | cons part rest ih =>
    simp only [offsetSlots_cons, headWidth, ih, List.length_cons,
      Bool.false_eq_true, if_false, Nat.succ_mul]
    omega

/-- The bodies of such a sequence are its elements laid end to end. -/
theorem bodiesOf_offset : ∀ (parts : List Bytes),
    bodiesOf (offsetSlots parts) = concatParts parts := by
  intro parts
  -- Every variable-size payload belongs to the body region in element order.
  induction parts with
  | nil => rfl
  | cons part rest ih =>
    simp only [offsetSlots_cons, bodiesOf, concatParts, ih]

/-- Each offset in the table names where its own body begins. -/
theorem readUint_headOf_offset : ∀ (parts : List Bytes) (start : Nat) (front bodies : Bytes)
    (index : Nat),
    index < parts.length →
    start + bodyWidth (offsetSlots parts) < 2 ^ (8 * bytesPerOffset) →
    readUint (front ++ (headOf start (offsetSlots parts) ++ bodies))
        (front.size + index * bytesPerOffset) bytesPerOffset
      = (runningOffsets start parts)[index]! := by
  intro parts
  induction parts with
  | nil => intro _ _ _ index inside _; simp at inside
  | cons part rest ih =>
    intro start front bodies index inside nameable
    simp only [offsetSlots_cons, headOf, bodyWidth] at nameable ⊢
    match index with
    | 0 =>
      -- The first offset sits at the front of the table.
      simp only [Nat.zero_mul, Nat.add_zero, runningOffsets]
      have shaped : front ++ (uintBytes bytesPerOffset start
          ++ headOf (start + part.size) (offsetSlots rest) ++ bodies)
          = front ++ (uintBytes bytesPerOffset start
            ++ (headOf (start + part.size) (offsetSlots rest) ++ bodies)) := by
        simp [Array.append_assoc]
      rw [shaped]
      have past := readUint_append front (uintBytes bytesPerOffset start
        ++ (headOf (start + part.size) (offsetSlots rest) ++ bodies)) 0 bytesPerOffset
      simp only [Nat.add_zero] at past
      rw [past, readUint_uintBytes_prefix _ _ _ (by omega)]
      simp
    | index + 1 =>
      -- Every later one lies four bytes further along, which is the next element's.
      have shaped : front ++ (uintBytes bytesPerOffset start
          ++ headOf (start + part.size) (offsetSlots rest) ++ bodies)
          = (front ++ uintBytes bytesPerOffset start)
            ++ (headOf (start + part.size) (offsetSlots rest) ++ bodies) := by
        simp [Array.append_assoc]
      have widthIs : (front ++ uintBytes bytesPerOffset start).size
          = front.size + bytesPerOffset := by simp [uintBytes_size]
      have onward := ih (start + part.size) (front ++ uintBytes bytesPerOffset start) bodies
        index (by simpa using inside) (by omega)
      rw [widthIs] at onward
      have shift : front.size + (index + 1) * bytesPerOffset
          = front.size + bytesPerOffset + index * bytesPerOffset := by
        rw [Nat.succ_mul]
        omega
      rw [shaped, shift, onward]
      simp [runningOffsets]

/-- One offset per element. -/
theorem runningOffsets_length : ∀ (parts : List Bytes) (start : Nat),
    (runningOffsets start parts).length = parts.length := by
  intro parts
  -- Advancing the byte cursor does not change the one-offset-per-element correspondence.
  induction parts with
  | nil => intro _; rfl
  | cons part rest ih => intro start; simp [runningOffsets, ih]

/-- The table reads back the offsets the layout named. -/
theorem readOffsets_headOf (parts : List Bytes) (start : Nat) (bodies : Bytes)
    (nameable : start + bodyWidth (offsetSlots parts) < 2 ^ (8 * bytesPerOffset)) :
    readOffsets (headOf start (offsetSlots parts) ++ bodies) parts.length
      = runningOffsets start parts := by
  -- Both tables have one entry per body, so compare their offsets position by position.
  apply List.ext_getElem
  · simp [readOffsets, runningOffsets_length]
  · intro index inRead _
    simp only [readOffsets, List.getElem_map, List.getElem_range]
    have inParts : index < parts.length := by simpa [readOffsets] using inRead
    -- Read the selected four-byte header entry using the bounded integer round-trip theorem.
    have read := readUint_headOf_offset parts start #[] bodies index inParts nameable
    simp only [Array.size_empty, Array.empty_append, Nat.zero_add] at read
    rw [read]
    simp [getElem!_pos, runningOffsets_length, inParts]

/-- The spans between those offsets are the widths of the elements. -/
theorem offsetSpans_runningOffsets : ∀ (parts : List Bytes) (start : Nat),
    offsetSpans (runningOffsets start parts) (start + bodyWidth (offsetSlots parts))
      = .ok (parts.map Array.size) := by
  intro parts
  induction parts with
  | nil => intro _; simp [runningOffsets, offsetSpans]
  | cons part rest ih =>
    intro start
    simp only [runningOffsets, offsetSlots_cons, bodyWidth]
    -- Either this is the last element, closed by the budget, or the next one closes it.
    cases later : runningOffsets (start + part.size) rest with
    | nil =>
      have none : rest = [] := by
        cases rest with
        | nil => rfl
        | cons _ _ => simp [runningOffsets] at later
      subst none
      rw [show bodyWidth (offsetSlots ([] : List Bytes)) = 0 from rfl, Nat.add_zero,
        offsetSpans, if_neg (by omega),
        show start + part.size - start = part.size from by omega]
      rfl
    | cons next others =>
      have headIs : next = start + part.size := by
        cases rest with
        | nil => simp [runningOffsets] at later
        | cons _ _ =>
          simp only [runningOffsets] at later
          exact (List.cons.inj later).1.symm
      have onward := ih (start + part.size)
      rw [later] at onward
      subst headIs
      rw [offsetSpans, if_neg (by omega),
        show start + part.size - start = part.size by omega,
        show start + (part.size + bodyWidth (offsetSlots rest))
          = start + part.size + bodyWidth (offsetSlots rest) by omega, onward]
      rfl

/-- Each body is cut back out of where its offset put it. -/
theorem map_extract_offsets : ∀ (parts : List Bytes) (front : Bytes),
    ((runningOffsets front.size parts).zip (parts.map Array.size)).map
        (fun pair => (front ++ concatParts parts).extract pair.1 (pair.1 + pair.2))
      = parts := by
  intro parts
  -- Remove the first recovered body, then treat its bytes as part of the prefix for the suffix.
  induction parts with
  | nil => intro _; simp [runningOffsets]
  | cons part rest ih =>
    intro front
    simp only [runningOffsets, concatParts, List.map_cons, List.zip_cons_cons, extract_middle]
    -- Moving the first body into the consumed prefix keeps all later absolute positions unchanged.
    have shaped : front ++ (part ++ concatParts rest) = (front ++ part) ++ concatParts rest := by
      simp [Array.append_assoc]
    have onward := ih (front ++ part)
    rw [← shaped] at onward
    simp only [Array.size_append] at onward
    -- The remaining bodies are recovered at exactly those translated positions.
    rw [onward]

/-- A sequence of variable elements encodes to its table and then its bodies. -/
theorem assemble_offset (parts : List Bytes) (bytes : Bytes)
    (wrote : assemble (parts.map fun _ => false) parts = .ok bytes) :
    bytes = headOf (parts.length * bytesPerOffset) (offsetSlots parts) ++ concatParts parts := by
  -- Successful assembly guarantees that the entire table and payload fit the offset range.
  have nameable := assemble_nameable (parts.map fun _ => false) parts bytes wrote
  simp only [assemble, if_neg (show ¬ headWidth ((parts.map fun _ => false).zip parts)
    + bodyWidth ((parts.map fun _ => false).zip parts) ≥ 2 ^ (8 * bytesPerOffset) by omega),
    Pure.pure, Except.pure] at wrote
  rw [← Except.ok.inj wrote]
  show headOf (headWidth (offsetSlots parts)) (offsetSlots parts)
    ++ bodiesOf (offsetSlots parts) = _
  -- Replace the measured header by four bytes per element and the bodies by their concatenation.
  rw [headWidth_offset, bodiesOf_offset]

/-- The whole encoding is the table and the bodies together. -/
theorem offset_encoding_size (parts : List Bytes) :
    (headOf (parts.length * bytesPerOffset) (offsetSlots parts) ++ concatParts parts).size
      = parts.length * bytesPerOffset + bodyWidth (offsetSlots parts) := by
  -- Count the four-byte entries separately from the total payload bytes.
  rw [Array.size_append, headOf_size, headWidth_offset, ← bodiesOf_size, bodiesOf_offset]

/-- The first offset names where the bodies begin. -/
theorem runningOffsets_head (start : Nat) (part : Bytes) (rest : List Bytes) :
    (runningOffsets start (part :: rest))[0]! = start := by
  -- The first body begins at the initial cursor before any payload length is added.
  simp [runningOffsets, getElem!_pos]

/-- A struct of fields that each take a byte takes one itself. -/
theorem fieldsFixedSize_pos : ∀ (fields : List Desc) (width : Nat),
    ¬ fields.isEmpty = true →
    (∀ field ∈ fields, ∀ w, field.fixedSize = some w → 0 < w) →
    Desc.fieldsFixedSize fields = some width → 0 < width := by
  intro fields
  induction fields with
  | nil => intro _ notEmpty _ _; simp at notEmpty
  | cons field rest _ =>
    intro width _ widths fixed
    simp only [Desc.fieldsFixedSize] at fixed
    -- The first field alone already takes a byte, whatever the rest take.
    split at fixed
    · rename_i fieldWidth remaining fieldIs _
      have positive := widths field (by simp) fieldWidth fieldIs
      simp only [Option.some.injEq] at fixed
      omega
    · simp at fixed
