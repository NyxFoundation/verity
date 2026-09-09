import Ssz.Codec.Root
import Ssz.Type.Compatibility

/-! A byte array and its sequence of one-byte integer values have the same SSZ meaning. -/

namespace Ssz

/-- Re-express each byte as the value of a one-byte unsigned integer. -/
def byteValues (data : Bytes) : List Value :=
  data.toList.map fun byte => .uint byte.toNat

/-- Each byte encodes to itself when read as a one-byte integer. -/
theorem serializeEach_bytes (data : List UInt8) :
    serializeEach (.uint 1) (data.map fun byte => .uint byte.toNat) =
      .ok (data.map fun byte => #[byte]) := by
  -- Encoding proceeds one byte at a time and preserves both its value and its position.
  induction data with
  | nil => simp [serializeEach]
  | cons byte rest ih =>
    have bound : byte.toNat < 256 := byte.toNat_lt
    simp [serializeEach, serialize, uintBytes, bound, Nat.mod_eq_of_lt bound, ih,
      Bind.bind, Except.bind, pure, Except.pure]

/-- Singleton fixed slots contribute one byte each, all in the fixed part. -/
private theorem byteSlots_width (data : List UInt8) :
    headWidth (data.map fun byte => (true, #[byte])) = data.length ∧
      bodyWidth (data.map fun byte => (true, #[byte])) = 0 := by
  -- There are no offsets or trailing bodies for fixed-width elements.
  induction data with
  | nil => simp [headWidth, bodyWidth]
  | cons byte rest ih => simp [headWidth, bodyWidth, ih, Nat.add_comm]

/-- Singleton fixed slots concatenate to the original byte sequence. -/
private theorem byteSlots_content (data : List UInt8) (start : Nat) :
    headOf start (data.map fun byte => (true, #[byte])) = data.toArray ∧
      bodiesOf (data.map fun byte => (true, #[byte])) = #[] := by
  -- Each slot appends one original byte without changing it.
  induction data with
  | nil => simp [headOf, bodiesOf]
  | cons byte rest ih =>
    rw [List.toArray_cons]
    simp only [List.map_cons, headOf, bodiesOf, ih.1, ih.2, and_self]

/-- Byte sequences use no offsets and preserve their input below the encoder's size ceiling. -/
theorem serializeSequence_bytes (data : Bytes) (nameable : data.size < 2 ^ 32) :
    serializeSequence (.uint 1) (byteValues data) = .ok data := by
  -- Treat each original byte as one fixed-width integer payload.
  unfold serializeSequence byteValues
  rw [serializeEach_bytes]
  simp only [Bind.bind, Except.bind, Desc.isFixed, Desc.fixedSize, Option.isSome_some]
  -- The sequence contains exactly one inline slot for each byte.
  have slots : ((data.toList.map fun byte => #[byte]).map fun _ => true).zip
      (data.toList.map fun byte => #[byte]) = data.toList.map fun byte => (true, #[byte]) := by
    induction data.toList with
    | nil => rfl
    | cons byte rest ih => simp only [List.map_cons, List.zip_cons_cons, ih]
  simp only [assemble, slots]
  rw [(byteSlots_width _).1, (byteSlots_width _).2]
  simp only [Array.length_toList, Nat.add_zero]
  -- The sequence's composite bound is required even though no offset entries are stored.
  rw [if_neg (by simpa [bytesPerOffset] using Nat.not_le.mpr nameable)]
  simp only [pure, Except.pure, (byteSlots_content _ _).1, (byteSlots_content data.toList 0).2,
    Array.append_empty, Array.toArray_toList]

/-- Re-expressing bytes preserves the number of sequence elements. -/
@[simp] theorem byteValues_length (data : Bytes) : (byteValues data).length = data.size := by
  simp [byteValues]

/-- Both fixed-byte spellings serialize alike, including rejection of a wrong element count. -/
theorem serialize_byteVector_alias (data : Bytes) (length : Nat)
    (nameable : data.size < 2 ^ 32) :
    serialize (.byteVector length) (.bytes data) =
      serialize (.vector (.uint 1) length) (.seq (byteValues data)) := by
  -- Fixed-width elements add no offset table, so their assembly is the original byte array.
  simp only [serialize, byteValues_length, serializeSequence_bytes data nameable]

/-- Both bounded-byte spellings serialize alike, including rejection of an exceeded limit. -/
theorem serialize_byteList_alias (data : Bytes) (limit : Nat)
    (nameable : data.size < 2 ^ 32) :
    serialize (.byteList limit) (.bytes data) =
      serialize (.list (.uint 1) limit) (.seq (byteValues data)) := by
  -- Both representations check the same byte count and reproduce the payload within the composite bound.
  simp only [serialize, byteValues_length, serializeSequence_bytes data nameable]

/-- Appending one-byte encodings reconstructs the original bytes after any prefix. -/
private theorem fold_byteParts (data : List UInt8) (front : Bytes) :
    (data.map fun byte => #[byte]).foldl (fun total part => total ++ part) front =
      front ++ data.toArray := by
  -- Generalizing the prefix lets each induction step append the next original byte.
  induction data generalizing front with
  | nil => simp
  | cons byte rest ih =>
    simp only [List.map_cons, List.foldl_cons, ih]
    rw [List.toArray_cons byte rest, Array.append_assoc]

/-- Packing one-byte integer encodings gives the same nodes as packing the byte array. -/
theorem packElements_bytes (data : Bytes) :
    packElements (data.toList.map fun byte => #[byte]) = packBytes data := by
  -- Concatenating the one-byte integer encodings recovers the original byte array before chunking.
  simp [packElements, fold_byteParts]

/-- A fixed byte array and a vector of bytes lay down exactly the same packed tree. -/
theorem merkleLayout_byteVector_alias (data : Bytes) :
    merkleLayout (.byteVector data.size) (.bytes data) =
      merkleLayout (.vector (.uint 1) data.size) (.seq (byteValues data)) := by
  -- The byte count determines the same rounded-up node capacity on both paths.
  simp [merkleLayout, fixedLeaf, serialize, sequenceLayout,
    Desc.isBasic, Desc.itemLength, byteValues, serializeEach_bytes,
    packElements_bytes, chunksForBytes, packBytes_size, Bind.bind, Except.bind, pure, Except.pure]

/-- A bounded byte array and a list of bytes lay down the same nodes and length commitment. -/
theorem merkleLayout_byteList_alias (data : Bytes) (limit : Nat) :
    merkleLayout (.byteList limit) (.bytes data) =
      merkleLayout (.list (.uint 1) limit) (.seq (byteValues data)) := by
  -- Both the element count and the capacity count bytes, so even limit failures coincide.
  simp [merkleLayout, sequenceLayout, Desc.isBasic, Desc.itemLength,
    byteValues, serializeEach_bytes, packElements_bytes, Bind.bind, Except.bind, pure, Except.pure]

/-- Fixed byte arrays and byte vectors have identical Merkle roots for every payload. -/
theorem hashTreeRoot_byteVector_alias (data : Bytes) :
    hashTreeRoot (.byteVector data.size) (.bytes data) =
      hashTreeRoot (.vector (.uint 1) data.size) (.seq (byteValues data)) := by
  -- Packed leaves consume no recursive rooting budget, so the two type depths are immaterial.
  simp [hashTreeRoot, Desc.nesting, hashTreeRootAt, merkleLayout, fixedLeaf, serialize,
    sequenceLayout, Desc.isBasic, Desc.itemLength, byteValues, serializeEach_bytes,
    packElements_bytes, chunksForBytes, packBytes_size, layoutChunksAt,
    MerkleLayout.packing, Leaves.count, Bind.bind, Except.bind, pure, Except.pure]

/-- Bounded byte arrays and byte lists have identical roots and identical limit failures. -/
theorem hashTreeRoot_byteList_alias (data : Bytes) (limit : Nat) :
    hashTreeRoot (.byteList limit) (.bytes data) =
      hashTreeRoot (.list (.uint 1) limit) (.seq (byteValues data)) := by
  -- Both paths pack the same bytes and mix in the same element count.
  by_cases fits : data.size ≤ limit
  · simp [hashTreeRoot, Desc.nesting, hashTreeRootAt, merkleLayout, show ¬ limit < data.size from Nat.not_lt.mpr fits,
      sequenceLayout, Desc.isBasic, Desc.itemLength, byteValues, serializeEach_bytes,
      packElements_bytes, layoutChunksAt, MerkleLayout.packing, Leaves.count,
      Bind.bind, Except.bind, pure, Except.pure]
  · simp [hashTreeRoot, Desc.nesting, hashTreeRootAt, merkleLayout,
      show limit < data.size by omega, byteValues, Bind.bind, Except.bind]
    rfl

end Ssz
