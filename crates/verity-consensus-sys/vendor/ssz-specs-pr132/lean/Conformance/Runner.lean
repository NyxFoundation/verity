import Conformance.Registry

/-! Running the reference tests the Python specification generates. -/

namespace Conformance

open Ssz Lean

/-- A refusal, as a line of text. -/
def describe (fault : Err) : String := toString (repr fault)

/--
What one fixture asserts.

An encoding fixture pins the bytes, the root, and the value they decode back to.
A refusal fixture pins only that the bytes are turned away.
-/
def checkFixture (moduleName : String) (entry : Json) : Except String Unit := do
  -- The fixture’s module and type name select the declaration used for every check.
  let typeName ← (← entry.getObjVal? "typeName").getStr?
  let some shape := lookupShape moduleName typeName
    | throw s!"no type registered for {moduleName}/{typeName}"
  match entry.getObjVal? "rejectionReason" with
  | .ok reasonJson =>
    -- A refusal fixture holds bytes no value encodes to, and names why.
    let expectedReason ← reasonJson.getStr?
    let raw ← fromHex (← (← entry.getObjVal? "rawBytes").getStr?)
    match deserialize shape raw with
    | .ok _ => throw s!"{typeName}: decoding should have been refused"
    | .error fault =>
      -- Rejecting malformed bytes is insufficient if the rejection category disagrees with the fixture.
      if fault.reason != expectedReason then
        throw s!"{typeName}: refused with {fault.reason}, expected {expectedReason}"
      return ()
  | .error _ =>
    -- Successful fixtures independently pin the original value, serialized bytes, and Merkle root.
    let value ← readValue shape (← entry.getObjVal? "value")
    let expectedBytes ← fromHex (← (← entry.getObjVal? "serialized").getStr?)
    let expectedRoot ← fromHex (← (← entry.getObjVal? "root").getStr?)
    -- The encoding must be the bytes the specification produced, to the byte.
    let encoded ← (serialize shape value).mapError fun fault =>
      s!"{typeName}: encoding refused: {describe fault}"
    if encoded != expectedBytes then
      throw s!"{typeName}: encoded {toHex encoded}, expected {toHex expectedBytes}"
    -- The root must match too, which pins the whole tree and not just the leaves.
    let root ← (hashTreeRoot shape value).mapError fun fault =>
      s!"{typeName}: rooting refused: {describe fault}"
    if root != expectedRoot then
      throw s!"{typeName}: rooted {toHex root}, expected {toHex expectedRoot}"
    -- Decoding the bytes back must give the value they came from.
    let decoded ← (deserialize shape expectedBytes).mapError fun fault =>
      s!"{typeName}: decoding refused: {describe fault}"
    if decoded != value then
      throw s!"{typeName}: decoded a different value than the one encoded"
    return ()

/-- Every fixture file below a directory. -/
partial def fixtureFiles (dir : System.FilePath) : IO (Array System.FilePath) := do
  -- Only JSON files are collected, while directories are traversed recursively.
  let mut out := #[]
  for entry in ← dir.readDir do
    -- Fixtures sit one directory per module, so the walk descends rather than listing.
    if ← entry.path.isDir then
      out := out ++ (← fixtureFiles entry.path)
    else if entry.path.toString.endsWith ".json" then
      out := out.push entry.path
  return out

/-- How a run of the fixtures went. -/
structure Tally where
  /-- Fixtures that asserted what they claim. -/
  passed : Nat := 0
  /-- One line per fixture that did not. -/
  failures : Array String := #[]

/-- Running every fixture below a directory, and saying how it went. -/
def run (root : System.FilePath) : IO Tally := do
  let files ← fixtureFiles root
  let mut tally : Tally := {}
  -- Sorting paths makes failure reports reproducible across filesystem iteration orders.
  for file in files.qsort (fun a b => a.toString < b.toString) do
    -- The directory a fixture sits in names the module that declared its type.
    let moduleName := (file.parent.bind System.FilePath.fileName).getD ""
    let text ← IO.FS.readFile file
    -- Unreadable JSON records a failure without preventing the remaining fixtures from running.
    match Json.parse text with
    | .error message =>
      let line := s!"{file}: unreadable: {message}"
      tally := { tally with failures := tally.failures.push line }
    | .ok json =>
      -- One file holds its entries under the test that produced them.
      let entries : List (String × Json) := match json.getObj? with
        | .ok object => object.toList
        | .error _ => []
      -- Each named fixture contributes one success or one diagnostic to the final tally.
      for (name, entry) in entries do
        match checkFixture moduleName entry with
        | .ok _ => tally := { tally with passed := tally.passed + 1 }
        | .error message =>
          let line := s!"{name}: {message}"
          tally := { tally with failures := tally.failures.push line }
  return tally

end Conformance
