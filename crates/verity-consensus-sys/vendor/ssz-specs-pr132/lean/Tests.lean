import Ssz

/-! Regression checks independent of the Python SSZ implementation. -/

open Ssz

private def expect (label : String) (holds : Bool) : IO Unit := do
  -- A failed invariant stops the executable with a useful case name.
  unless holds do throw (IO.userError label)

private def checkCanonical (shape : Desc) (bytes : Bytes) : IO Unit := do
  -- Every accepted encoding must be reproduced exactly, including offsets and padding.
  match deserialize shape bytes with
  | .error _ => pure ()
  | .ok value => expect "accepted bytes have a different encoding" ((serialize shape value).toOption == some bytes)

private def checkBranches (shape : Desc) (value : Value) : IO Unit := do
  -- Authenticate all readable nodes in a finite prefix, including padding and mixed-in words.
  let .ok root := hashTreeRoot shape value | throw (IO.userError "fixture has no root")
  for index in [2:128] do
    match nodeRoot shape value index, buildProof shape value index with
    -- A readable node and its generated siblings must rebuild the same value root.
    | .ok leaf, .ok branch =>
      expect s!"branch at {index}" ((verifyMerkleProof leaf branch index root).toOption == some true)
    -- Proof construction must succeed whenever the corresponding node is readable.
    | .ok _, .error _ => throw (IO.userError s!"readable node has no branch at {index}")
    | .error _, _ => pure ()

-- A finite tree satisfies the branch premise without assigning children to its leaves.
example (left right : Bytes) :
    BranchConsistent (fun index => if index = 1 then combine left right
      else if index = 2 then left else right) 2 := by
  -- Only the single parent equation at level zero is required.
  intro level below
  have levelZero : level = 0 := by have : level < 1 := below; omega
  subst level
  rfl

def main : IO Unit := do
  -- A splice cannot turn a nonexistent outer node into a valid index.
  expect "zero outer index" ((gindexConcat 0 3).isOk == false)
  -- Field names and selectors are declaration keys, so duplicates are invalid.
  expect "duplicate field names"
    ((Desc.container ["flag", "flag"] [.bool, .bool]).wellFormed.isOk == false)
  -- The same uniqueness requirement applies when fields occupy progressive positions.
  expect "duplicate progressive field names"
    ((Desc.progressiveContainer [true, true] ["flag", "flag"] [.bool, .bool]).wellFormed.isOk == false)
  -- Repeated tags must report the duplicated selector rather than select an arbitrary option.
  expect "duplicate selector diagnostic"
    (match (Desc.compatibleUnion [7, 7] [.bool, .bool]).wellFormed with
    | .error (.unionSelectorRepeated 7) => true
    | _ => false)
  -- One active field permits one value, even when unused positions precede it.
  let shape := Desc.progressiveContainer [false, true] ["flag"] [.bool]
  -- Fixture state: one active position accepts one field value, while two values must fail.
  let extra := Value.seq [.bool false, .bool true]
  expect "extra progressive field" ((hashTreeRoot shape extra).isOk == false)
  -- Removing the sole value must also fail, even though the layout contains an inactive gap.
  expect "missing progressive field" ((hashTreeRoot shape (.seq [])).isOk == false)
  -- The matching one-value payload establishes the positive case for the same declaration.
  expect "valid progressive field" ((hashTreeRoot shape (.seq [.bool true])).isOk)
  -- An empty tree deeper than the cache still has a hashed zero root.
  expect "depth 65 zero root" ((merkleizeBounded #[] (some (2 ^ 65))).toOption == some #[
    0x30, 0x3c, 0xe3, 0x88, 0x09, 0xba, 0x7a, 0x77, 0xb6, 0x60, 0xad, 0x0b, 0x07, 0x4a, 0xf9, 0xc6,
    0xbc, 0xd5, 0xc0, 0x2b, 0xbf, 0xf2, 0xf3, 0xb0, 0x24, 0x86, 0x33, 0xb0, 0xb8, 0x76, 0xe4, 0x49])
  -- SHA-256 padding changes block count at 56 bytes and crosses a block at 64.
  -- An empty message still hashes one padding block with an explicit zero bit length.
  expect "SHA-256 0 bytes" ((Sha256.hash ⟨(Array.range 0).map UInt8.ofNat⟩).data == #[
    0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
    0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55])
  -- The standard three-byte message checks a familiar digest independently of SSZ.
  expect "SHA-256 3 bytes" ((Sha256.hash ⟨#[0x61, 0x62, 0x63]⟩).data == #[
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad])
  -- Fifty-five payload bytes leave exactly room for the delimiter and eight-byte length.
  expect "SHA-256 55 bytes" ((Sha256.hash ⟨(Array.range 55).map UInt8.ofNat⟩).data == #[
    0x46, 0x3e, 0xb2, 0x8e, 0x72, 0xf8, 0x2e, 0x0a, 0x96, 0xc0, 0xa4, 0xcc, 0x53, 0x69, 0x0c, 0x57,
    0x12, 0x81, 0x13, 0x1f, 0x67, 0x2a, 0xa2, 0x29, 0xe0, 0xd4, 0x5a, 0xe5, 0x9b, 0x59, 0x8b, 0x59])
  -- One additional payload byte forces the length field into a second block.
  expect "SHA-256 56 bytes" ((Sha256.hash ⟨(Array.range 56).map UInt8.ofNat⟩).data == #[
    0xda, 0x2a, 0xe4, 0xd6, 0xb3, 0x67, 0x48, 0xf2, 0xa3, 0x18, 0xf2, 0x3e, 0x7a, 0xb1, 0xdf, 0xdf,
    0x45, 0xac, 0xdc, 0x9d, 0x04, 0x9b, 0xd8, 0x0e, 0x59, 0xde, 0x82, 0xa6, 0x08, 0x95, 0xf5, 0x62])
  -- At sixty-three bytes, the delimiter fills the first block’s last position.
  expect "SHA-256 63 bytes" ((Sha256.hash ⟨(Array.range 63).map UInt8.ofNat⟩).data == #[
    0x29, 0xaf, 0x26, 0x86, 0xfd, 0x53, 0x37, 0x4a, 0x36, 0xb0, 0x84, 0x66, 0x94, 0xcc, 0x34, 0x21,
    0x77, 0xe4, 0x28, 0xd1, 0x64, 0x75, 0x15, 0xf0, 0x78, 0x78, 0x4d, 0x69, 0xcd, 0xb9, 0xe4, 0x88])
  -- A complete payload block requires a separate padding block.
  expect "SHA-256 64 bytes" ((Sha256.hash ⟨(Array.range 64).map UInt8.ofNat⟩).data == #[
    0xfd, 0xea, 0xb9, 0xac, 0xf3, 0x71, 0x03, 0x62, 0xbd, 0x26, 0x58, 0xcd, 0xc9, 0xa2, 0x9e, 0x8f,
    0x9c, 0x75, 0x7f, 0xcf, 0x98, 0x11, 0x60, 0x3a, 0x8c, 0x44, 0x7c, 0xd1, 0xd9, 0x15, 0x11, 0x08])
  -- The next payload byte belongs to the second block before its padding is appended.
  expect "SHA-256 65 bytes" ((Sha256.hash ⟨(Array.range 65).map UInt8.ofNat⟩).data == #[
    0x4b, 0xfd, 0x2c, 0x8b, 0x6f, 0x1e, 0xec, 0x7a, 0x2a, 0xfe, 0xb4, 0x8b, 0x93, 0x4e, 0xe4, 0xb2,
    0x69, 0x41, 0x82, 0x02, 0x7e, 0x6d, 0x0f, 0xc0, 0x75, 0x07, 0x4f, 0x2f, 0xab, 0xb3, 0x17, 0x81])
  -- The same padding threshold repeats after one complete block of payload.
  expect "SHA-256 119 bytes" ((Sha256.hash ⟨(Array.range 119).map UInt8.ofNat⟩).data == #[
    0xda, 0x18, 0x79, 0x7e, 0xd7, 0xc3, 0xa7, 0x77, 0xf0, 0x84, 0x7f, 0x42, 0x97, 0x24, 0xa2, 0xd8,
    0xcd, 0x51, 0x38, 0xe6, 0xed, 0x28, 0x95, 0xc3, 0xfa, 0x1a, 0x6d, 0x39, 0xd1, 0x8f, 0x7e, 0xc6])
  -- Crossing that threshold forces a third compression block.
  expect "SHA-256 120 bytes" ((Sha256.hash ⟨(Array.range 120).map UInt8.ofNat⟩).data == #[
    0xf5, 0x2b, 0x23, 0xdb, 0x1f, 0xbb, 0x6d, 0xed, 0x89, 0xef, 0x42, 0xa2, 0x3c, 0xe0, 0xc8, 0x92,
    0x2c, 0x45, 0xf2, 0x5c, 0x50, 0xb5, 0x68, 0xa9, 0x3b, 0xf1, 0xc0, 0x75, 0x42, 0x0b, 0xbb, 0x7c])
  -- Shifting one byte across a child boundary preserves concatenation but violates SSZ.
  -- Mutation: two 32-byte children become lengths 31 and 33 while their concatenation stays 64 bytes.
  let short := Array.replicate 31 (0 : UInt8)
  let long := Array.replicate 33 (0 : UInt8)
  let root := combine zeroChunk zeroChunk
  expect "short proof leaf" ((verifyMerkleProof short [long] 2 root).isOk == false)
  -- The shared-proof verifier must reject the same ambiguous child boundary.
  expect "short multiproof leaf"
    ((verifyMerkleMultiproof [short] [long] [2] root).isOk == false)
  -- Both tree shapes and nested values must authenticate their readable nodes.
  checkBranches (.list .uint256 5) (.seq [.uint 1, .uint 2, .uint 3])
  -- An unbounded progressive spine must authenticate the same readable-node prefix.
  checkBranches (.progressiveList .uint256) (.seq [.uint 1, .uint 2, .uint 3])
  -- Gaps in a progressive container remain part of the authenticated tree layout.
  checkBranches shape (.seq [.bool true])
  -- A selected nested option must remain authenticated below its selector word.
  checkBranches (.compatibleUnion [1] [shape]) (.union 1 (.seq [.bool true]))
  -- Exhaust all one- and two-byte inputs for small packed and delimited types.
  let shapes : List Desc := [.bool, .uint8, .uint16, .byteVector 2, .byteList 2,
    .bitVector 9, .bitList 9, .progressiveBitList, .list .bool 2,
    .vector .bool 2, .progressiveList .bool, .compatibleUnion [1] [.bool]]
  for shape in shapes do
    -- The empty byte string exercises rejection and empty-value encodings separately for each type.
    checkCanonical shape #[]
    -- Every possible first byte includes valid tags, invalid tags, and delimiter positions.
    for first in [0:256] do
      checkCanonical shape #[UInt8.ofNat first]
      -- Every possible second byte also tests partial final bytes and trailing-data rejection.
      for second in [0:256] do
        checkCanonical shape #[UInt8.ofNat first, UInt8.ofNat second]
  -- Equal offsets encode empty bodies, while a gap before the first body is invalid.
  let lists := Desc.list (.byteList 2) 2
  checkCanonical lists #[8, 0, 0, 0, 8, 0, 0, 0]
  -- A first offset of five leaves one unused byte after a four-byte header and must be rejected.
  expect "offset gap" ((deserialize (.container ["items"] [.byteList 2])
    #[5, 0, 0, 0, 0]).isOk == false)
  IO.println "Lean regressions passed"
