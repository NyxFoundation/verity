import Ssz.Hash.Sha256

/-! Fixed widths, padding, and the SHA-256 message-schedule recurrence. -/

namespace Ssz.Sha256

/-- Every digest has the width required of an SSZ Merkle node. -/
@[simp] theorem hash_size (message : ByteArray) : (hash message).size = 32 := by
  -- Output positions are the thirty-two finite indices, regardless of the folded state.
  simp only [hash, digest, ByteArray.size, Array.size_ofFn]

/-- Compression preserves the eight-word chaining state. -/
@[simp] theorem compress_size (state : Vector UInt32 8) (block : ByteArray) (start : Nat) :
    (compress state block start).toArray.size = 8 := by
  -- The state has eight positions in its type, so no round can change its width.
  simp

/-- Each block supplies exactly one word for each of the sixty-four rounds. -/
@[simp] theorem schedule_size (block : ByteArray) (start : Nat) :
    (schedule block start).toArray.size = 64 := by
  -- The schedule has exactly sixty-four positions in its type.
  simp

/-- Extending the schedule preserves every word already computed. -/
theorem schedulePrefix_get (block : ByteArray) (start count upper i : Nat)
    (within : i < count) (bound : count ≤ upper) :
    (schedulePrefix block start upper)[i]'(by omega) =
      (schedulePrefix block start count)[i] := by
  -- The only new entry is appended, so induction never changes an earlier position.
  induction upper generalizing count with
  | zero => omega
  | succ upper ih =>
    by_cases same : count = upper + 1
    · subst count; rfl
    -- When this is not the full prefix, removing its last appended entry leaves the requested position intact.
    · have lower : count ≤ upper := by omega
      rw [schedulePrefix, Vector.getElem_push_lt (by omega)]
      exact ih count within lower

/-- The first sixteen schedule words are the block's big-endian input words. -/
theorem schedule_input (block : ByteArray) (start i : Nat) (input : i < 16) :
    (schedule block start)[i]'(by omega) = wordAt block (start + 4 * i) := by
  -- Later extensions preserve the word at the position where it was first appended.
  rw [schedule, schedulePrefix_get block start (i + 1) 64 i (by omega) (by omega)]
  simp [schedulePrefix, input]

/-- Later words satisfy the four-term recurrence of FIPS 180-4, section 6.2.2. -/
theorem schedule_recurrence (block : ByteArray) (start i : Nat)
    (later : 16 ≤ i) (within : i < 64) :
    (schedule block start)[i] =
      (schedule block start)[i - 16]'(by omega) +
      sigma0 ((schedule block start)[i - 15]'(by omega)) +
      (schedule block start)[i - 7]'(by omega) +
      sigma1 ((schedule block start)[i - 2]'(by omega)) := by
  -- The recurrence reads only older words, whose values are preserved in the final schedule.
  unfold schedule
  rw [schedulePrefix_get block start (i + 1) 64 i (by omega) (by omega)]
  rw [schedulePrefix_get block start i 64 (i - 16) (by omega) (by omega)]
  rw [schedulePrefix_get block start i 64 (i - 15) (by omega) (by omega)]
  rw [schedulePrefix_get block start i 64 (i - 7) (by omega) (by omega)]
  rw [schedulePrefix_get block start i 64 (i - 2) (by omega) (by omega)]
  simp [schedulePrefix, show ¬ i < 16 by omega]

/-- A length occupies the final eight bytes of the padded message. -/
@[simp] theorem lengthBytes_size (size : Nat) : (lengthBytes size).size = 8 := by
  -- One output position is allocated for each byte of the 64-bit length.
  simp only [lengthBytes, ByteArray.size, Array.size_ofFn]

/-- Padding adds a delimiter, fewer than sixty-four zeros, and the eight-byte length. -/
theorem pad_size (message : ByteArray) :
    (pad message).size = message.size + 1 + paddingZeros message.size + 8 := by
  -- The message, delimiter, zero suffix, and length word occupy disjoint appended regions.
  simp only [pad, ByteArray.size_append, ByteArray.size_push, lengthBytes_size]
  simp only [ByteArray.size, Array.size_replicate]

/-- Whole zero-byte padding never adds an unnecessary complete block. -/
theorem paddingZeros_lt (size : Nat) : paddingZeros size < 64 := by
  -- Choosing the remainder modulo 64 excludes an unnecessary extra block.
  exact Nat.mod_lt _ (by decide)

/-- The padded message is an integral number of sixty-four-byte blocks. -/
theorem pad_size_mod (message : ByteArray) : (pad message).size % 64 = 0 := by
  -- The zero suffix cancels the residue left by the message and its nine framing bytes.
  rw [pad_size]
  unfold paddingZeros
  omega

/-- The chosen zero count is the smallest count that completes a block. -/
theorem paddingZeros_minimal (size zeros : Nat)
    (complete : (size + 1 + zeros + 8) % 64 = 0) : paddingZeros size ≤ zeros := by
  -- Congruence fixes one residue below sixty-four, and all other solutions add whole blocks.
  unfold paddingZeros
  omega

/-- Every byte read while parsing a scheduled block lies within the padded message. -/
theorem padded_word_in_bounds (message : ByteArray) (block word : Nat)
    (blockWithin : block < (pad message).size / 64) (wordWithin : word < 16) :
    64 * block + 4 * word + 3 < (pad message).size := by
  -- Sixteen four-byte words occupy exactly one complete block.
  omega

/-- Padding preserves each original byte at its original position. -/
theorem pad_prefix (message : ByteArray) (i : Nat) (within : i < message.size) :
    (pad message)[i]'(by rw [pad_size]; omega) = message[i] := by
  -- Both appended regions lie strictly after the original message.
  unfold pad
  rw [ByteArray.getElem_append_left (by simp; omega)]
  rw [ByteArray.getElem_append_left (by simp; omega)]
  exact Array.getElem_push_lt within

/-- The first byte after the message carries exactly the closing one bit. -/
theorem pad_delimiter (message : ByteArray) :
    (pad message)[message.size]'(by rw [pad_size]; omega) = 0x80 := by
  -- The first appended byte lies after every original byte and before both suffix regions.
  unfold pad
  rw [ByteArray.getElem_append_left (by simp; omega)]
  rw [ByteArray.getElem_append_left (by simp)]
  exact Array.getElem_push_eq

/-- Every whole byte between the delimiter and the length is zero. -/
theorem pad_zero (message : ByteArray) (i : Nat) (within : i < paddingZeros message.size) :
    (pad message)[message.size + 1 + i]'(by rw [pad_size]; omega) = 0 := by
  -- The requested position lies inside the replicated zero region, between delimiter and length.
  have zeroSize : (ByteArray.mk (Array.replicate (paddingZeros message.size) 0)).size =
      paddingZeros message.size := Array.size_replicate
  unfold pad
  rw [ByteArray.getElem_append_left (by
    simp only [ByteArray.size_append, ByteArray.size_push, zeroSize]
    omega)]
  -- After excluding the final length suffix, the position belongs to the zero padding rather than the message.
  rw [ByteArray.getElem_append_right (by simp)]
  exact Array.getElem_replicate _

/-- The encoded bit length occupies exactly the final eight bytes. -/
theorem pad_length_suffix (message : ByteArray) :
    (pad message).extract ((pad message).size - 8) (pad message).size =
      lengthBytes message.size := by
  -- The length is the final appended region, and its width is fixed independently of the input.
  have zeroSize : (ByteArray.mk (Array.replicate (paddingZeros message.size) 0)).size =
      paddingZeros message.size := Array.size_replicate
  rw [pad_size]
  unfold pad
  apply ByteArray.extract_append_eq_right
  · simp only [ByteArray.size_append, ByteArray.size_push, zeroSize]
    omega
  · simp only [ByteArray.size_append, ByteArray.size_push, lengthBytes_size, zeroSize]

private theorem messageBitLength_toNat (size : Nat) (admitted : size < 2 ^ 61) :
    (UInt64.ofNat size * 8).toNat = size * 8 := by
  -- FIPS 180-4 admits fewer than 2^64 message bits, so this multiplication cannot wrap.
  have sizeBound : size < UInt64.size := by change size < 18446744073709551616; omega
  simp only [UInt64.toNat_mul, UInt64.toNat_ofNat_of_lt' sizeBound]
  change (size * 8) % 18446744073709551616 = size * 8
  exact Nat.mod_eq_of_lt (by omega)

private theorem lengthByteOffset_toNat (i : Fin 8) :
    (8 * UInt64.ofNat i.val).toNat = 8 * i.val := by
  -- The eight byte positions occupy only bit offsets zero through fifty-six.
  have bound : i.val < UInt64.size := by change i.val < 18446744073709551616; omega
  simp only [UInt64.toNat_mul, UInt64.toNat_ofNat_of_lt' bound]
  change (8 * i.val) % 18446744073709551616 = 8 * i.val
  exact Nat.mod_eq_of_lt (by omega)

private theorem lengthByteShift_toNat (i : Fin 8) :
    (56 - 8 * UInt64.ofNat i.val).toNat = 56 - 8 * i.val := by
  -- Big-endian extraction starts with the highest byte and never subtracts past zero.
  rw [UInt64.toNat_sub_of_le]
  · simp only [lengthByteOffset_toNat]; rfl
  · apply UInt64.le_iff_toNat_le.mpr
    rw [lengthByteOffset_toNat]
    change 8 * i.val ≤ 56
    omega

/-- An admitted message length is encoded in big-endian base-256 digits without overflow. -/
theorem lengthBytes_digit (size : Nat) (admitted : size < 2 ^ 61) (i : Fin 8) :
    ((lengthBytes size)[i.val]'(by simp)).toNat =
      (size * 8 / 2 ^ (56 - 8 * i.val)) % 256 := by
  -- Exact length and shift conversions reduce byte extraction to natural-number arithmetic.
  simp only [lengthBytes, ByteArray.getElem_eq_getElem_data, Array.getElem_ofFn,
    UInt64.toNat_toUInt8, UInt64.toNat_shiftRight, messageBitLength_toNat size admitted,
    lengthByteShift_toNat]
  -- Dividing selects the byte, and reduction modulo 256 keeps its eight bits.
  change ((size * 8) >>> ((56 - 8 * i.val) % 64)) % 256 = _
  rw [Nat.mod_eq_of_lt (show 56 - 8 * i.val < 64 by omega), Nat.shiftRight_eq_div_pow]

end Ssz.Sha256
