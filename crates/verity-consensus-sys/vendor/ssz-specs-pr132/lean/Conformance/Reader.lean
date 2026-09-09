import Ssz
import Lean.Data.Json

/-! Reading a fixture's JSON into a value of the type it names. -/

namespace Conformance

open Ssz Lean

/-- One hexadecimal digit as its value. -/
def hexDigit (c : Char) : Except String Nat :=
  -- Decimal digits and either letter case denote the same sixteen hexadecimal values.
  if '0' ≤ c && c ≤ '9' then .ok (c.toNat - '0'.toNat)
  else if 'a' ≤ c && c ≤ 'f' then .ok (c.toNat - 'a'.toNat + 10)
  else if 'A' ≤ c && c ≤ 'F' then .ok (c.toNat - 'A'.toNat + 10)
  else .error s!"not a hexadecimal digit: {c}"

/-- Bytes from a hexadecimal string, with or without the leading marker. -/
def fromHex (text : String) : Except String Bytes := do
  let characters := text.toList
  -- The marker is optional, and the fixtures write it on every hexadecimal field.
  let digits := if characters.take 2 == ['0', 'x'] then characters.drop 2 else characters
  -- An odd digit count cannot describe a whole number of bytes.
  if digits.length % 2 != 0 then throw s!"odd number of hexadecimal digits: {text}"
  let mut out : Bytes := #[]
  let mut rest := digits
  -- Two digits make a byte, the first of them the high half.
  while !rest.isEmpty do
    let high ← hexDigit rest[0]!
    let low ← hexDigit rest[1]!
    -- The high nibble contributes sixteen times its value, followed by the low nibble.
    out := out.push (UInt8.ofNat (16 * high + low))
    rest := rest.drop 2
  return out

/-- A hexadecimal string for bytes, as the fixtures write them. -/
def toHex (data : Bytes) : String :=
  data.foldl (fun text byte =>
    -- Each byte prints as two hexadecimal digits, including a leading zero below sixteen.
    let digits := Nat.toDigits 16 byte.toNat
    text ++ (if byte.toNat < 16 then "0" else "") ++ String.ofList digits) "0x"

/-- The value a payload holds, read against the type it is meant to fit. -/
partial def readValue : Desc → Json → Except String Value
  | .bool, json => return .bool (← json.getBool?)
  -- An integer is written as a decimal string on its own, and as a number inside a shape.
  | .uint _, json =>
    match json.getStr? with
    | .ok text =>
      match text.toNat? with
      | some n => return .uint n
      | none => throw s!"not a number: {text}"
    | .error _ => return .uint (← json.getNat?)
  -- A byte vector is its own hexadecimal string, with nothing wrapping it.
  | .byteVector _, json => return .bytes (← fromHex (← json.getStr?))
  | .byteList _, json =>
    return .bytes (← fromHex (← (← json.getObjVal? "data").getStr?))
  -- Bit fixtures carry explicit booleans, preserving their count before SSZ packing.
  | .bitVector _, json | .bitList _, json | .progressiveBitList, json => do
    let entries ← (← json.getObjVal? "data").getArr?
    return .bits (← entries.mapM Json.getBool?)
  -- Every sequence entry is read against the same declared element type.
  | .vector element _, json | .list element _, json | .progressiveList element, json => do
    let entries ← (← json.getObjVal? "data").getArr?
    return .seq (← entries.toList.mapM (readValue element))
  -- Fields are read in declaration order, which is the order SSZ encodes them in.
  | .container names fields, json | .progressiveContainer _ names fields, json => do
    return .seq (← (names.zip fields).mapM fun (name, field) => do
      readValue field (← json.getObjVal? name))
  | .compatibleUnion selectors options, json => do
    -- The selector identifies the option type needed to interpret the union payload.
    let selector ← (← json.getObjVal? "selector").getNat?
    let body ← json.getObjVal? "data"
    -- An absent selector is rejected rather than assigned an arbitrary option.
    match (selectors.zip options).find? fun (chosen, _) => chosen == selector with
    | some (_, option) => return .union selector (← readValue option body)
    | none => throw s!"no option for selector {selector}"

end Conformance
