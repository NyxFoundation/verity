import Ssz.Type.Desc

/-! Values, and what it means for one to fit a type. -/

namespace Ssz

/--
An SSZ value, carrying no type of its own.

A value is read against a type rather than being one.
A decoder can then hand back what it read and have the result checked afterwards.
-/
inductive Value where
  /-- A boolean. -/
  | bool (b : Bool)
  /-- An unsigned integer, whose width comes from the type it is read against. -/
  | uint (n : Nat)
  /-- Opaque bytes, for the two byte-array shapes. -/
  | bytes (data : Array UInt8)
  /-- Bits, for the three bitfield shapes. -/
  | bits (data : Array Bool)
  /-- Elements in order, for the sequences and for the structs. -/
  | seq (elements : List Value)
  /-- One option of a union, under the selector it was chosen by. -/
  | union (selector : Nat) (data : Value)
  deriving Repr, Inhabited

mutual

/-- Structural equality, descending into nested components. -/
def Value.beq : Value → Value → Bool
  -- Matching constructors compare their stored data, including every nested component.
  | .bool a, .bool b => a == b
  | .uint a, .uint b => a == b
  | .bytes a, .bytes b => a == b
  | .bits a, .bits b => a == b
  | .seq a, .seq b => Value.beqList a b
  | .union a x, .union b y => a == b && Value.beq x y
  -- Different constructors describe different objects, whatever their contents.
  | _, _ => false

/-- Pairwise equality of nested components, preserving their order and count. -/
def Value.beqList : List Value → List Value → Bool
  -- Lists agree only when both end together and each corresponding pair agrees.
  | [], [] => true
  | x :: xs, y :: ys => Value.beq x y && Value.beqList xs ys
  | _, _ => false

end

-- Equality compares the full value, including every sequence element and selected union payload.
instance : BEq Value := ⟨Value.beq⟩

mutual

/--
Whether a value fits a type.

Serialization assumes this, and deserialization establishes it.
-/
def Value.fits : Desc → Value → Bool
  | .bool, .bool _ => true
  -- An integer fits when it lies below what its declared width can hold.
  | .uint width, .uint n => n < 2 ^ (8 * width)
  -- Fixed arrays require the declared count, while bounded arrays allow any shorter payload.
  | .byteVector length, .bytes data => data.size == length
  | .byteList limit, .bytes data => data.size ≤ limit
  | .bitVector length, .bits data => data.size == length
  | .bitList limit, .bits data => data.size ≤ limit
  -- No limit bounds a progressive shape, so any count fits.
  | .progressiveBitList, .bits _ => true
  -- A sequence must satisfy both its count rule and every element’s own domain.
  | .vector element length, .seq elements =>
      elements.length == length && Value.allFit element elements
  | .list element limit, .seq elements =>
      elements.length ≤ limit && Value.allFit element elements
  | .progressiveList element, .seq elements => Value.allFit element elements
  | .container _ fields, .seq elements => Value.fieldsFit fields elements
  -- A struct holds one value per declared field, never one per layout position.
  | .progressiveContainer _ _ fields, .seq elements => Value.fieldsFit fields elements
  -- The selected option supplies the payload type.
  -- Declaration validity checks selector legality.
  | .compatibleUnion selectors options, .union selector data =>
      Value.optionFits selectors options selector data
  | _, _ => false

/-- Whether every element of a sequence fits the one element type. -/
def Value.allFit (element : Desc) : List Value → Bool
  | [] => true
  -- No element can be skipped, including after an earlier successful check.
  | value :: rest => Value.fits element value && Value.allFit element rest

/-- Whether a struct's values pair one to one with its fields, each fitting its own. -/
def Value.fieldsFit : List Desc → List Value → Bool
  | [], [] => true
  -- Fields and values advance together, so either unmatched suffix is rejected.
  | field :: fields, value :: values =>
    Value.fits field value && Value.fieldsFit fields values
  | _, _ => false

/-- Whether a union's value fits the option its selector names. -/
def Value.optionFits : List Nat → List Desc → Nat → Value → Bool
  | [], [], _, _ => false
  | chosen :: selectors, option :: options, selector, data =>
      -- Only the matching option checks the payload.
      -- Unmatched selectors continue in declaration order.
      if chosen == selector then Value.fits option data
      else Value.optionFits selectors options selector data
  -- A selector list and an option list of different lengths name no type at all.
  | _, _, _, _ => false

end

end Ssz
