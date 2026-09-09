import Ssz.Type.Value
import Ssz.Codec.Error

/-! Turning a value into bytes. -/

namespace Ssz

/--
An unsigned integer as little-endian bytes of the given width.

The low byte is written first, so each step drops the byte it just wrote.
Values outside the given width are truncated, so callers requiring exact recovery must enforce the range.
-/
def uintBytes : (width : Nat) → (value : Nat) → Bytes
  -- A width of no bytes writes nothing, whatever the value.
  | 0, _ => #[]
  -- Otherwise the low byte goes first, and the rest of the value follows it.
  | width + 1, value => #[UInt8.ofNat (value % 256)] ++ uintBytes width (value / 256)

/-- The eight bits that land in one byte, gathered into it. -/
def packByte (bits : Array Bool) (byteIndex : Nat) : UInt8 :=
  -- A byte begins at eight times its position, and offsets zero through seven select its bits.
  -- The eight bits of one byte are therefore the eight positions from its own start.
  let held (offset : Nat) : UInt8 :=
    -- An absent data bit contributes zero, which also supplies the high padding of the last byte.
    if bits[byteIndex * 8 + offset]?.getD false then (1 : UInt8) <<< UInt8.ofNat offset else 0
  -- The masks occupy disjoint positions, so bitwise union packs all eight bits without carries.
  held 0 ||| held 1 ||| held 2 ||| held 3 ||| held 4 ||| held 5 ||| held 6 ||| held 7

/--
Bits packed into bytes, the first bit lowest in the first byte.

    bits [1, 0, 1, 1]  ->  byte 0 = 0b00001101

Each byte is written from the bits that land in it, rather than the bits being written one at a time into a buffer.
A byte then stands on its own, which is what lets a bit be read back where it was put.
-/
def packBits (bits : Array Bool) (byteCount : Nat) : Bytes :=
  -- Construct each complete output byte from its own eight-bit window.
  Array.ofFn (n := byteCount) fun byteIndex => packByte bits byteIndex.val

/--
Bits packed into bytes, closed by a set bit one past the last of them.

The closing bit is what recovers the count on the way back.
An empty value therefore still takes one byte.
-/
def packBitsDelimited (bits : Array Bool) : Bytes :=
  -- The closing bit is one more bit, so the packing is the same with that bit on the end.
  -- One bit more than the data is carried, so the width follows from the count plus one.
  packBits (bits.push true) ((bits.size + 8) / 8)

/-- Bytes the fixed part occupies, before any body follows it. -/
def headWidth : List (Bool × Bytes) → Nat
  | [] => 0
  -- A part written in place takes its own width, and one that follows takes an offset.
  | (isInline, part) :: rest =>
      (if isInline then part.size else bytesPerOffset) + headWidth rest

/-- Bytes the bodies occupy, which is every part not written in place. -/
def bodyWidth : List (Bool × Bytes) → Nat
  | [] => 0
  -- Inline fields consume header space only.
  | (true, _) :: rest => bodyWidth rest
  -- Variable payloads are concatenated after the header, so their lengths add.
  | (false, part) :: rest => part.size + bodyWidth rest

/--
The fixed part, given where the first body begins.

An inline part sits here in full.
Anything else leaves the offset of its body, and the next body begins past its end.
-/
def headOf (start : Nat) : List (Bool × Bytes) → Bytes
  | [] => #[]
  -- An inline field keeps the next body position unchanged.
  | (true, part) :: rest => part ++ headOf start rest
  -- An offset names the current body position before advancing by that payload length.
  | (false, part) :: rest =>
      uintBytes bytesPerOffset start ++ headOf (start + part.size) rest

/-- The bodies, in the order their offsets name them. -/
def bodiesOf : List (Bool × Bytes) → Bytes
  | [] => #[]
  -- Inline bytes have already been written in the header.
  | (true, _) :: rest => bodiesOf rest
  -- Append variable payloads in field order so their starts match the stored offsets.
  | (false, part) :: rest => part ++ bodiesOf rest

/--
Parts laid out as a struct or a sequence encodes them.

Fixed parts sit in place, variable ones leave an offset behind and follow after.

    fields    u64      list     u8
    encoding  [8 bytes][offset ][1 byte][list body]
-/
def assemble (inline : List Bool) (parts : List Bytes) : Except Err Bytes := do
  -- Pair each payload with the declaration's choice of inline bytes or body offset.
  let slots := inline.zip parts
  -- Bodies start past the last slot, so the first offset is the width of everything before.
  let leading := headWidth slots
  -- Offsets only grow, so the end of the last body is the one that has to be nameable.
  -- An offset past what four bytes name would wrap, so it is refused instead.
  let total := leading + bodyWidth slots
  if total ≥ 2 ^ (8 * bytesPerOffset) then throw (.offsetOverflow total)
  -- The final encoding consists of the complete header followed by the ordered payload bodies.
  return headOf leading slots ++ bodiesOf slots

/-- The option a selector names, or a refusal when it names none. -/
def lookupOption : List Nat → List Desc → Nat → Except Err Desc
  | [], [], selector => .error (.unknownSelector selector)
  -- Selectors and option types advance together until the first matching entry.
  | chosen :: selectors, option :: options, selector =>
    if chosen == selector then .ok option else lookupOption selectors options selector
  -- Selectors and options that do not pair up name no union.
  | _, _, _ => .error .badDeclaration

mutual

/-- The SSZ encoding of a value, read against the type it is meant to fit. -/
def serialize : Desc → Value → Except Err Bytes
  | .bool, .bool b => .ok #[if b then 1 else 0]
  -- The integer range check prevents truncation by the fixed-width byte writer.
  | .uint width, .uint n =>
    if n < 2 ^ (8 * width) then .ok (uintBytes width n) else .error .typeMismatch
  -- A byte array is its own encoding, with only its count to check.
  | .byteVector length, .bytes data =>
    if data.size == length then .ok data else .error (.scope length data.size)
  | .byteList limit, .bytes data =>
    if data.size ≤ limit then .ok data else .error (.overLimit limit data.size)
  -- A fixed bit count needs no closing bit, since the width already gives it.
  | .bitVector length, .bits data =>
    if data.size == length then .ok (packBits data ((length + 7) / 8))
    else .error (.scope length data.size)
  -- Variable bit counts need a closing one bit so trailing zero data bits are preserved.
  | .bitList limit, .bits data =>
    if data.size ≤ limit then .ok (packBitsDelimited data)
    else .error (.overLimit limit data.size)
  | .progressiveBitList, .bits data => .ok (packBitsDelimited data)
  -- A vector checks the exact element count before using the common sequence layout.
  | .vector element length, .seq elements =>
    if elements.length == length then serializeSequence element elements
    else .error (.scope length elements.length)
  -- A list checks its capacity before laying out the element encodings.
  | .list element limit, .seq elements =>
    if elements.length ≤ limit then serializeSequence element elements
    else .error (.overLimit limit elements.length)
  -- Progressive lists share the sequence encoding without a declared count limit.
  | .progressiveList element, .seq elements => serializeSequence element elements
  | .container _ fields, .seq values => serializeStruct fields values
  -- A gap holds no field, so a struct encodes exactly the fields it declares.
  | .progressiveContainer _ _ fields, .seq values => serializeStruct fields values
  | .compatibleUnion selectors options, .union selector data => do
    -- The declared selector identifies the payload type before any payload bytes are written.
    let option ← lookupOption selectors options selector
    let body ← serialize option data
    -- One byte of selector comes first, and the option's own encoding follows.
    return #[UInt8.ofNat selector] ++ body
  | _, _ => .error .typeMismatch

/-- The encoding of a sequence whose elements all share one type. -/
def serializeSequence (element : Desc) (elements : List Value) : Except Err Bytes := do
  -- Encode every element before assigning offsets, since payload lengths determine body positions.
  let parts ← serializeEach element elements
  -- Fixed elements need no table, since the count follows from the width.
  assemble (parts.map fun _ => element.isFixed) parts

/-- The encoding of a struct, each field read against its own type. -/
def serializeStruct (fields : List Desc) (values : List Value) : Except Err Bytes := do
  -- Each field is encoded under its own declaration before the mixed header is assembled.
  let parts ← serializeFields fields values
  -- Fixed fields remain inline and variable fields contribute four-byte body references.
  assemble (fields.map Desc.isFixed) parts

/-- Each element of a sequence encoded on its own. -/
def serializeEach (element : Desc) : List Value → Except Err (List Bytes)
  -- An empty sequence encodes to no parts at all.
  | [] => .ok []
  | value :: rest => do
    -- Every element is encoded against the one type they all share.
    let head ← serialize element value
    let tail ← serializeEach element rest
    return head :: tail

/-- Each field of a struct encoded on its own, paired with the type it was declared as. -/
def serializeFields : List Desc → List Value → Except Err (List Bytes)
  -- Both lists run out together, which is what pairing one to one means.
  | [], [] => .ok []
  | field :: fields, value :: values => do
    -- Each value is encoded against the type its own field was declared as.
    let head ← serialize field value
    let tail ← serializeFields fields values
    return head :: tail
  -- A struct given a different number of values than it has fields fits no type.
  | _, _ => .error .typeMismatch

end

end Ssz
