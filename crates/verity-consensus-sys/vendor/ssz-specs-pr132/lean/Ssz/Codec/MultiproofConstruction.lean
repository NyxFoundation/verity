import Ssz.Codec.NodeWidths
import Ssz.Codec.RootDomain
import Ssz.Codec.WalkerLaws
import Ssz.Codec.BranchConstruction
import Ssz.Merkle.Multiproof

/-! Multiproofs constructed from a value reconstruct its root without extending its leaf nodes. -/

namespace Ssz

/-- Each indexed node is a successful read of the value's finite tree. -/
def ReadNodes (read : Nat → Except Err Bytes) (nodes : List (Nat × Bytes)) : Prop :=
  ∀ index node, (index, node) ∈ nodes → read index = .ok node

/-- A level fold preserves successful reads of a tree closed under parents and siblings. -/
theorem foldLevelNodes_reads {read : Nat → Except Err Bytes} (closed : WalkerClosed read)
    {depth : Nat} (positive : 0 < depth) {nodes pending : List (Nat × Bytes)}
    (allRead : ReadNodes read nodes) (pendingRead : ReadNodes read pending)
    {parents kept : List (Nat × Bytes)}
    (folded : foldLevelNodes depth nodes pending = .ok (parents, kept)) :
    ReadNodes read (parents ++ kept) := by
  -- Only the parent of a genuinely readable pair needs a hash equation.
  induction pending generalizing parents kept with
  | nil =>
    simp [foldLevelNodes] at folded
    rcases folded with ⟨rfl, rfl⟩
    simp [ReadNodes]
  | cons pair rest ih =>
    rcases pair with ⟨index, node⟩
    have head := pendingRead index node (by simp)
    -- The untouched suffix still consists of genuine reads from the same finite tree.
    have tail : ReadNodes read rest := fun i v member => pendingRead i v (by simp [member])
    simp only [foldLevelNodes] at folded
    split at folded
    · cases rec : foldLevelNodes depth nodes rest with
      | error error => simp [rec, Bind.bind, Except.bind] at folded
      | ok result =>
        rcases result with ⟨ps, ks⟩
        simp [rec, Bind.bind, Except.bind, Pure.pure, Except.pure] at folded
        rcases folded with ⟨rfl, rfl⟩
        have reads := ih tail rec
        intro i v member
        simp only [List.mem_append, List.mem_cons] at member
        rcases member with member | same | member
        · exact reads i v (by simp [member])
        · cases same; exact head
        · exact reads i v (by simp [member])
    · rename_i atDepth
      have atDepth : levelOf index = depth := by simpa using atDepth
      split at folded
      · rename_i even
        have even : index % 2 = 0 := by simpa using even
        cases sibling : nodeAt nodes (index + 1) with
        | none => simp [sibling, throw] at folded
        | some siblingValue =>
          -- The neighboring helper value is identified with the sibling returned by the actual tree reader.
          have supplied := allRead (index + 1) siblingValue (nodeAt_member sibling)
          obtain ⟨parent, siblingRead, parentRead, readsSibling, equation⟩ :=
            closed index node (two_le_of_level_positive atDepth positive) head
          obtain ⟨whole, siblingIndex⟩ := gindexSibling_even even
          have siblingIndex : gindexSibling index = index + 1 := by omega
          rw [siblingIndex, supplied] at readsSibling
          cases readsSibling
          -- The stored parent is the ordered hash of this readable child and its supplied sibling.
          have parentAgrees : read (gindexParent index) = .ok (combine node siblingValue) := by
            rw [if_pos even] at equation
            rw [← equation]
            exact parentRead
          cases rec : foldLevelNodes depth nodes rest with
          | error error => simp [sibling, rec, Bind.bind, Except.bind] at folded
          | ok result =>
            rcases result with ⟨ps, ks⟩
            simp [sibling, rec, Bind.bind, Except.bind, Pure.pure, Except.pure] at folded
            rcases folded with ⟨rfl, rfl⟩
            have reads := ih tail rec
            intro i v member
            simp only [List.mem_append, List.mem_cons] at member
            rcases member with (same | member) | member
            · cases same; exact parentAgrees
            · exact reads i v (by simp [member])
            · exact reads i v (by simp [member])
      · split at folded
        · simp [throw] at folded
        · exact ih tail folded

/-- A successful complete fold returns the node actually stored at the root position. -/
theorem foldToRoot_reads {read : Nat → Except Err Bytes} (closed : WalkerClosed read) :
    ∀ depth nodes, ReadNodes read nodes → ∀ root,
      foldToRoot depth nodes = .ok root → read 1 = .ok root := by
  -- Each intermediate list still consists entirely of genuine tree reads.
  intro depth
  induction depth with
  | zero =>
    intro nodes reads root built
    simp only [foldToRoot] at built
    cases found : nodeAt nodes 1 with
    | none => simp [found] at built
    | some value =>
      simp [found] at built
      subst root
      exact reads 1 value (nodeAt_member found)
  | succ depth ih =>
    intro nodes reads root built
    cases level : foldLevel (depth + 1) nodes with
    | error error => simp [foldToRoot, level, Bind.bind, Except.bind] at built
    | ok result =>
      -- One complete level reduction preserves genuine reads for both newly computed parents and retained nodes.
      have resultReads : ReadNodes read result := by
        unfold foldLevel at level
        cases rec : foldLevelNodes (depth + 1) nodes nodes with
        | error error => simp [rec, Bind.bind, Except.bind] at level
        | ok groups =>
          rcases groups with ⟨parents, kept⟩
          simp [rec, Bind.bind, Except.bind, Pure.pure, Except.pure] at level
          subst result
          exact foldLevelNodes_reads closed (by omega) reads reads rec
      exact ih result resultReads root (by simpa [foldToRoot, level, Bind.bind, Except.bind] using built)

/-- Reading a list preserves both its length and the value read at each paired index. -/
theorem mapM_readNodes {read : Nat → Except Err Bytes} {indices : List Nat} {values : List Bytes}
    (built : indices.mapM read = .ok values) :
    values.length = indices.length ∧ ReadNodes read (indices.zip values) := by
  -- Each successful head read contributes exactly one indexed output before the remaining reads.
  induction indices generalizing values with
  | nil =>
    simp [Pure.pure, Except.pure] at built
    subst values
    simp [ReadNodes]
  | cons index rest ih =>
    cases first : read index with
    | error error => simp [List.mapM_cons, first, Bind.bind, Except.bind] at built
    | ok value =>
      cases tail : rest.mapM read with
      | error error => simp [List.mapM_cons, first, tail, Bind.bind, Except.bind] at built
      | ok remaining =>
        simp [List.mapM_cons, first, tail, Bind.bind, Except.bind, Pure.pure, Except.pure] at built
        subst values
        obtain ⟨count, reads⟩ := ih tail
        refine ⟨by simpa using count, ?_⟩
        intro i node member
        simp only [List.zip_cons_cons, List.mem_cons] at member
        rcases member with same | member
        · cases same; exact first
        · exact reads i node member

/-- Valid helper positions and genuine node reads always reconstruct the same finite tree's root. -/
theorem closed_multiproof_rebuilds_root {read : Nat → Except Err Bytes} (closed : WalkerClosed read)
    {indices helpers : List Nat} {leaves proof : List Bytes}
    (indexed : getHelperIndices indices = .ok helpers)
    (leafReads : indices.mapM read = .ok leaves) (helperReads : helpers.mapM read = .ok proof) :
    ∃ root, calculateMultiMerkleRoot leaves proof indices = .ok root ∧ read 1 = .ok root := by
  -- Helper-frontier completeness supplies a result, and preserved reads identify it as the root.
  obtain ⟨leafCount, leavesRead⟩ := mapM_readNodes leafReads
  obtain ⟨proofCount, proofRead⟩ := mapM_readNodes helperReads
  -- Helper-frontier completeness guarantees that reconstruction succeeds before read preservation identifies its result.
  obtain ⟨root, reconstructed⟩ := calculateMultiMerkleRoot_complete indexed leafCount proofCount
  have reads : ReadNodes read (indices.zip leaves ++ helpers.zip proof) := by
    intro i node member
    rcases List.mem_append.mp member with member | member
    · exact leavesRead i node member
    · exact proofRead i node member
  refine ⟨root, reconstructed, ?_⟩
  have folded := reconstructed
  simp [calculateMultiMerkleRoot, indexed, leafCount, proofCount, Bind.bind, Except.bind] at folded
  exact foldToRoot_reads closed _ _ reads _ folded

/-- A multiproof built from any readable value nodes reconstructs the actual value root. -/
theorem buildMultiproof_rebuilds_value_root (shape : Desc) (value : Value) (root : Bytes)
    (rooted : hashTreeRoot shape value = .ok root) (indices helpers : List Nat)
    (indexed : getHelperIndices indices = .ok helpers) (leaves : List Bytes)
    (leafReads : indices.mapM (nodeRoot shape value) = .ok leaves)
    (proof : List Bytes) (built : buildMultiproof shape value indices = .ok proof) :
    calculateMultiMerkleRoot leaves proof indices = .ok root := by
  -- The finite value walker supplies the parent equations only at positions actually reconstructed.
  have helperReads : helpers.mapM (nodeRoot shape value) = .ok proof := by
    simpa only [buildMultiproof, indexed, Bind.bind, Except.bind] using built
  obtain ⟨result, reconstructed, rootRead⟩ := closed_multiproof_rebuilds_root
    (nodeRoot_closed shape value root rooted) indexed leafReads helperReads
  rw [nodeRoot_root, rooted] at rootRead
  cases rootRead
  exact reconstructed

/-- Every index in a successfully read list has a successful individual read. -/
theorem mapM_readable {read : Nat → Except Err Bytes} {indices : List Nat} {values : List Bytes}
    (built : indices.mapM read = .ok values) {index : Nat} (member : index ∈ indices) :
    ∃ node, read index = .ok node := by
  -- The paired output retains all input positions because successful reading preserves length.
  obtain ⟨count, reads⟩ := mapM_readNodes built
  have keys := List.map_fst_zip (by omega : indices.length ≤ values.length)
  rw [← keys] at member
  obtain ⟨pair, pairMember, same⟩ := List.mem_map.mp member
  exact ⟨pair.2, same ▸ reads pair.1 pair.2 pairMember⟩

/-- A finite list of individually readable indices can be read as a complete list. -/
theorem mapM_readable_list (read : Nat → Except Err Bytes) :
    ∀ indices : List Nat, (∀ index ∈ indices, ∃ node, read index = .ok node) →
      ∃ values, indices.mapM read = .ok values := by
  -- Reading the head and the remaining list constructs the result in the same order.
  intro indices
  induction indices with
  | nil => intro _; exact ⟨[], rfl⟩
  | cons index rest ih =>
    intro readable
    obtain ⟨node, head⟩ := readable index (by simp)
    obtain ⟨tail, tailRead⟩ := ih (fun i member => readable i (by simp [member]))
    exact ⟨node :: tail,
      by simp [List.mapM_cons, head, tailRead, Bind.bind, Except.bind, Pure.pure, Except.pure]⟩

/-- Every helper needed by readable claims is readable in a tree closed under branches. -/
theorem closed_helper_reads {read : Nat → Except Err Bytes} (closed : WalkerClosed read)
    {indices helpers : List Nat} {leaves : List Bytes}
    (indexed : getHelperIndices indices = .ok helpers) (leafReads : indices.mapM read = .ok leaves) :
    ∃ proof, helpers.mapM read = .ok proof := by
  -- Every helper is a sibling along one of the claimed nodes' readable branches.
  obtain ⟨_, valid, shape⟩ := getHelperIndices_frontier indexed
  apply mapM_readable_list read helpers
  intro helper member
  -- Every necessary helper belongs to a claimed node’s finite sibling path.
  obtain ⟨siblingPath, _⟩ := (shape helper).mp member
  obtain ⟨ancestor, pathMember, siblingEq⟩ := List.mem_map.mp siblingPath
  obtain ⟨claim, claimed, stepMember⟩ := List.mem_flatMap.mp pathMember
  obtain ⟨step, below, ancestorEq⟩ := List.mem_map.mp stepMember
  -- The claim’s successful input read starts an upward branch whose siblings all remain readable.
  obtain ⟨leaf, leafRead⟩ := mapM_readable leafReads claimed
  have named : 1 ≤ claim := by have := valid claim claimed; omega
  obtain ⟨branch, root, branchRead, _, _⟩ :=
    closed_branch_climb read closed (Nat.log2 claim) claim 0 leaf
      (by simpa only [Nat.zero_add, shiftRight_depth named] using Nat.le_refl 1)
      (by simpa only [Nat.shiftRight_zero] using leafRead)
  simp only [Nat.zero_add] at branchRead
  apply mapM_readable branchRead
  exact List.mem_map.mpr ⟨step, below, (congrArg gindexSibling ancestorEq).trans siblingEq⟩

/-- Every valid request for readable value nodes builds a multiproof that reconstructs the value root. -/
theorem buildMultiproof_correct (shape : Desc) (value : Value) (root : Bytes)
    (rooted : hashTreeRoot shape value = .ok root) (indices helpers : List Nat)
    (indexed : getHelperIndices indices = .ok helpers) (leaves : List Bytes)
    (leafReads : indices.mapM (nodeRoot shape value) = .ok leaves) :
    ∃ proof, buildMultiproof shape value indices = .ok proof ∧
      calculateMultiMerkleRoot leaves proof indices = .ok root := by
  -- Branch closure provides all helpers, and reconstruction preserves their actual value reads.
  have closed := nodeRoot_closed shape value root rooted
  obtain ⟨proof, helperReads⟩ := closed_helper_reads closed indexed leafReads
  have built : buildMultiproof shape value indices = .ok proof := by
    simpa only [buildMultiproof, indexed, Bind.bind, Except.bind] using helperReads
  exact ⟨proof, built,
    buildMultiproof_rebuilds_value_root shape value root rooted indices helpers indexed leaves
      leafReads proof built⟩

private theorem value_reads_width (shape : Desc) (value : Value) (indices : List Nat)
    (nodes : List Bytes) (built : indices.mapM (nodeRoot shape value) = .ok nodes) :
    ∀ node ∈ nodes, node.size = bytesPerChunk := by
  -- Each returned node is paired with the successful value-tree read that produced it.
  obtain ⟨count, reads⟩ := mapM_readNodes built
  have outputs := List.map_snd_zip (by omega : nodes.length ≤ indices.length)
  intro node member
  rw [← outputs] at member
  obtain ⟨pair, paired, same⟩ := List.mem_map.mp member
  have width := nodeRoot_size shape value pair.1 pair.2 (reads pair.1 pair.2 paired)
  simpa only [same] using width

/-- Every valid request for readable value nodes builds a multiproof accepted against the value root. -/
theorem buildMultiproof_verifies (shape : Desc) (value : Value) (root : Bytes)
    (rooted : hashTreeRoot shape value = .ok root) (indices helpers : List Nat)
    (indexed : getHelperIndices indices = .ok helpers) (leaves : List Bytes)
    (leafReads : indices.mapM (nodeRoot shape value) = .ok leaves) :
    ∃ proof, buildMultiproof shape value indices = .ok proof ∧
      verifyMerkleMultiproof leaves proof indices root = .ok true := by
  -- Reconstruction gives the root equality, and every claimed or auxiliary node has full width.
  obtain ⟨proof, built, rebuilt⟩ :=
    buildMultiproof_correct shape value root rooted indices helpers indexed leaves leafReads
  have helperReads : helpers.mapM (nodeRoot shape value) = .ok proof := by
    simpa only [buildMultiproof, indexed, Bind.bind, Except.bind] using built
  -- Claimed leaves and generated helpers must separately satisfy the 32-byte node-width requirement.
  have leafWidths := (checkChunks_ok_iff leaves).mpr (value_reads_width shape value indices leaves leafReads)
  have proofWidths := (checkChunks_ok_iff proof).mpr (value_reads_width shape value helpers proof helperReads)
  refine ⟨proof, built, ?_⟩
  simp [verifyMerkleMultiproof, checkChunk, hashTreeRoot_size shape value root rooted,
    leafWidths, proofWidths, rebuilt, Bind.bind, Except.bind, Pure.pure, Except.pure]

/-- An admissible value has a root and an accepted multiproof for every valid readable request. -/
theorem buildMultiproof_fits_verifies (shape : Desc) (value : Value)
    (sound : shape.wellFormed = .ok ()) (fitted : Fits shape value)
    (indices helpers : List Nat) (indexed : getHelperIndices indices = .ok helpers)
    (leaves : List Bytes) (leafReads : indices.mapM (nodeRoot shape value) = .ok leaves) :
    ∃ root proof, hashTreeRoot shape value = .ok root ∧
      buildMultiproof shape value indices = .ok proof ∧
      verifyMerkleMultiproof leaves proof indices root = .ok true := by
  -- Admissibility supplies a complete root before the finite helper frontier is reconstructed.
  obtain ⟨root, rooted, _⟩ := hashTreeRoot_total shape value sound fitted
  obtain ⟨proof, built, verified⟩ :=
    buildMultiproof_verifies shape value root rooted indices helpers indexed leaves leafReads
  exact ⟨root, proof, rooted, built, verified⟩

end Ssz
