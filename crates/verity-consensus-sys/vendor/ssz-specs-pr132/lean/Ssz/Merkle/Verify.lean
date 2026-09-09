import Ssz.Merkle.Gindex
import Ssz.Merkle.Tree

/-! Rebuilding a root from nodes and the indices they sit at, reading no declaration. -/

namespace Ssz

/--
Joining a node with each branch node in turn, from the leaf upward.

The level counts how far the walk has climbed, which is the bit of the index it reads.
-/
def climbBranch (index : Nat) : (level : Nat) → (node : Bytes) → List Bytes → Bytes
  -- Nothing is left to join, so the node reached is the root the branch rebuilt.
  | _, node, [] => node
  | level, node, sibling :: rest =>
    -- A set bit puts this node on the right, so the branch node joins on the left.
    let parent := if gindexBit index level then combine sibling node else combine node sibling
    climbBranch index (level + 1) parent rest

/--
A root rebuilt from one leaf and its branch.

Each bit of the index is read from the leaf upward.
It says which side the branch node joins on.
The verifier checks node widths before using this algebraic reconstruction.
-/
def calculateMerkleRoot (leaf : Bytes) (proof : List Bytes) (index : Nat) :
    Except Err Bytes := do
  -- An index fixes the exact number of siblings needed before the upward fold can start.
  let depth ← gindexLength index
  if proof.length != depth then throw (.branchLength depth proof.length)
  return climbBranch index 0 leaf proof

/-- Refuse a hash operand that is not one complete 32-byte SSZ node. -/
def checkChunk (node : Bytes) : Except Err Unit :=
  -- Fixed-width children make the boundary between the two hash operands unambiguous.
  if node.size = bytesPerChunk then .ok () else .error (.scope bytesPerChunk node.size)

/-- Refuse a proof containing a node of the wrong width. -/
def checkChunks : List Bytes → Except Err Unit
  | [] => .ok ()
  | node :: rest => do
    -- Every sibling must occupy a whole node before any pairing is attempted.
    checkChunk node
    checkChunks rest

/-- Whether one leaf and its branch rebuild the expected root. -/
def verifyMerkleProof (leaf : Bytes) (proof : List Bytes) (index : Nat) (root : Bytes) :
    Except Err Bool := do
  -- A 31-byte child and a 33-byte sibling must not impersonate two 32-byte nodes.
  checkChunk leaf
  checkChunk root
  checkChunks proof
  return (← calculateMerkleRoot leaf proof index) == root

/-- The node held at an index, where one is held. -/
def nodeAt (nodes : List (Nat × Bytes)) (index : Nat) : Option Bytes :=
  -- Search by position so the matching bytes are read independently of storage order.
  (nodes.find? fun (at_, _) => at_ == index).map fun (_, node) => node

/-- Levels a node sits below the root, which its index carries in its leading bit. -/
def levelOf (index : Nat) : Nat :=
  -- The leading binary bit marks the root, leaving one remaining bit per tree level.
  Nat.log2 index

/-- One level's parents and shallower nodes, kept in their original order. -/
def foldLevelNodes (depth : Nat) (nodes : List (Nat × Bytes)) :
    List (Nat × Bytes) → Except Err (List (Nat × Bytes) × List (Nat × Bytes))
  | [] => .ok ([], [])
  | (index, node) :: rest => do
    if levelOf index != depth then
      -- A shallower node waits until the walk reaches its level.
      let (parents, kept) ← foldLevelNodes depth nodes rest
      return (parents, (index, node) :: kept)
    else if index % 2 == 0 then
      -- An even index owns the pair, with the odd index on its right.
      match nodeAt nodes (index + 1) with
      | some sibling =>
        let (parents, kept) ← foldLevelNodes depth nodes rest
        return ((gindexParent index, combine node sibling) :: parents, kept)
      | none => throw .proofIncomplete
    else if (nodeAt nodes (index - 1)).isNone then
      -- An odd node must have a left sibling, which performs the actual hash.
      throw .proofIncomplete
    else foldLevelNodes depth nodes rest

/-- Every pair at one level folded into its parent, with shallower nodes kept aside. -/
def foldLevel (depth : Nat) (nodes : List (Nat × Bytes)) :
    Except Err (List (Nat × Bytes)) := do
  -- Parents precede the waiting nodes, preserving the order within each group.
  let (parents, kept) ← foldLevelNodes depth nodes nodes
  return parents ++ kept

/--
Folding a node set upward one level at a time, until only the root is left.

Recursion is on the depth rather than on the node set.
The fold is therefore structural, and needs no bound of its own.
-/
def foldToRoot : Nat → List (Nat × Bytes) → Except Err Bytes
  -- Nothing sits above the root, so the fold ends by reading it.
  | 0, nodes =>
    match nodeAt nodes 1 with
    | some root => .ok root
    -- A proof from which the root never appeared authenticates nothing.
    | none => .error .proofIncomplete
  -- Otherwise the deepest level folds away and the rest is the same problem, one shallower.
  | depth + 1, nodes => do foldToRoot depth (← foldLevel (depth + 1) nodes)

/--
A root rebuilt from several leaves and the nodes they share.

Pairs combine from the deepest level upward, each parent built once.
The verifier checks node widths before using this algebraic reconstruction.
-/
def calculateMultiMerkleRoot (leaves proof : List Bytes) (indices : List Nat) :
    Except Err Bytes := do
  if leaves.length != indices.length then throw (.leafCount indices.length leaves.length)
  let helpers ← getHelperIndices indices
  if proof.length != helpers.length then throw (.proofLength helpers.length proof.length)
  let nodes := indices.zip leaves ++ helpers.zip proof
  -- No node sits below the deepest index given, so that is where the fold starts.
  let deepest := (nodes.map fun (index, _) => levelOf index).foldl max 0
  foldToRoot deepest nodes

/-- Whether several leaves and their nodes rebuild the expected root. -/
def verifyMerkleMultiproof (leaves proof : List Bytes) (indices : List Nat) (root : Bytes) :
    Except Err Bool := do
  -- The claimed nodes, auxiliary nodes, and expected root all have the SSZ hash width.
  checkChunk root
  checkChunks leaves
  checkChunks proof
  return (← calculateMultiMerkleRoot leaves proof indices) == root

/--
A node reading that is a Merkle tree: every node is the two below it, combined.

Nothing here fixes the leaves, so this covers any tree built with the same pairing.
-/
def MerkleTree (node : Nat → Bytes) : Prop :=
  -- This equation is global.
  -- Finite authentication results instead restrict it to the required ancestors.
  ∀ index, 1 ≤ index → node index = combine (node (2 * index)) (node (2 * index + 1))

/-- A node and its sibling combine to the node above them, in whichever order they sit. -/
theorem combine_sibling {node : Nat → Bytes} (tree : MerkleTree node) (index : Nat)
    (named : 1 ≤ index / 2) :
    (if index % 2 = 1 then combine (node (gindexSibling index)) (node index)
      else combine (node index) (node (gindexSibling index))) = node (index / 2) := by
  rcases Nat.mod_two_eq_zero_or_one index with even | odd
  · -- An even node is the left child, so the sibling joins on the right.
    rw [if_neg (by omega)]
    obtain ⟨whole, pair⟩ := gindexSibling_even even
    have expanded : combine (node index) (node (gindexSibling index))
        = combine (node (2 * (index / 2))) (node (2 * (index / 2) + 1)) := by
      rw [← pair, ← whole]
    rw [expanded]
    exact (tree _ named).symm
  · -- An odd node is the right child, so the sibling joins on the left.
    rw [if_pos (by omega)]
    obtain ⟨whole, pair⟩ := gindexSibling_odd odd
    have expanded : combine (node (gindexSibling index)) (node index)
        = combine (node (2 * (index / 2))) (node (2 * (index / 2) + 1)) := by
      rw [← whole, ← pair]
    rw [expanded]
    exact (tree _ named).symm

/-- One level of the climb joins a node with its sibling, giving the node above them. -/
theorem climbBranch_step {node : Nat → Bytes} (tree : MerkleTree node) (index level : Nat)
    (named : 1 ≤ index >>> (level + 1)) :
    (if gindexBit index level then combine (node (gindexSibling (index >>> level)))
        (node (index >>> level))
      else combine (node (index >>> level)) (node (gindexSibling (index >>> level))))
      = node (index >>> (level + 1)) := by
  -- Climbing one level is halving, and the turn taken is the parity of what is halved.
  rw [shiftRight_succ] at named ⊢
  rw [gindexBit_parity]
  simp only [decide_eq_true_eq]
  exact combine_sibling tree (index >>> level) named

/-- A node shifted further up is no larger than the same node shifted less. -/
theorem shiftRight_le (index level : Nat) : index >>> level ≤ index := by
  -- Shifting is dividing by a power of two, which never grows a number.
  rw [Nat.shiftRight_eq_div_pow]
  exact Nat.div_le_self index (2 ^ level)

/--
Climbing a branch of a Merkle tree arrives at the node above every level it climbed.

The branch is read as the tree's own siblings, one per level, from the leaf upward.
-/
theorem climbBranch_folds {node : Nat → Bytes} (tree : MerkleTree node) :
    ∀ (count index level : Nat), 1 ≤ index >>> (level + count) →
      climbBranch index level (node (index >>> level))
          ((List.range count).map fun step => node (gindexSibling (index >>> (level + step))))
        = node (index >>> (level + count)) := by
  intro count
  induction count with
  | zero =>
    -- Nothing to climb, so the node reached is the one the walk opened on.
    intro index level _
    simp [climbBranch]
  | succ count ih =>
    intro index level reaches
    -- The first sibling is the one at this level, and the rest belong one level up.
    rw [List.range_succ_eq_map]
    simp only [List.map_cons, List.map_map, Function.comp_def, Nat.add_zero]
    rw [climbBranch]
    -- The levels the rest of the branch names are the ones the climb reaches next.
    have shift : (fun step => node (gindexSibling (index >>> (level + (step + 1)))))
        = fun step => node (gindexSibling (index >>> (level + 1 + step))) := by
      funext step
      have same : level + (step + 1) = level + 1 + step := by omega
      rw [same]
    -- The remaining climb still has to reach the root, which is what it opened able to do.
    have onward : 1 ≤ index >>> (level + 1 + count) := by
      have same : level + 1 + count = level + (count + 1) := by omega
      rw [same]
      exact reaches
    -- One level of the climb is a node joined with its sibling, which is the node above.
    have above : 1 ≤ index >>> (level + 1) :=
      Nat.le_trans onward (by rw [Nat.shiftRight_add]; exact shiftRight_le _ _)
    rw [shift, climbBranch_step tree index level above, ih index (level + 1) onward]
    have same : level + 1 + count = level + (count + 1) := by omega
    rw [same]

/-- Reading an index that names a node gives its depth, which is at least one level. -/
theorem gindexLength_ok {index depth : Nat} (measured : gindexLength index = .ok depth) :
    depth = Nat.log2 index ∧ 1 ≤ index ∧ depth ≠ 0 := by
  -- A number naming no node is refused, and so is the root, which sits on no branch.
  unfold gindexLength gindexDepth at measured
  split at measured
  · simp [Bind.bind, Except.bind] at measured
  · rename_i named
    simp only [Bind.bind, Except.bind] at measured
    split at measured
    · simp at measured
    · rename_i deep
      simp only [Except.ok.injEq] at measured
      simp only [beq_iff_eq] at deep
      exact ⟨measured.symm, by omega, measured ▸ deep⟩

/-- The branch of an index is the tree's siblings, one per level below the root. -/
theorem getBranchIndices_eq {index : Nat} {branch : List Nat}
    (built : getBranchIndices index = .ok branch) :
    branch = (List.range (Nat.log2 index)).map (fun step => gindexSibling (index >>> step))
      ∧ 1 ≤ index ∧ Nat.log2 index ≠ 0 := by
  -- The walk upward names one ancestor per level, and the branch is what sits beside them.
  unfold getBranchIndices getPathIndices at built
  cases measured : gindexLength index with
  | error _ =>
    rw [measured] at built
    simp [Bind.bind, Except.bind] at built
  | ok depth =>
    obtain ⟨named, positive, deep⟩ := gindexLength_ok measured
    subst named
    rw [measured] at built
    simp [Bind.bind, Except.bind, pure, Except.pure, List.map_map, Function.comp_def] at built
    exact ⟨built.symm, positive, deep⟩

/-- Each parent on one branch is its child and sibling hashed in tree order. -/
def BranchConsistent (node : Nat → Bytes) (index : Nat) : Prop :=
  -- Only parent equations along this one path are required, with no assumptions below its leaf.
  ∀ level, level < Nat.log2 index →
    (if gindexBit index level then
      combine (node (gindexSibling (index >>> level))) (node (index >>> level))
    else combine (node (index >>> level)) (node (gindexSibling (index >>> level))))
      = node (index >>> (level + 1))

/-- A climb reaches its last ancestor when each step agrees with its parent. -/
theorem climbBranch_folds_on_path (node : Nat → Bytes) (index : Nat) :
    ∀ count level,
      (∀ step, level ≤ step → step < level + count →
        (if gindexBit index step then
          combine (node (gindexSibling (index >>> step))) (node (index >>> step))
        else combine (node (index >>> step)) (node (gindexSibling (index >>> step))))
          = node (index >>> (step + 1))) →
      climbBranch index level (node (index >>> level))
        ((List.range count).map fun step => node (gindexSibling (index >>> (level + step))))
        = node (index >>> (level + count)) := by
  intro count
  induction count with
  | zero =>
    -- An empty branch leaves the current node unchanged.
    intro level _
    simp [climbBranch]
  | succ count ih =>
    intro level parents
    -- Peel off the first sibling, then start the remaining climb one level higher.
    rw [List.range_succ_eq_map]
    simp only [List.map_cons, List.map_map, Function.comp_def, Nat.add_zero, climbBranch]
    rw [parents level (by omega) (by omega)]
    simpa [Nat.add_assoc, Nat.add_comm, Nat.add_left_comm] using
      ih (level + 1) (fun step lower upper => parents step (by omega) (by omega))

/-- A branch rebuilds the root using only the parent equations along that branch. -/
theorem branch_rebuilds_root_on_path {node : Nat → Bytes} {index : Nat}
    (parents : BranchConsistent node index) {branch : List Nat}
    (built : getBranchIndices index = .ok branch) :
    calculateMerkleRoot (node index) (branch.map node) index = .ok (node 1) := by
  -- The index determines both the branch length and the sibling order.
  obtain ⟨spelled, named, deep⟩ := getBranchIndices_eq built
  subst spelled
  have notZero : index ≠ 0 := by omega
  have measured : gindexLength index = .ok (Nat.log2 index) := by
    simp [gindexLength, gindexDepth, Bind.bind, Except.bind, notZero, deep]
  -- No equation is needed below the claimed node or on an unrelated branch.
  have climbed := climbBranch_folds_on_path node index (Nat.log2 index) 0
    (fun step _ upper => parents step (by omega))
  simp only [Nat.zero_add, Nat.shiftRight_zero] at climbed
  rw [shiftRight_depth named] at climbed
  simp [calculateMerkleRoot, measured, Bind.bind, Except.bind, List.map_map,
    Function.comp_def, climbed, pure, Except.pure]

/--
Every branch this library builds rebuilds the root of the tree it was read from.

Nothing about the leaves is assumed, only that each node is the two below it combined.
-/
theorem branch_rebuilds_root {node : Nat → Bytes} (tree : MerkleTree node) {index : Nat}
    {branch : List Nat} (built : getBranchIndices index = .ok branch) :
    calculateMerkleRoot (node index) (branch.map node) index = .ok (node 1) := by
  obtain ⟨spelled, named, deep⟩ := getBranchIndices_eq built
  subst spelled
  -- The branch is as long as the index is deep, so the verifier accepts its length.
  have notZero : ¬ index = 0 := by omega
  have measured : gindexLength index = .ok (Nat.log2 index) := by
    simp [gindexLength, gindexDepth, Bind.bind, Except.bind, notZero, deep]
  -- Climbing every level the index carries arrives at the root, which is where it stops.
  have climbed := climbBranch_folds tree (Nat.log2 index) index 0
  simp only [Nat.zero_add, Nat.shiftRight_zero] at climbed
  rw [shiftRight_depth named] at climbed
  simp [calculateMerkleRoot, measured, Bind.bind, Except.bind, List.map_map, Function.comp_def,
    climbed, pure, Except.pure]

end Ssz
