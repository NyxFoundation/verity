import Ssz.Type.Value
import Ssz.Codec.Error

/-! The value a type takes when nothing is supplied, and how to tell one. -/

namespace Ssz

mutual

/--
The default value of a type, which every type but one has.

    unsigned integer, boolean          zero, false
    fixed byte array                   every byte zero
    bitvector                          every bit clear
    vector                             the element default, once per position
    container, progressive container   one field default per field
    list, bitlist, progressive lists    empty
    compatible union                   none, since no option is the one to take

A composite builds from its parts, so a part with no default leaves it none.
-/
def Desc.default : Desc → Except Err Value
  -- Scalar defaults are false or zero, independently of their encoded width.
  | .bool => .ok (.bool false)
  | .uint _ => .ok (.uint 0)
  -- Fixed arrays preserve every declared position, filling it with zero.
  | .byteVector length => .ok (.bytes (Array.replicate length 0))
  | .bitVector length => .ok (.bits (Array.replicate length false))
  -- A bounded or unbounded sequence holds nothing until something is put in it.
  | .byteList _ => .ok (.bytes #[])
  | .bitList _ => .ok (.bits #[])
  | .progressiveBitList => .ok (.bits #[])
  | .list _ _ => .ok (.seq [])
  | .progressiveList _ => .ok (.seq [])
  | .vector element length => do
    -- Every position holds the element's own default, so the whole is built once.
    let one ← element.default
    return .seq (List.replicate length one)
  | .container _ fields => return .seq (← Desc.defaultFields fields)
  | .progressiveContainer _ _ fields => return .seq (← Desc.defaultFields fields)
  -- A union would have to pick an option, and no option is the one to pick.
  | .compatibleUnion _ _ => .error .typeMismatch

/-- The default of each field of a struct. -/
def Desc.defaultFields : List Desc → Except Err (List Value)
  -- A struct of no fields holds nothing to default.
  | [] => .ok []
  | field :: rest => do
    -- Each field takes its own default, so a field with none leaves the struct with none.
    let head ← field.default
    -- Every later field must also have a default before the container can be constructed.
    let tail ← Desc.defaultFields rest
    return head :: tail

end

/-- Whether a value is the default of its own type, which the specification calls zeroed. -/
def Desc.isZero (shape : Desc) (value : Value) : Except Err Bool := do
  -- A type without a default reports an error rather than classifying its value as nonzero.
  return (← shape.default) == value

end Ssz
