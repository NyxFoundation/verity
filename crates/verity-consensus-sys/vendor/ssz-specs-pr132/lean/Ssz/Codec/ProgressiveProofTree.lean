import Ssz.Codec.ProofTree
import Ssz.Codec.RootLaws

/-! Progressive proof walks follow the same widening spine that computes the root. -/

namespace Ssz

/-- With no turns left, a progressive walk reads the remaining spine. -/
theorem progressiveNode_zero (budget : Nat) (layout : MerkleLayout)
    (index start capacity : Nat) :
    progressiveNode budget layout index 0 start capacity = (do
      let rest ← layoutChunksAt budget layout start
      return merkleizeProgressiveFrom rest capacity) := by
  -- No level has been entered, so the entire suffix belongs to this node.
  simp [progressiveNode, spineWalk, Bind.bind, Except.bind]

/-- One progressive turn enters the current level or advances to the next. -/
theorem progressiveNode_step (budget : Nat) (layout : MerkleLayout)
    (index depth start capacity : Nat) :
    progressiveNode budget layout index (depth + 1) start capacity =
      if start ≥ layout.leaves.count then .error .pathPastSpine
      else if !gindexBit index depth then boundedNode budget layout index depth start capacity
      else progressiveNode budget layout index depth (start + capacity) (capacity * 4) := by
  -- The left side is a bounded tree, and the right side repeats the same walk one level farther.
  rw [progressiveNode, spineWalk]
  by_cases empty : start ≥ layout.leaves.count
  · simp [empty, throw, throwThe, MonadExceptOf.throw, Bind.bind, Except.bind]
  · simp only [if_neg empty]
    by_cases left : (!gindexBit index depth) = true
    · simp [left, Bind.bind, Except.bind, Pure.pure, Except.pure]
    · simp only [if_neg left]
      cases walked : spineWalk layout.leaves.count index depth (start + capacity) (capacity * 4)
      <;> simp [progressiveNode, walked, Bind.bind, Except.bind]

/-- Progressive capacities are powers of two with twice the level as their depth. -/
theorem four_pow_eq (level : Nat) : 4 ^ level = 2 ^ (2 * level) := by
  -- Multiplying the exponent by two squares the base before raising it to the level.
  rw [Nat.pow_mul]

/-- A power-of-four capacity starts the progressive root at its corresponding level. -/
theorem merkleizeProgressiveFrom_power (chunks : Array Bytes) (level : Nat) :
    merkleizeProgressiveFrom chunks (4 ^ level) = merkleizeProgressive chunks.toList level := by
  -- The executable conversion divides the binary depth by two.
  simp [merkleizeProgressiveFrom, four_pow_eq, Nat.log2_two_pow]

/-- A suffix read preserves the order and extent of the remaining leaves. -/
theorem extract_suffix_list (chunks : Array Bytes) (start : Nat) :
    (chunks.extract start chunks.size).toList = chunks.toList.drop start := by
  -- Extracting through the end takes every element left after the starting position.
  rw [Array.toList_extract, List.extract_eq_take_drop]
  simpa only [List.length_drop, Array.length_toList] using
    List.take_length (l := chunks.toList.drop start)

/-- A finite leaf interval is the matching prefix of the remaining suffix. -/
theorem extract_interval_list (chunks : Array Bytes) (start width : Nat) :
    (chunks.extract start (start + width)).toList = (chunks.toList.drop start).take width := by
  -- The right endpoint is exclusive, so subtracting the start leaves exactly the width.
  simp [Array.toList_extract]

/-- The first progressive level is its bounded subtree, followed by a four-times-wider suffix. -/
theorem progressive_suffix_split (chunks : Array Bytes) (start level : Nat)
    (occupied : start < chunks.size) :
    merkleizeProgressive (chunks.toList.drop start) level =
      combine (subtreeAt chunks (2 * level) start)
        (merkleizeProgressive (chunks.toList.drop (start + 4 ^ level)) (level + 1)) := by
  -- A nonempty suffix opens one level, and the next level starts after its capacity.
  have nonempty : (chunks.toList.drop start).isEmpty = false := by
    simp only [List.isEmpty_eq_false_iff]
    intro empty
    have lengths := congrArg List.length empty
    simp only [List.length_drop, Array.length_toList, List.length_nil] at lengths
    omega
  rw [merkleizeProgressive, if_neg (by simp [nonempty])]
  simp only [List.drop_drop]
  -- The current bounded segment is the prefix of the same suffix used by progressive merkleization.
  have interval := congrArg List.toArray (extract_interval_list chunks start (4 ^ level))
  simp only [Array.toArray_toList] at interval
  rw [← interval]
  have window := merkleizeBounded_window chunks (depth := 2 * level) start
  -- Extracting a segment cannot exceed its power-of-two capacity, including a partially filled final segment.
  have span : (chunks.extract start (start + 2 ^ (2 * level))).size ≤ 2 ^ (2 * level) := by
    simp only [Array.size_extract]
    generalize 2 ^ (2 * level) = width
    omega
  simp only [merkleizeBounded, if_neg (Nat.not_lt.mpr span), depthFor_pow,
    Pure.pure, Except.pure, Except.ok.injEq] at window
  simpa only [four_pow_eq, depthFor_pow] using congrArg
    (fun left => combine left
      (merkleizeProgressive (chunks.toList.drop (start + 4 ^ level)) (level + 1))) window

/-- Positions within a progressive spine and its bounded levels, including their leaves. -/
def ProgressivePosition (count index : Nat) : Nat → Nat → Nat → Prop
  -- With no turns left, the suffix root is readable even when it is the zero terminator.
  | 0, _, _ => True
  | depth + 1, start, level =>
    start < count ∧ if !gindexBit index depth then depth ≤ 2 * level
      else ProgressivePosition count index depth (start + 4 ^ level) (level + 1)

/-- Positions having two children within the progressive spine or one of its bounded levels. -/
def ProgressiveInterior (count index : Nat) : Nat → Nat → Nat → Prop
  -- A spine position has children only when at least one payload leaf remains.
  | 0, start, _ => start < count
  | depth + 1, start, level =>
    start < count ∧ if !gindexBit index depth then depth < 2 * level
      else ProgressiveInterior count index depth (start + 4 ^ level) (level + 1)

/-- The root at a position in a progressive spine, expressed in terms of its leaf array. -/
def progressiveReading (chunks : Array Bytes) (index : Nat) : Nat → Nat → Nat → Bytes
  -- Stopping on the spine commits to the complete remaining suffix.
  | 0, start, level => merkleizeProgressive (chunks.toList.drop start) level
  | depth + 1, start, level =>
    -- A left turn selects the current bounded segment, while a right turn skips its full capacity.
    if !gindexBit index depth then boundedTreeNode chunks (2 * level) index depth start
    else progressiveReading chunks index depth (start + 4 ^ level) (level + 1)

/-- Materialized leaves determine every position within the progressive tree above them. -/
theorem progressiveNode_read {budget : Nat} {layout : MerkleLayout} {chunks : Array Bytes}
    (materialized : layoutChunksAt budget layout = .ok chunks) (index : Nat) :
    ∀ depth start level, ProgressivePosition chunks.size index depth start level →
      progressiveNode budget layout index depth start (4 ^ level) =
        .ok (progressiveReading chunks index depth start level) := by
  -- Left turns read bounded intervals, while right turns advance through the suffix.
  intro depth
  induction depth with
  | zero =>
    intro start level _
    rw [progressiveNode_zero, layoutChunksAt_suffix budget layout chunks materialized start]
    simp only [Bind.bind, Except.bind, Pure.pure, Except.pure,
      merkleizeProgressiveFrom_power, extract_suffix_list, progressiveReading]
  | succ depth ih =>
    intro start level position
    obtain ⟨occupied, position⟩ := position
    -- The materialized array and the layout agree on where the finite spine ends.
    have count := layoutChunksAt_size budget layout chunks materialized
    rw [progressiveNode_step, if_neg (by omega)]
    by_cases left : (!gindexBit index depth) = true
    · simp only [if_pos left] at position ⊢
      have inside : depth ≤ depthFor (4 ^ level) := by
        simpa only [four_pow_eq, depthFor_pow] using position
      rw [boundedNode_window (layoutChunksAt_window budget layout chunks materialized)
        index depth start (4 ^ level) inside]
      simp only [four_pow_eq, depthFor_pow, progressiveReading, if_pos left]
    · simp only [if_neg left] at position ⊢
      rw [← Nat.pow_succ]
      simpa only [progressiveReading, if_neg left] using ih _ _ position

/-- An internal progressive position hashes the two positions immediately below it. -/
theorem progressiveReading_split (chunks : Array Bytes) (index : Nat) :
    ∀ depth start level, ProgressiveInterior chunks.size index depth start level →
      progressiveReading chunks index depth start level =
        combine (progressiveReading chunks (2 * index) (depth + 1) start level)
          (progressiveReading chunks (2 * index + 1) (depth + 1) start level) := by
  -- At a spine node the two children are its current level and its suffix.
  intro depth
  induction depth with
  | zero =>
    intro start level occupied
    have left : gindexBit (2 * index) 0 = false := by simp [gindexBit, Nat.testBit_zero]
    have right : gindexBit (2 * index + 1) 0 = true := by simp [gindexBit, Nat.testBit_zero]
    simp only [progressiveReading, left, right, Bool.not_false, Bool.not_true, Bool.false_eq_true, if_true, if_false]
    simp only [boundedTreeNode, gindexBelow, Nat.sub_zero, Nat.pow_zero, Nat.mod_one,
      Nat.zero_mul, Nat.add_zero]
    exact progressive_suffix_split chunks start level occupied
  | succ depth ih =>
    intro start level interior
    obtain ⟨occupied, interior⟩ := interior
    -- Both child indices retain all higher turns, so they enter the same segment as their parent.
    obtain ⟨leftChild, rightChild⟩ := gindexBit_children index depth
    simp only [progressiveReading, leftChild, rightChild]
    by_cases left : (!gindexBit index depth) = true
    · simp only [if_pos left] at interior ⊢
      exact boundedTreeNode_split chunks (2 * level) index depth start interior
    · simp only [if_neg left] at interior ⊢
      exact ih _ _ interior

/-- A readable non-root position has a readable sibling and an internal parent. -/
theorem progressivePosition_parent_sibling (count index : Nat) :
    ∀ depth start level, ProgressivePosition count index (depth + 1) start level →
      ProgressiveInterior count (index / 2) depth start level ∧
        ProgressivePosition count (gindexSibling index) (depth + 1) start level := by
  -- Removing the last turn identifies the parent, while flipping it identifies the sibling.
  intro depth
  induction depth with
  | zero =>
    intro start level position
    refine ⟨position.1, position.1, ?_⟩
    simp only [ProgressivePosition]
    split
    · omega
    · trivial
  | succ depth ih =>
    intro start level position
    obtain ⟨occupied, position⟩ := position
    -- Dropping the lowest turn and flipping that turn leave every earlier spine choice unchanged.
    have parentTurn : gindexBit (index / 2) depth = gindexBit index (depth + 1) := by
      simp [gindexBit, Nat.testBit_add_one]
    have siblingTurn := gindexBit_sibling_high index (depth + 1) (by omega)
    simp only [ProgressiveInterior, ProgressivePosition, parentTurn, siblingTurn]
    by_cases left : (!gindexBit index (depth + 1)) = true
    · simp only [if_pos left] at position ⊢
      exact ⟨⟨occupied, by omega⟩, occupied, position⟩
    · simp only [if_neg left] at position ⊢
      obtain ⟨parent, sibling⟩ := ih _ _ position
      exact ⟨⟨occupied, parent⟩, occupied, sibling⟩

/-- Every internal position is itself a readable node. -/
theorem progressiveInterior_position (count index : Nat) :
    ∀ depth start level, ProgressiveInterior count index depth start level →
      ProgressivePosition count index depth start level := by
  -- Internal bounded nodes use a strict depth bound, which also permits reading them.
  intro depth
  induction depth with
  | zero => intros; trivial
  | succ depth ih =>
    intro start level interior
    refine ⟨interior.1, ?_⟩
    by_cases left : (!gindexBit index depth) = true
    · simp only [ProgressiveInterior, if_pos left] at interior
      simpa only [if_pos left] using Nat.le_of_lt interior.2
    · simp only [ProgressiveInterior, if_neg left] at interior
      simpa only [if_neg left] using ih _ _ interior.2

/-- Every ancestor of a readable progressive position is readable too. -/
theorem progressivePosition_ancestor {count index depth start level : Nat}
    (position : ProgressivePosition count index depth start level) :
    ∀ step, step ≤ depth →
      ProgressivePosition count (index >>> step) (depth - step) start level := by
  -- Repeatedly remove the final turn, preserving the original spine origin.
  intro step
  induction step with
  | zero => simpa using position
  | succ step ih =>
    intro bounded
    have previous := ih (by omega)
    -- Removing one more turn exposes the next ancestor without changing the original segment origin.
    have remaining : depth - step = depth - (step + 1) + 1 := by omega
    rw [remaining] at previous
    have parent := (progressivePosition_parent_sibling count (index >>> step) _ _ _ previous).1
    rw [shiftRight_succ]
    exact progressiveInterior_position _ _ _ _ _ parent

/-- Both children of an internal progressive position are readable positions. -/
theorem progressiveInterior_children (count index : Nat) :
    ∀ depth start level, ProgressiveInterior count index depth start level →
      ProgressivePosition count (2 * index) (depth + 1) start level ∧
        ProgressivePosition count (2 * index + 1) (depth + 1) start level := by
  -- The current spine node opens two children, and bounded internal nodes leave one more turn.
  intro depth
  induction depth with
  | zero =>
    intro start level occupied
    constructor <;> refine ⟨occupied, ?_⟩ <;> split
    · omega
    · trivial
    · omega
    · trivial
  | succ depth ih =>
    intro start level interior
    obtain ⟨occupied, interior⟩ := interior
    -- Both child indices retain all higher turns, so they enter the same segment as their parent.
    obtain ⟨leftChild, rightChild⟩ := gindexBit_children index depth
    simp only [ProgressivePosition, leftChild, rightChild]
    by_cases left : (!gindexBit index depth) = true
    · simp only [if_pos left] at interior ⊢
      exact ⟨⟨occupied, by omega⟩, occupied, by omega⟩
    · simp only [if_neg left] at interior ⊢
      obtain ⟨left, right⟩ := ih _ _ interior
      exact ⟨⟨occupied, left⟩, occupied, right⟩

/-- The executable progressive walker combines its two child reads at every internal position. -/
theorem progressiveNode_split {budget : Nat} {layout : MerkleLayout} {chunks : Array Bytes}
    (materialized : layoutChunksAt budget layout = .ok chunks)
    (index depth start level : Nat)
    (interior : ProgressiveInterior chunks.size index depth start level) :
    progressiveNode budget layout index depth start (4 ^ level) = (do
      let left ← progressiveNode budget layout (2 * index) (depth + 1) start (4 ^ level)
      let right ← progressiveNode budget layout (2 * index + 1) (depth + 1) start (4 ^ level)
      return combine left right) := by
  -- Successful materialization gives the same leaf array to the parent and both children.
  obtain ⟨left, right⟩ := progressiveInterior_children _ _ _ _ _ interior
  rw [progressiveNode_read materialized index depth start level
      (progressiveInterior_position _ _ _ _ _ interior),
    progressiveNode_read materialized (2 * index) (depth + 1) start level left,
    progressiveNode_read materialized (2 * index + 1) (depth + 1) start level right]
  simp only [Bind.bind, Except.bind, Pure.pure, Except.pure]
  exact congrArg Except.ok (progressiveReading_split chunks index depth start level interior)

/-- A progressive tree with its length or active-field word mixed in on the right. -/
def progressiveMixedReading (chunks : Array Bytes) (word : Bytes) (index : Nat) : Bytes :=
  -- Index one commits to both the spine and its word, while index three selects only the word.
  if index = 1 then combine (merkleizeProgressive chunks.toList) word
  else if index = 3 then word
  else progressiveReading chunks index (Nat.log2 index - 1) 0 0

/-- A packed progressive layout hashes its spine against the mixed-in word. -/
theorem hashTreeRoot_packed_progressive {shape : Desc} {value : Value} {chunks : Array Bytes}
    {word : Bytes}
    (layout : merkleLayout shape value = .ok (.packing chunks none (some word))) :
    hashTreeRoot shape value = .ok (combine (merkleizeProgressive chunks.toList) word) := by
  -- Packed leaves do not consume nested-root budget before their progressive spine is built.
  have positive := shape.nesting_pos
  unfold hashTreeRoot
  cases nesting : shape.nesting with
  | zero => omega
  | succ budget =>
    simp [hashTreeRootAt, layout, layoutChunksAt, MerkleLayout.packing, mixIn,
      Leaves.count, Bind.bind, Except.bind, Pure.pure, Except.pure]

/-- The actual walker reads each position of a packed progressive value's tree. -/
theorem nodeRoot_packed_progressive {shape : Desc} {value : Value} {chunks : Array Bytes}
    {word : Bytes}
    (layout : merkleLayout shape value = .ok (.packing chunks none (some word)))
    (index : Nat) (readable : index = 1 ∨ index = 3 ∨
      (2 ≤ index ∧ gindexBit index (Nat.log2 index - 1) = false ∧
        ProgressivePosition chunks.size index (Nat.log2 index - 1) 0 0)) :
    nodeRoot shape value index = .ok (progressiveMixedReading chunks word index) := by
  -- The first turn chooses the word or a position within the progressive spine.
  rcases readable with root | wordIndex | ⟨named, leftSide, position⟩
  · subst index
    rw [nodeRoot_root, hashTreeRoot_packed_progressive layout]
    simp [progressiveMixedReading]
  · subst index
    have depth : gindexLength 3 = .ok 1 := rfl
    simp [nodeRoot, nodeRootAt, layout, depth, MerkleLayout.packing,
      gindexBit, progressiveMixedReading, Bind.bind, Except.bind, Pure.pure, Except.pure]
  · have notRoot : index ≠ 1 := by omega
    have notWord : index ≠ 3 := by
      intro same
      subst index
      change true = false at leftSide
      contradiction
    have positive : Nat.log2 index ≠ 0 := by
      have recursion := levelOf_parent named
      change Nat.log2 index = _ at recursion
      omega
    cases index with
    | zero => omega
    | succ previous =>
      simp only [nodeRoot, Nat.add_succ, nodeRootAt]
      simp [layout, gindexLength, gindexDepth, show ¬previous + 1 < 1 by omega,
        positive, leftSide, MerkleLayout.packing, Bind.bind, Except.bind,
        progressiveMixedReading, show previous ≠ 0 by omega, notWord]
      apply progressiveNode_read (chunks := chunks) _ _ _ _ _ position
      simp [layoutChunksAt, Leaves.count, Pure.pure, Except.pure]

/-- The mixed progressive root is its spine and its single word combined. -/
theorem progressiveMixedReading_root (chunks : Array Bytes) (word : Bytes) :
    progressiveMixedReading chunks word 1 =
      combine (progressiveMixedReading chunks word 2) (progressiveMixedReading chunks word 3) := by
  -- Removing the mixing turn at position two leaves the root of the whole spine.
  have depth : Nat.log2 2 = 1 := rfl
  simp [progressiveMixedReading, progressiveReading, depth]

/-- An internal position in the mixed tree's left spine obeys its parent equation. -/
theorem progressiveMixedReading_parent (chunks : Array Bytes) (word : Bytes) (index : Nat)
    (named : 2 ≤ index) (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (internal : ProgressiveInterior chunks.size index (Nat.log2 index - 1) 0 0) :
    progressiveMixedReading chunks word index =
      combine (progressiveMixedReading chunks word (2 * index))
        (progressiveMixedReading chunks word (2 * index + 1)) := by
  -- The two child indices append their final turn below the shared mixing turn.
  have positive : 0 < Nat.log2 index := by
    have recursion := levelOf_parent named
    change Nat.log2 index = _ at recursion
    omega
  have notWord : index ≠ 3 := by
    intro same
    subst index
    change true = false at leftSide
    contradiction
  have left : Nat.log2 (2 * index) = Nat.log2 index + 1 := Nat.log2_two_mul (by omega)
  have right : Nat.log2 (2 * index + 1) = Nat.log2 index + 1 := by
    have recursion := levelOf_parent (index := 2 * index + 1) (by omega)
    have half : (2 * index + 1) / 2 = index := by omega
    simpa only [levelOf, half] using recursion
  have depth : Nat.log2 index = Nat.log2 index - 1 + 1 := by omega
  simp only [progressiveMixedReading, if_neg (by omega : index ≠ 1), if_neg notWord,
    if_neg (by omega : 2 * index ≠ 1), if_neg (by omega : 2 * index ≠ 3),
    if_neg (by omega : 2 * index + 1 ≠ 1), if_neg (by omega : 2 * index + 1 ≠ 3),
    left, right, Nat.add_sub_cancel]
  rw [progressiveReading_split chunks index (Nat.log2 index - 1) 0 0 internal, ← depth]

/-- Every parent equation along a readable progressive branch follows from its tree shape. -/
theorem progressiveMixedReading_branch_left (chunks : Array Bytes) (word : Bytes) (index : Nat)
    (named : 2 ≤ index) (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (position : ProgressivePosition chunks.size index (Nat.log2 index - 1) 0 0) :
    BranchConsistent (progressiveMixedReading chunks word) index := by
  -- Ancestors stay within the same spine until the last step reaches the mixing root.
  intro level below
  have positive := path_shift_positive (by omega : 1 ≤ index) below
  have currentDepth := levelOf_shift (by omega : 1 ≤ index) level (by omega)
  have parentDepth := levelOf_parent positive
  have equation : progressiveMixedReading chunks word ((index >>> level) / 2) =
      combine (progressiveMixedReading chunks word (2 * ((index >>> level) / 2)))
        (progressiveMixedReading chunks word (2 * ((index >>> level) / 2) + 1)) := by
    by_cases root : (index >>> level) / 2 = 1
    · simpa only [root] using progressiveMixedReading_root chunks word
    · have parentPositive : 2 ≤ (index >>> level) / 2 := by omega
      have stillBelow : level + 1 < Nat.log2 index := by
        have above := levelOf_parent parentPositive
        omega
      have turn := gindexBit_shift_top (by omega : 1 ≤ index) stillBelow
      rw [shiftRight_succ] at turn
      apply progressiveMixedReading_parent chunks word _ parentPositive (turn.trans leftSide)
      -- An upward branch remains readable at every ancestor of its original progressive position.
      have current := progressivePosition_ancestor position level (by omega)
      have remaining : Nat.log2 index - 1 - level = Nat.log2 ((index >>> level) / 2) - 1 + 1 := by
        have above := levelOf_parent parentPositive
        change Nat.log2 (index >>> level) + level = Nat.log2 index at currentDepth
        change Nat.log2 (index >>> level) = Nat.log2 ((index >>> level) / 2) + 1 at parentDepth
        change Nat.log2 ((index >>> level) / 2) = _ at above
        omega
      rw [remaining] at current
      exact (progressivePosition_parent_sibling _ _ _ _ _ current).1
  -- The final bit chooses the order in which the current node and sibling are combined.
  rw [shiftRight_succ, gindexBit_parity]
  simp only [decide_eq_true_eq]
  rcases Nat.mod_two_eq_zero_or_one (index >>> level) with even | odd
  · rw [if_neg (by omega)]
    obtain ⟨left, right⟩ := gindexSibling_even even
    rw [right]
    exact (congrArg (fun at_ => combine (progressiveMixedReading chunks word at_)
      (progressiveMixedReading chunks word (2 * ((index >>> level) / 2) + 1))) left).trans equation.symm
  · rw [if_pos odd]
    obtain ⟨right, left⟩ := gindexSibling_odd odd
    rw [left]
    exact (congrArg (fun at_ => combine (progressiveMixedReading chunks word (2 * ((index >>> level) / 2)))
      (progressiveMixedReading chunks word at_)) right).trans equation.symm

/-- Siblings on a readable progressive branch remain readable, including the final mixed-in word. -/
theorem progressive_mixed_branch_sibling {chunks : Array Bytes} {index step : Nat}
    (named : 2 ≤ index) (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (position : ProgressivePosition chunks.size index (Nat.log2 index - 1) 0 0)
    (below : step < Nat.log2 index) :
    let sibling := gindexSibling (index >>> step)
    sibling = 3 ∨ (2 ≤ sibling ∧ gindexBit sibling (Nat.log2 sibling - 1) = false ∧
      ProgressivePosition chunks.size sibling (Nat.log2 sibling - 1) 0 0) := by
  -- All siblings except the mixing word are positions within the same finite progressive tree.
  dsimp only
  have basic := mixed_branch_sibling named leftSide (Nat.le_refl (Nat.log2 index - 1)) below
  rcases basic with word | ⟨positive, siblingTurn, _⟩
  · exact Or.inl word
  · refine Or.inr ⟨positive, siblingTurn, ?_⟩
    have currentPositive := path_shift_positive (by omega : 1 ≤ index) below
    have currentDepth := levelOf_shift (by omega : 1 ≤ index) step (by omega)
    have siblingDepth := levelOf_sibling currentPositive
    -- An upward branch remains readable at every ancestor of its original progressive position.
    have current := progressivePosition_ancestor position step (by omega)
    -- A sibling other than the mixing word must still have a position inside the contents tree.
    have stillInside : 0 < Nat.log2 index - 1 - step := by
      by_cases positiveRemaining : 0 < Nat.log2 index - 1 - step
      · exact positiveRemaining
      · have one : Nat.log2 (index >>> step) = 1 := by
          change Nat.log2 (index >>> step) + step = Nat.log2 index at currentDepth
          omega
        have siblingOne : Nat.log2 (gindexSibling (index >>> step)) = 1 := siblingDepth.trans one
        have turn := (gindexBit_shift_top (by omega : 1 ≤ index) below).trans leftSide
        have currentNotOdd : (index >>> step) % 2 ≠ 1 := by
          have raw : (index >>> step).testBit 0 = false := by
            simpa only [gindexBit, one, Nat.sub_self] using turn
          intro odd
          simp [Nat.testBit_zero, odd] at raw
        have siblingNotOdd : gindexSibling (index >>> step) % 2 ≠ 1 := by
          simpa [gindexBit, Nat.testBit_zero, siblingOne] using siblingTurn
        have flips := gindexSibling_parity (index >>> step)
        have sides := Nat.mod_two_eq_zero_or_one (index >>> step)
        omega
    have remaining : Nat.log2 index - 1 - step = (Nat.log2 index - 1 - step - 1) + 1 := by omega
    rw [remaining] at current
    have sibling := (progressivePosition_parent_sibling _ _ _ _ _ current).2
    have depth : Nat.log2 (gindexSibling (index >>> step)) - 1 =
        Nat.log2 index - 1 - step := by
      change Nat.log2 (index >>> step) + step = Nat.log2 index at currentDepth
      change Nat.log2 (gindexSibling (index >>> step)) = Nat.log2 (index >>> step) at siblingDepth
      omega
    rw [depth, remaining]
    exact sibling

/-- A constructed proof of packed progressive contents rebuilds the actual value root. -/
theorem buildProof_packed_progressive_rebuilds_root {shape : Desc} {value : Value}
    {chunks : Array Bytes} {word : Bytes} {index : Nat} {indices : List Nat}
    (layout : merkleLayout shape value = .ok (.packing chunks none (some word)))
    (indexed : getBranchIndices index = .ok indices)
    (leftSide : gindexBit index (Nat.log2 index - 1) = false)
    (position : ProgressivePosition chunks.size index (Nat.log2 index - 1) 0 0) :
    buildProof shape value index = .ok (indices.map (progressiveMixedReading chunks word)) ∧
    calculateMerkleRoot (progressiveMixedReading chunks word index)
      (indices.map (progressiveMixedReading chunks word)) index = hashTreeRoot shape value := by
  -- Readability is preserved along the whole branch, including spine terminators and zero padding.
  obtain ⟨spelled, named, nonzero⟩ := getBranchIndices_eq indexed
  have positive : 2 ≤ index := two_le_of_level_positive (depth := Nat.log2 index) rfl (by omega)
  -- Each helper is either an authenticated progressive position or the single final mixing word.
  have reads : ∀ at_ ∈ indices, nodeRoot shape value at_ =
      .ok (progressiveMixedReading chunks word at_) := by
    intro at_ member
    rw [spelled] at member
    obtain ⟨step, below, same⟩ := List.mem_map.mp member
    subst at_
    apply nodeRoot_packed_progressive layout _
    exact Or.inr (progressive_mixed_branch_sibling positive leftSide position (List.mem_range.mp below))
  constructor
  · simp [buildProof, indexed, mapM_of_ok_on _ _ indices reads, Bind.bind, Except.bind]
  · rw [branch_rebuilds_root_on_path
      (progressiveMixedReading_branch_left chunks word index positive leftSide position) indexed,
      hashTreeRoot_packed_progressive layout]
    simp [progressiveMixedReading]

/-- A progressive value's mixed-in word is authenticated by the complete spine beside it. -/
theorem buildProof_packed_progressive_word_rebuilds_root {shape : Desc} {value : Value}
    {chunks : Array Bytes} {word : Bytes}
    (layout : merkleLayout shape value = .ok (.packing chunks none (some word))) :
    buildProof shape value 3 = .ok [merkleizeProgressive chunks.toList] ∧
    calculateMerkleRoot word [merkleizeProgressive chunks.toList] 3 = hashTreeRoot shape value := by
  -- Position two names the whole spine, even when it is the empty zero terminator.
  have depth : Nat.log2 2 = 1 := rfl
  have read := nodeRoot_packed_progressive layout 2
    (Or.inr (Or.inr ⟨by decide, rfl, by simp [depth, ProgressivePosition]⟩))
  have branch : getBranchIndices 3 = .ok [2] := by with_unfolding_all rfl
  constructor
  · simp [buildProof, branch, read, progressiveMixedReading, progressiveReading, depth,
      Bind.bind, Except.bind, Pure.pure, Except.pure]
  · rw [hashTreeRoot_packed_progressive layout]
    rfl

end Ssz
