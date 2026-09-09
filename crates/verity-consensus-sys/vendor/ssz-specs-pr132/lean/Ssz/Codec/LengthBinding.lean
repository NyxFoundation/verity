import Ssz.Merkle.Widths

/-! Count and layout words preserve their information within their stated widths. -/

namespace Ssz

/-- Equal fixed-width integer encodings contain the same integer when both values fit. -/
theorem uintBytes_injective {width left right : Nat}
    (leftFits : left < 2 ^ (8 * width)) (rightFits : right < 2 ^ (8 * width))
    (same : uintBytes width left = uintBytes width right) : left = right := by
  -- Reading the common byte string recovers each bounded integer exactly.
  have read := congrArg (fun bytes => readUint bytes 0 width) same
  simpa only [readUint_uintBytes width left leftFits, readUint_uintBytes width right rightFits] using read

/-- A 256-bit length word distinguishes every count that fits the SSZ mixing field. -/
theorem lengthWord_injective {left right : Nat}
    (leftFits : left < 2 ^ 256) (rightFits : right < 2 ^ 256)
    (same : lengthWord left = lengthWord right) : left = right := by
  -- A length word is a 32-byte unsigned encoding, so every count below 2^256 is recovered exactly.
  exact uintBytes_injective leftFits rightFits same

/-- Equal layout words preserve every active position within the declared common width. -/
theorem activeFieldsWord_injective {left right : List Bool}
    (sameLength : left.length = right.length) (bounded : left.length ≤ bitsPerChunk)
    (same : activeFieldsWord left = activeFieldsWord right) : left = right := by
  -- Equal words give equal bits, including clear positions between occupied fields.
  apply List.ext_getElem sameLength
  intro position insideLeft insideRight
  -- The common mask length places every requested position within the 256 available bits.
  have inside : position < bitsPerChunk := by omega
  -- Select the containing byte, shift the requested bit to the low position, and mask away the other bits.
  have bit := congrArg
    (fun bytes : Bytes => (bytes[position / 8]! >>> UInt8.ofNat (position % 8)) &&& 1 == 1) same
  -- The packed bit at that position is exactly the corresponding active-field flag.
  rw [activeFieldsWord_bit left position inside, activeFieldsWord_bit right position inside] at bit
  simpa only [List.getElem?_eq_getElem insideLeft, List.getElem?_eq_getElem insideRight,
    Option.getD_some] using bit

end Ssz
