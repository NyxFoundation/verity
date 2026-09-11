import Ssz.Codec.Root
import Ssz.Merkle.Verify

/-! Reading an index against a value's data, and building the proofs that carry it. -/

namespace Ssz

/--
Walking the spine of a progressive shape, one turn per level.

    root
     +-- level 1, holding leaf 0
     `-- the rest of the spine
          +-- level 2, holding leaves 1 to 4
          `-- the rest of the spine
               +-- level 3, holding leaves 5 to 20
               `-- the zero node that closes it

-/
structure SpineStop where
  /-- Turns of the index still to take, once the spine has been left or exhausted. -/
  depth : Nat
  /-- First leaf of the level reached. -/
  leavesFrom : Nat
  /-- Leaves that level holds. -/
  capacity : Nat
  /-- Whether the walk turned off the spine, rather than running out of turns on it. -/
  turnedLeft : Bool

/--
Walking the spine of a progressive shape, one turn per level.

Answers with the level the walk stopped at, and how it stopped.
-/
def spineWalk (leafCount index : Nat) :
    (depth : Nat) → (leavesFrom : Nat) → (capacity : Nat) → Except Err SpineStop
  -- The turns ran out on the spine itself, so the index names a spine node.
  | 0, leavesFrom, capacity => .ok ⟨0, leavesFrom, capacity, false⟩
  -- One turn left to spend, at the spine node covering leaves from here on.
  | depth + 1, leavesFrom, capacity => do
    -- A level whose first leaf is past the data is a level the spine never opened.
    if leavesFrom ≥ leafCount then throw .pathPastSpine
    -- The turn is the bit one place below the one already spent.
    -- A clear bit turns left, which is where this level's own subtree hangs.
    if !gindexBit index depth then return ⟨depth, leavesFrom, capacity, true⟩
    -- A set bit turns right, so this level's leaves are behind us.
    -- The next level starts past them, and holds four times as many.
    spineWalk leafCount index depth (leavesFrom + capacity) (capacity * 4)

mutual

/--
Root of the subtree at one generalized index of a value's own Merkle tree.

The index is read from the top down, one bit per level.
A budget covering the declared type depth suffices for every addressable nested value.
-/
def nodeRootAt : Nat → Desc → Value → Nat → Except Err Bytes
  -- An exhausted budget cannot inspect another type, while public calls supply enough depth.
  | 0, _, _, _ => .error .proofIncomplete
  -- One unit permits this type’s layout to be inspected before entering a shallower child.
  | budget + 1, shape, value, index => do
    -- Zero names no node of any tree.
    if index < 1 then throw (.notAGindex index)
    -- The root has no branch, but it is a node, so this walk admits it.
    if index == 1 then return (← hashTreeRoot shape value)
    -- What this value puts under its tree, which is the shape the index is read against.
    let layout ← merkleLayout shape value
    -- Every bit below the leading one is a turn, so the bit width is the depth to walk.
    let full ← gindexLength index
    -- A mixed-in word is the right child, which puts the leaves one level down on the left.
    let depth := if layout.mixin.isSome then full - 1 else full
    -- Where a word is mixed in, the very first turn chooses between it and the contents.
    match layout.mixin with
    | some word =>
      -- The first turn went right, so the index names the word rather than the contents.
      if gindexBit index depth then
        -- A word is one leaf, so a turn below it descends into something with no parts.
        if depth != 0 then throw .pathIntoMixin
        -- The word is the whole node, there being nothing under it.
        return word
    -- No word is mixed in, so no turn was spent choosing, and the depth stands as measured.
    | none => pure ()
    -- Which of the two tree shapes the contents took decides how the turns are read.
    match layout.limit with
    -- A bounded shape puts every leaf in one tree, so the remaining turns walk it.
    | some capacity => boundedNode budget layout index depth 0 capacity
    -- A progressive shape spreads its leaves down a spine of widening subtrees.
    | none => progressiveNode budget layout index depth 0 1
termination_by budget => (budget, 0)

/--
Root of a node of the subtree a walk ended in, or of something inside one leaf.

    turns left <= height   the index stops inside this subtree
    turns left >  height   the index passes into one leaf's own tree
-/
def boundedNode (budget : Nat) (layout : MerkleLayout)
    (index depth leavesFrom capacity : Nat) : Except Err Bytes := do
  -- A bounded tree is padded out to a power of two, which fixes its height.
  let width := nextPow2 capacity
  -- The height is how many turns it takes to reach a leaf from the subtree's own root.
  let treeDepth := Nat.log2 width
  -- Fewer turns left than that, and the index stops on an inner node of this subtree.
  if depth ≤ treeDepth then
    -- The index names a node of this subtree, which spans a run of leaves.
    --
    -- Each turn halves that run, so the turns left say how wide the run still is.
    let span := width >>> depth
    -- The turns themselves say which run of that width, counted from the level's start.
    let start := leavesFrom + gindexBelow index depth * span
    -- Merkleizing that run alone gives the node's root, its own padding included.
    merkleizeBounded (← layoutChunksAt budget layout start (some (start + span))) (some span)
  -- More turns left than the height, so the index runs out through the bottom.
  else
    -- Deeper than the subtree: the rest is measured inside one leaf's own tree.
    let below := depth - treeDepth
    -- Dropping those turns leaves the ones that pick the leaf, read as a position in it.
    let leaf := leavesFrom + gindexBelow (index >>> below) treeDepth
    -- Whether that leaf has a tree of its own depends on how the shape put it there.
    match layout.leaves with
    -- Packed data has no tree below a leaf, elements there sharing one node.
    | .packed _ => throw .pathIntoPacked
    -- A nested leaf is a value with a tree of its own, which the walk descends into.
    | .nested values =>
      match values[leaf]? with
      -- The remaining turns are read as an index of the nested value's own tree.
      | some (some (nestedShape, nestedValue)) =>
        nodeRootAt budget nestedShape nestedValue (gindexRebase index below)
      -- A position past the leaves, or one holding no value, has no tree to descend into.
      | _ => throw .pathIntoGap
termination_by (budget, 1)

/-- A progressive walk reads either a bounded level or the remaining spine. -/
def progressiveNode (budget : Nat) (layout : MerkleLayout)
    (index depth start capacity : Nat) : Except Err Bytes := do
  -- Left turns enter a level's binary tree, while an exhausted path names the spine itself.
  let stop ← spineWalk layout.leaves.count index depth start capacity
  if stop.turnedLeft then
    boundedNode budget layout index stop.depth stop.leavesFrom stop.capacity
  else
    -- Stopping on the spine authenticates the whole remaining suffix, including its zero terminator.
    let rest ← layoutChunksAt budget layout stop.leavesFrom
    return merkleizeProgressiveFrom rest stop.capacity
termination_by (budget, 2)

end

/--
Root of the subtree at one generalized index of a value's own Merkle tree.

The budget is the type's nesting plus the index.
That is past anything the walk can spend.
-/
def nodeRoot (shape : Desc) (value : Value) (index : Nat) : Except Err Bytes :=
  -- Each level of the walk enters one nested type or consumes one turn of the index.
  nodeRootAt (shape.nesting + index) shape value index

/--
Branch that authenticates one generalized index of a value against its root.

Every node on the path contributes its sibling, bottom-up, as a verifier reads it.
-/
def buildProof (shape : Desc) (value : Value) (index : Nat) : Except Err (List Bytes) := do
  -- A verifier holds the leaf and rebuilds upward, needing the other child at each level.
  (← getBranchIndices index).mapM (nodeRoot shape value)

/--
Nodes that authenticate several generalized indices of a value at once.

Only what a verifier cannot rebuild is carried.
The claims themselves are the caller's to supply.
-/
def buildMultiproof (shape : Desc) (value : Value) (indices : List Nat) :
    Except Err (List Bytes) := do
  -- Branches that meet share every node above the meeting point, and those are dropped.
  (← getHelperIndices indices).mapM (nodeRoot shape value)

/-- Reading a list of nodes that all answer gives the list of what they answered. -/
theorem mapM_of_ok {alpha : Type} (node : alpha → Bytes) :
    ∀ items : List alpha,
      items.mapM (fun item => (.ok (node item) : Except Err Bytes)) = .ok (items.map node)
  | [] => rfl
  | item :: rest => by
    -- Reading the head answers, so the whole reading is the head with the rest behind it.
    simp [List.mapM_cons, mapM_of_ok node rest, Bind.bind, Except.bind, pure, Except.pure]

/-- The index naming the root reads back the value's own root, as it must. -/
theorem nodeRoot_root (shape : Desc) (value : Value) :
    nodeRoot shape value 1 = hashTreeRoot shape value := by
  -- The walk answers the root outright, before it reads anything of the shape.
  rw [nodeRoot, nodeRootAt]
  cases hashTreeRoot shape value <;> rfl

/-- Reading a finite set of successful nodes returns exactly those nodes. -/
theorem mapM_of_ok_on (node : Nat → Bytes) (read : Nat → Except Err Bytes) :
    ∀ indices : List Nat, (∀ index ∈ indices, read index = .ok (node index)) →
      indices.mapM read = .ok (indices.map node)
  | [], _ => rfl
  | index :: rest, reads => by
    -- The head and tail need success only at the positions actually requested.
    simp [List.mapM_cons, reads index List.mem_cons_self,
      mapM_of_ok_on node read rest (fun at_ member => reads at_ (List.mem_cons_of_mem _ member)),
      Bind.bind, Except.bind, pure, Except.pure]

/--
A constructed branch rebuilds the value's root if its parent equations hold.

Only the root, the claimed node, and its branch siblings must be readable.
Leaves need no children, and index zero is never required.
-/
theorem buildProof_rebuilds_root {shape : Desc} {value : Value} {index : Nat}
    {node : Nat → Bytes} {indices : List Nat}
    (indexed : getBranchIndices index = .ok indices)
    (parents : BranchConsistent node index)
    (walk : ∀ position ∈ index :: 1 :: indices,
      nodeRoot shape value position = .ok (node position))
    {branch : List Bytes} (built : buildProof shape value index = .ok branch) :
    calculateMerkleRoot (node index) branch index = hashTreeRoot shape value := by
  -- Construction reads precisely the siblings named by the index.
  have reads := mapM_of_ok_on node (nodeRoot shape value) indices
    (fun position member => walk position (by simp [member]))
  simp only [buildProof, indexed, Bind.bind, Except.bind, reads, Except.ok.injEq] at built
  subst branch
  -- The local parent equations reach the root, which the walker identifies with the value.
  rw [branch_rebuilds_root_on_path parents indexed]
  have root := walk 1 (by simp)
  simpa only [nodeRoot_root] using root.symm

/-- Zero is rejected independently of the type and value being walked. -/
theorem nodeRoot_zero (shape : Desc) (value : Value) :
    nodeRoot shape value 0 = .error (.notAGindex 0) := by
  -- Every type has positive nesting, so the walk reaches the index check.
  cases shape <;> simp [nodeRoot, Desc.nesting, nodeRootAt, Bind.bind, Except.bind] <;> rfl

end Ssz
