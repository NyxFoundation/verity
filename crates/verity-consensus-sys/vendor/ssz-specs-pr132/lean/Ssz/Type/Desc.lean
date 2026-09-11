/-! The SSZ type universe, written as data. -/

namespace Ssz

/-- Width of the offset an encoding puts in front of each variable-size part. -/
abbrev bytesPerOffset : Nat := 4

/--
An SSZ type.

Types are data here rather than Lean types.

One definition of serialization or of merkleization then covers the whole universe.
-/
inductive Desc where
  /-- A true or false value. -/
  | bool
  /-- An unsigned integer of the given width in bytes. -/
  | uint (byteWidth : Nat)
  /-- A fixed number of opaque bytes. -/
  | byteVector (length : Nat)
  /-- A variable number of opaque bytes, up to a limit. -/
  | byteList (limit : Nat)
  /-- A fixed number of bits. -/
  | bitVector (length : Nat)
  /-- A variable number of bits, up to a limit. -/
  | bitList (limit : Nat)
  /-- A variable number of bits with no limit. -/
  | progressiveBitList
  /-- A fixed number of elements. -/
  | vector (element : Desc) (length : Nat)
  /-- A variable number of elements, up to a limit. -/
  | list (element : Desc) (limit : Nat)
  /-- A variable number of elements with no limit. -/
  | progressiveList (element : Desc)
  /-- A fixed sequence of named fields. -/
  | container (names : List String) (fields : List Desc)
  /-- Named fields that keep their tree positions as the set of them changes. -/
  | progressiveContainer (activeFields : List Bool) (names : List String) (fields : List Desc)
  /-- A choice between options that share one tree shape. -/
  | compatibleUnion (selectors : List Nat) (options : List Desc)
  deriving Repr, Inhabited

mutual

/-- Structural equality, descending into nested components. -/
def Desc.beq : Desc → Desc → Bool
  -- Matching constructors compare their stored data, including every nested component.
  | .bool, .bool => true
  | .uint a, .uint b => a == b
  | .byteVector a, .byteVector b => a == b
  | .byteList a, .byteList b => a == b
  | .bitVector a, .bitVector b => a == b
  | .bitList a, .bitList b => a == b
  | .progressiveBitList, .progressiveBitList => true
  | .vector a n, .vector b m => Desc.beq a b && n == m
  | .list a n, .list b m => Desc.beq a b && n == m
  | .progressiveList a, .progressiveList b => Desc.beq a b
  | .container names fields, .container others types =>
      names == others && Desc.beqList fields types
  | .progressiveContainer active names fields, .progressiveContainer mask others types =>
      active == mask && names == others && Desc.beqList fields types
  | .compatibleUnion selectors options, .compatibleUnion others types =>
      selectors == others && Desc.beqList options types
  -- Different constructors describe different objects, whatever their contents.
  | _, _ => false

/-- Pairwise equality of nested components, preserving their order and count. -/
def Desc.beqList : List Desc → List Desc → Bool
  -- Lists agree only when both end together and each corresponding pair agrees.
  -- Both component lists must end together, so extra fields cannot compare equal.
  | [], [] => true
  | x :: xs, y :: ys => Desc.beq x y && Desc.beqList xs ys
  | _, _ => false

end

-- Boolean equality follows the complete nested declaration, including names and selectors.
instance : BEq Desc := ⟨Desc.beq⟩

mutual

/--
Encoded width in bytes, where the type has one.

Nothing is returned for a type whose encoding varies with the value it holds.
-/
def Desc.fixedSize : Desc → Option Nat
  -- One byte carries a boolean, and its width is fixed by the encoding, not by the value.
  | .bool => some 1
  | .uint width => some width
  | .byteVector length => some length
  -- A fixed bit count still takes whole bytes, so the last one may be part padding.
  | .bitVector length => some ((length + 7) / 8)
  -- A vector is fixed exactly when its element is.
  -- It then holds that width once per element.
  | .vector element length => element.fixedSize.map (· * length)
  | .container _ fields => Desc.fieldsFixedSize fields
  -- A gap contributes no bytes, so only the declared fields are measured.
  | .progressiveContainer _ _ fields => Desc.fieldsFixedSize fields
  -- Everything else carries a count that the encoding does not fix in advance.
  | _ => none

/-- Total width of a field list, where every field has one. -/
def Desc.fieldsFixedSize : List Desc → Option Nat
  -- An empty field suffix contributes no additional encoded bytes.
  | [] => some 0
  | field :: rest =>
    match field.fixedSize, Desc.fieldsFixedSize rest with
    -- Fixed fields concatenate without offsets, so their byte counts add.
    | some width, some remaining => some (width + remaining)
    -- One variable field is enough to leave the whole struct variable.
    | _, _ => none

end

mutual

/--
Levels of type nesting below this one.

Bounds how far a path can descend.
Each level of a path enters one nested type.
-/
def Desc.nesting : Desc → Nat
  -- A sequence adds one level above the type of its elements.
  | .vector element _ => element.nesting + 1
  | .list element _ => element.nesting + 1
  | .progressiveList element => element.nesting + 1
  -- A field path can be only as deep as the deepest declared field.
  | .container _ fields => Desc.deepestNesting fields + 1
  | .progressiveContainer _ _ fields => Desc.deepestNesting fields + 1
  | .compatibleUnion _ options => Desc.deepestNesting options + 1
  -- A basic value and a packed one hold no type below them.
  | _ => 1

/-- The deepest nesting among a list of types. -/
def Desc.deepestNesting : List Desc → Nat
  | [] => 0
  -- The maximum covers every possible child selected by a later path.
  | shape :: rest => max shape.nesting (Desc.deepestNesting rest)

end

/--
The six unsigned integers SSZ defines, named as the specification names them.

Widths in declarations are byte counts.
Eight bytes therefore describe a sixty-four-bit integer.
The named widths use bit counts to match the SSZ specification.
-/
def Desc.uint8 : Desc := .uint 1

@[inherit_doc Desc.uint8] def Desc.uint16 : Desc := .uint 2

@[inherit_doc Desc.uint8] def Desc.uint32 : Desc := .uint 4

@[inherit_doc Desc.uint8] def Desc.uint64 : Desc := .uint 8

@[inherit_doc Desc.uint8] def Desc.uint128 : Desc := .uint 16

@[inherit_doc Desc.uint8] def Desc.uint256 : Desc := .uint 32

/-- Whether the type encodes to the same width whatever value it holds. -/
def Desc.isFixed (d : Desc) : Bool := d.fixedSize.isSome

/--
Width of the part of a struct's encoding that comes before the bodies.

A fixed field sits there whole, and a variable one leaves an offset in its place.
-/
def Desc.leadingWidth (fields : List Desc) : Nat :=
  -- Variable fields contribute four-byte offsets instead of their later bodies.
  fields.foldl (fun total field => total + (field.fixedSize.getD bytesPerOffset)) 0

end Ssz
