import Ssz.Type.Desc
import Ssz.Merkle.Gindex
import Ssz.Merkle.Merkleize

/-! Paths through a type, resolved to the generalized index of the node they select. -/

namespace Ssz

/-- Smallest power of two at or above a count, and one for a count of none. -/
def nextPow2 (count : Nat) : Nat := 2 ^ depthFor count

/--
One step of a path.

A position is a field of a struct, an element of a sequence, or a union selector.
The other three name the words a root is hashed against, and each ends the path.
-/
inductive PathStep where
  /-- A field, an element, or a selector, counted from zero. -/
  | position (index : Nat)
  /-- The element count a variable-size shape mixes in. -/
  | length
  /-- The field layout a progressive container mixes in. -/
  | activeFields
  /-- The type selector a compatible union mixes in. -/
  | selector
  deriving Repr, BEq, Inhabited

/-- Bytes one element of a type occupies inside a node, or a whole node if composite. -/
def Desc.itemLength : Desc → Nat
  -- A basic element shares a node with its neighbours, so it takes only its own width.
  | .bool => 1
  | .uint width => width
  -- Everything else takes a node of its own, whatever its contents come to.
  | _ => bytesPerChunk

/--
Leaves a type merkleizes into, counting only its own level.

A basic value is one leaf, and bits and basic elements pack several to a leaf.
A composite element or a field takes one of its own.
-/
def Desc.chunkCount : Desc → Except Err Nat
  | .bool => .ok 1
  | .uint _ => .ok 1
  -- A union adds no leaf of its own, since the option it holds is the whole tree below.
  | .compatibleUnion _ _ => .ok 1
  | .bitVector length => .ok ((length + 8 * bytesPerChunk - 1) / (8 * bytesPerChunk))
  | .bitList limit => .ok ((limit + 8 * bytesPerChunk - 1) / (8 * bytesPerChunk))
  | .byteVector length => .ok ((length + bytesPerChunk - 1) / bytesPerChunk)
  | .byteList limit => .ok ((limit + bytesPerChunk - 1) / bytesPerChunk)
  | .vector element length =>
    .ok ((length * element.itemLength + bytesPerChunk - 1) / bytesPerChunk)
  | .list element limit =>
    .ok ((limit * element.itemLength + bytesPerChunk - 1) / bytesPerChunk)
  | .container _ fields => .ok fields.length
  -- A progressive shape grows without bound, so it has no leaf count to report.
  | _ => .error .noChunkCount

/-- The type reached by one step of a path. -/
def Desc.elementType : Desc → PathStep → Except Err Desc
  -- Field ordinals index declarations, including when their tree positions contain gaps.
  | .container _ fields, .position index =>
    match fields[index]? with
    | some field => .ok field
    | none => .error (.noSuchField index)
  | .progressiveContainer _ _ fields, .position index =>
    match fields[index]? with
    | some field => .ok field
    | none => .error (.noSuchField index)
  | .bitVector _, _ => .ok .bool
  | .bitList _, _ => .ok .bool
  | .progressiveBitList, _ => .ok .bool
  -- A byte array is a collection of single opaque bytes.
  | .byteVector _, _ => .ok (.uint 1)
  | .byteList _, _ => .ok (.uint 1)
  | .vector element _, _ => .ok element
  | .list element _, _ => .ok element
  | .progressiveList element, _ => .ok element
  | _, _ => .error .notSteppable

/--
Positions a shape declares, or none for one that grows with its data.

A declared position is addressable whether or not a value fills it.
-/
def Desc.positionCount : Desc → Except Err (Option Nat)
  -- A progressive shape grows with its data, so it declares no bound to report.
  | .progressiveList _ => .ok none
  | .progressiveBitList => .ok none
  | .vector _ length => .ok (some length)
  | .byteVector length => .ok (some length)
  | .bitVector length => .ok (some length)
  | .list _ limit => .ok (some limit)
  | .byteList limit => .ok (some limit)
  | .bitList limit => .ok (some limit)
  | _ => .error .notSteppable

/-- Position of the requested set bit, counted from zero, or none if it is absent. -/
def activePosition : List Bool → Nat → Option Nat
  -- An empty layout has no remaining active field.
  | [], _ => none
  -- A gap shifts every subsequent field one position to the right.
  | false :: rest, ordinal => (activePosition rest ordinal).map (· + 1)
  -- The first active field occupies this position.
  | true :: _, 0 => some 0
  -- Passing an active field consumes both a position and a field ordinal.
  | true :: rest, ordinal + 1 => (activePosition rest ordinal).map (· + 1)

/-- Layout position of a field, or an error identifying the missing field ordinal. -/
def layoutPosition (active : List Bool) (ordinal : Nat) : Except Err Nat :=
  -- EIP-7495 numbers fields consecutively while their active positions may contain gaps.
  match activePosition active ordinal with
  | some position => .ok position
  | none => .error (.noSuchField ordinal)

/-- Where one element sits inside the tree, and inside the node that holds it. -/
structure ChunkPosition where
  /-- Leaf the element lands in, which is what a generalized index names. -/
  chunk : Nat
  /-- First byte of the element inside that leaf. -/
  start : Nat
  /-- One past its last byte, equal to the first where the element is a single bit. -/
  stop : Nat
  deriving Repr, BEq

/-- Where one element sits: the node holding it, and its byte range inside that node. -/
def Desc.chunkPosition (shape : Desc) (step : PathStep) : Except Err ChunkPosition := do
  -- The child type determines whether the position shares a chunk or occupies a complete node.
  let width := (← shape.elementType step).itemLength
  match shape, step with
  -- A progressive container merkleizes a field at its layout position, not at its ordinal.
  | .progressiveContainer active _ _, .position ordinal =>
    return ⟨← layoutPosition active ordinal, 0, width⟩
  | .container _ _, .position ordinal => return ⟨ordinal, 0, width⟩
  | _, .position position =>
    -- Declared capacities bound type-level positions even when the current value is shorter.
    match ← shape.positionCount with
    | some count => if position ≥ count then throw (.noSuchPosition position)
    | none => pure ()
    match shape with
    -- One bit occupies no whole byte, so a bit reports an empty range inside its node.
    | .bitVector _ | .bitList _ | .progressiveBitList =>
      return ⟨position / (8 * bytesPerChunk), 0, 0⟩
    | _ =>
      -- Dividing the byte offset by thirty-two separates the chunk index from its internal byte range.
      let start := position * width
      return ⟨start / bytesPerChunk, start % bytesPerChunk, start % bytesPerChunk + width⟩
  | _, _ => throw .notSteppable

/-- Whether a type mixes in the word a reserved step names. -/
def Desc.mixesIn : Desc → PathStep → Bool
  -- Every variable-size shape hashes its contents against its own count.
  | .list _ _, .length => true
  | .byteList _, .length => true
  | .bitList _, .length => true
  | .progressiveList _, .length => true
  | .progressiveBitList, .length => true
  -- A progressive container mixes its layout in, which tells absent from zero.
  | .progressiveContainer _ _ _, .activeFields => true
  -- A union mixes its selector in, which separates options holding equal data.
  | .compatibleUnion _ _, .selector => true
  -- Every other pairing names a word the type does not have.
  | _, _ => false

/-- One path step's relative node index and child type, or no child for a terminal mixing word. -/
def Desc.resolveStep (shape : Desc) (step : PathStep) : Except Err (Nat × Option Desc) := do
  -- Basic values are leaves, so neither positions nor mixing words exist below them.
  match shape with
  | .bool | .uint _ => throw .noParts
  | _ => pure ()
  match step with
  | .length | .activeFields | .selector =>
    -- Reserved words are addressable only on types that actually commit to that word.
    if !shape.mixesIn step then throw .noMixin
    return (3, none)
  | .position ordinal =>
    match shape with
    | .compatibleUnion selectors options =>
      match selectors.idxOf? ordinal with
      | none => throw (.noSuchOption ordinal)
      | some slot =>
        -- Every selected option occupies the contents child, opposite the selector word.
        return (2, some options[slot]!)
    | .progressiveContainer _ _ _ | .progressiveList _ | .progressiveBitList =>
      -- Progressive chunks retain their positions as the right-hand spine grows.
      let placed ← shape.chunkPosition step
      return (progressiveChunkGindex placed.chunk, some (← shape.elementType step))
    | _ =>
      let placed ← shape.chunkPosition step
      -- A complete binary tree numbers its bottom level from the next power-of-two capacity.
      let leaf := nextPow2 (← shape.chunkCount) + placed.chunk
      -- Bounded lists place their entire contents below the left child of the root.
      let index ← match shape with
        | .list _ _ | .byteList _ | .bitList _ => gindexConcat 2 leaf
        | _ => pure leaf
      return (index, some (← shape.elementType step))

/-- Resolve a type path by splicing each relative subtree path into the preceding step. -/
def getGeneralizedIndex (shape : Desc) : List PathStep → Except Err Nat
  -- An empty path selects the root, including for a basic value.
  | [] => .ok 1
  | step :: rest => do
    -- Each step supplies both its relative address and the type available for further descent.
    let (index, target) ← shape.resolveStep step
    match target with
    | none =>
      -- A mixing word has no children, so its step must finish the path.
      if rest.isEmpty then return index else throw .noPartsMixin
    | some child =>
      -- The leading bit belongs to each subtree root and is removed when paths are joined.
      gindexConcat index (← getGeneralizedIndex child rest)

end Ssz
