import Ssz.Codec.Serialize

/-! What it means for a value to be one its type admits. -/

namespace Ssz

/--
Values a type admits.

An encoder may assume this domain, and a decoder must establish it.
It records value shape, scalar ranges, and declared collection capacities.

- Composite byte-size limits are separate requirements for successful serialization.
- Exact 256-bit count words are separate requirements for commitment binding.
-/
inductive Fits : Desc → Value → Prop
  /-- Either boolean fits. -/
  | bool (b : Bool) : Fits .bool (.bool b)
  /-- An integer fits a width that holds it. -/
  | uint {width n : Nat} (bound : n < 2 ^ (8 * width)) : Fits (.uint width) (.uint n)
  /-- A byte string of exactly the declared count. -/
  | byteVector {length : Nat} {data : Bytes} (exact : data.size = length) :
      Fits (.byteVector length) (.bytes data)
  /-- A byte string within the declared capacity. -/
  | byteList {limit : Nat} {data : Bytes} (within : data.size ≤ limit) :
      Fits (.byteList limit) (.bytes data)
  /-- A bit sequence of exactly the declared count. -/
  | bitVector {length : Nat} {data : Array Bool} (exact : data.size = length) :
      Fits (.bitVector length) (.bits data)
  /-- A bit sequence within the declared capacity. -/
  | bitList {limit : Nat} {data : Array Bool} (within : data.size ≤ limit) :
      Fits (.bitList limit) (.bits data)
  /-- A bit sequence of any length, no capacity being declared. -/
  | progressiveBitList {data : Array Bool} : Fits .progressiveBitList (.bits data)
  /-- Exactly the declared count of elements, each fitting the element type. -/
  | vector {element : Desc} {length : Nat} {elements : List Value}
      (count : elements.length = length)
      (each : ∀ value ∈ elements, Fits element value) :
      Fits (.vector element length) (.seq elements)
  /-- Elements within the declared capacity, each fitting the element type. -/
  | list {element : Desc} {limit : Nat} {elements : List Value}
      (within : elements.length ≤ limit)
      (each : ∀ value ∈ elements, Fits element value) :
      Fits (.list element limit) (.seq elements)
  /-- Elements of any count, no capacity being declared. -/
  | progressiveList {element : Desc} {elements : List Value}
      (each : ∀ value ∈ elements, Fits element value) :
      Fits (.progressiveList element) (.seq elements)
  /-- One value per declared field, each fitting the field it stands under. -/
  | container {names : List String} {fields : List Desc} {values : List Value}
      (paired : fields.length = values.length)
      (each : ∀ pair ∈ fields.zip values, Fits pair.1 pair.2) :
      Fits (.container names fields) (.seq values)
  /-- One value per declared field, the layout naming where each of them sits. -/
  | progressiveContainer {active : List Bool} {names : List String} {fields : List Desc}
      {values : List Value}
      (paired : fields.length = values.length)
      (each : ∀ pair ∈ fields.zip values, Fits pair.1 pair.2) :
      Fits (.progressiveContainer active names fields) (.seq values)
  /-- The value of an option the union declares, under the selector that names it. -/
  | compatibleUnion {selectors : List Nat} {options : List Desc} {selector : Nat}
      {option : Desc} {data : Value}
      (bounded : selector < 256)
      (named : lookupOption selectors options selector = .ok option)
      (inner : Fits option data) :
      Fits (.compatibleUnion selectors options) (.union selector data)

end Ssz
