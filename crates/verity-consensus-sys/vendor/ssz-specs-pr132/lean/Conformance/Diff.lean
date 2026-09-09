import Conformance.Runner

/-! Checking the Lean implementation against a corpus the Python generated on the fly. -/

namespace Conformance

open Ssz Lean

/-- A type read from JSON, where the corpus carries the type rather than naming one. -/
partial def readDesc (json : Json) : Except String Desc := do
  -- Which of the thirteen shapes this is, which decides what else to read.
  let kind ← (← json.getObjVal? "k").getStr?
  -- Missing or mistyped declaration fields are rejected instead of silently defaulted.
  -- A count: a width, a length, or a capacity, depending on the shape asking.
  let width (key : String) : Except String Nat := do (← json.getObjVal? key).getNat?
  -- The nested declaration supplies the one type shared by every sequence element.
  let element : Except String Desc := do readDesc (← json.getObjVal? "el")
  -- The types a struct or a union holds, likewise by recursion, one per entry.
  let descs (key : String) : Except String (List Desc) := do
    ((← (← json.getObjVal? key).getArr?).toList).mapM readDesc
  -- The field names a struct declares, which are part of what makes two types alike.
  let strings (key : String) : Except String (List String) := do
    ((← (← json.getObjVal? key).getArr?).toList).mapM Json.getStr?
  -- The selectors a union gives its options.
  let nats (key : String) : Except String (List Nat) := do
    ((← (← json.getObjVal? key).getArr?).toList).mapM Json.getNat?
  -- The layout a progressive container places its fields by, one bit per position.
  let bools (key : String) : Except String (List Bool) := do
    ((← (← json.getObjVal? key).getArr?).toList).mapM Json.getBool?
  -- Each declaration kind consumes precisely its required widths, fields, or selectors.
  match kind with
  | "bool" => return .bool
  | "uint" => return .uint (← width "w")
  | "byteVector" => return .byteVector (← width "n")
  | "byteList" => return .byteList (← width "n")
  | "bitVector" => return .bitVector (← width "n")
  | "bitList" => return .bitList (← width "n")
  | "progressiveBitList" => return .progressiveBitList
  | "vector" => return .vector (← element) (← width "n")
  | "list" => return .list (← element) (← width "n")
  | "progressiveList" => return .progressiveList (← element)
  | "container" => return .container (← strings "names") (← descs "fields")
  | "progressiveContainer" =>
    return .progressiveContainer (← bools "active") (← strings "names") (← descs "fields")
  | "compatibleUnion" => return .compatibleUnion (← nats "selectors") (← descs "options")
  | other => throw s!"unknown kind: {other}"

/-- One step of a path, as the corpus writes it. -/
def readStep (json : Json) : Except String PathStep := do
  -- A step is written either as a position or as the name of a mixed-in word.
  match json.getObjVal? "p" with
  | .ok position => return .position (← position.getNat?)
  -- No position means the step names a word, which is the last step a path can take.
  | .error _ =>
    match ← (← json.getObjVal? "w").getStr? with
    | "length" => return .length
    | "activeFields" => return .activeFields
    | "selector" => return .selector
    | other => throw s!"unknown word: {other}"

/-- Turning a refusal into a message, so the two sides can be compared as values. -/
def orFail {α : Type} (name : String) : Except Err α → Except String α
  | .ok value => .ok value
  -- A specification error becomes a labeled comparison failure without changing successful results.
  | .error fault => .error s!"{name} refused: {describe fault}"

/-- Every node of a branch, and the claim it authenticates, agreeing with the corpus. -/
def checkPath (shape : Desc) (value : Value) (root : Bytes) (entry : Json) :
    Except String Unit := do
  -- Positions and reserved-word steps are decoded before either implementation’s path is compared.
  let steps ← (← (← entry.getObjVal? "path").getArr?).toList.mapM readStep
  -- A path the specification refuses must be refused here too, at one step or another.
  if (← (← entry.getObjVal? "refused").getBool?) then
    match getGeneralizedIndex shape steps with
    | .error _ => return ()
    | .ok index =>
      -- A type-level address may exist while the selected value has no readable node or branch.
      match nodeRoot shape value index, buildProof shape value index with
      | .ok _, .ok _ => throw "walked a path the specification refuses"
      | _, _ => return ()
  -- Accepted paths must agree on their generalized index before their node and branch are compared.
  let expectedIndex ← (← entry.getObjVal? "gindex").getNat?
  let index ← orFail "gindex" (getGeneralizedIndex shape steps)
  if index != expectedIndex then
    throw s!"gindex {index}, expected {expectedIndex}"
  -- The node the index names must be the one the specification reports.
  let expectedNode ← fromHex (← (← entry.getObjVal? "node").getStr?)
  let node ← orFail "node" (nodeRoot shape value index)
  if node != expectedNode then
    throw s!"node {toHex node}, expected {toHex expectedNode}"
  -- The branch must match too, node for node and in the same order.
  -- Each expected sibling remains a complete byte string in its original proof order.
  let expectedProof ← (← (← entry.getObjVal? "proof").getArr?).toList.mapM fun item => do
    fromHex (← item.getStr?)
  let proof ← orFail "proof" (buildProof shape value index)
  if proof != expectedProof then
    throw s!"branch {proof.map toHex}, expected {expectedProof.map toHex}"
  -- And the branch must actually rebuild the root it claims to authenticate.
  if !(← orFail "verify" (verifyMerkleProof node proof index root)) then
    throw s!"branch for {index} does not rebuild the root"

/-- Several claims at once, and the nodes a verifier cannot rebuild for itself. -/
def checkMultiproof (shape : Desc) (value : Value) (root : Bytes) (entry : Json) :
    Except String Unit := do
  -- The nodes claimed together, which the specification already found acceptable.
  let indices ← (← (← entry.getObjVal? "indices").getArr?).toList.mapM Json.getNat?
  -- Each expected sibling remains a complete byte string in its original proof order.
  let expectedProof ← (← (← entry.getObjVal? "proof").getArr?).toList.mapM fun item => do
    fromHex (← item.getStr?)
  -- The shared nodes must match the specification's, node for node and in order.
  let proof ← orFail "multiproof" (buildMultiproof shape value indices)
  if proof != expectedProof then
    throw s!"multiproof {proof.map toHex}, expected {expectedProof.map toHex}"
  -- And the whole request must rebuild the root, which is what a verifier would do.
  let leaves ← indices.mapM fun index => orFail "leaf" (nodeRoot shape value index)
  if !(← orFail "verify" (verifyMerkleMultiproof leaves proof indices root)) then
    throw "multiproof does not rebuild the root"

/-- Everything one generated case asserts. -/
def checkCase (entry : Json) : Except String Unit := do
  -- The generated case carries its declaration directly, allowing arbitrary nested test shapes.
  let shape ← readDesc (← entry.getObjVal? "desc")
  -- A declaration the specification accepts must be accepted here, and likewise refused.
  let expectedValid ← (← entry.getObjVal? "wellFormed").getBool?
  if shape.wellFormed.toOption.isSome != expectedValid then
    throw s!"well-formedness disagrees, expected {expectedValid}"
  let value ← readValue shape (← entry.getObjVal? "value")
  let expectedBytes ← fromHex (← (← entry.getObjVal? "serialized").getStr?)
  let expectedRoot ← fromHex (← (← entry.getObjVal? "root").getStr?)
  -- Exact byte equality detects differences in field ordering, padding, and offset tables.
  let encoded ← orFail "encoding" (serialize shape value)
  if encoded != expectedBytes then
    throw s!"encoded {toHex encoded}, expected {toHex expectedBytes}"
  -- The root, which pins the whole tree and not just the leaves.
  let root ← orFail "rooting" (hashTreeRoot shape value)
  if root != expectedRoot then
    throw s!"rooted {toHex root}, expected {toHex expectedRoot}"
  -- Decoding the bytes back must give the value they came from.
  let decoded ← orFail "decoding" (deserialize shape expectedBytes)
  if decoded != value then throw "decoded a different value than the one encoded"
  -- Defaultable types must construct the same initial value in both implementations.
  match entry.getObjVal? "default" with
  | .ok defaultJson =>
    let expectedDefault ← readValue shape defaultJson
    let actual ← orFail "default" shape.default
    if actual != expectedDefault then throw "default disagrees"
  | .error _ => pure ()
  -- Every requested path checks address calculation, node reading, branch construction, and verification.
  for pathEntry in ← (← entry.getObjVal? "paths").getArr? do
    checkPath shape value root pathEntry
  -- An optional multiproof also checks shared-helper ordering and combined reconstruction.
  match entry.getObjVal? "multiproof" with
  | .ok multiproof => checkMultiproof shape value root multiproof
  | .error _ => pure ()

/-- Whether two types merkleize alike, compared against what the specification says. -/
def checkCompatible (entry : Json) : Except String Unit := do
  -- Both types are read from the corpus rather than named, so any pair can be drawn.
  let left ← readDesc (← entry.getObjVal? "left")
  let right ← readDesc (← entry.getObjVal? "right")
  -- What the specification answers about them, which this side must answer too.
  let expected ← (← entry.getObjVal? "compatible").getBool?
  if isCompatible left right != expected then
    throw s!"compatibility disagrees, expected {expected}"

/-- Running a generated corpus, and saying how it went. -/
def runDiff (file : System.FilePath) : IO (Nat × Array String) := do
  -- The corpus arrives as one file, so the language boundary is crossed once per batch.
  let text ← IO.FS.readFile file
  let json ← IO.ofExcept (Json.parse text)
  let mut passed := 0
  let mut failures : Array String := #[]
  for (label, key, check) in
      [("case", "cases", checkCase), ("compatibility", "compatible", checkCompatible)] do
    -- Absent corpus sections contribute no cases, allowing independent case and compatibility batches.
    let entries := (json.getObjVal? key).toOption.bind (·.getArr?.toOption) |>.getD #[]
    -- Every failure is retained with its case label so one run reports the entire batch.
    for entry in entries do
      match check entry with
      | .ok _ => passed := passed + 1
      | .error message =>
        let name := ((entry.getObjVal? "name").bind Json.getStr?).toOption.getD "?"
        failures := failures.push s!"{label} {name}: {message}"
  return (passed, failures)

end Conformance
