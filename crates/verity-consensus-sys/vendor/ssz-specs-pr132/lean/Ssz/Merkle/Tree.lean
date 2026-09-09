import Ssz.Hash.Sha256
import Ssz.Codec.Serialize
import Ssz.Merkle.Merkleize

/-! The two tree shapes SSZ merkleizes into, and the words a shape hashes itself against. -/

namespace Ssz

/-- The all-zero node. -/
def zeroChunk : Bytes :=
  -- Absent leaves contribute thirty-two zero bytes before any parent hash is computed.
  Array.replicate bytesPerChunk 0

/-- Folding two nodes into their parent. -/
def combine (left right : Bytes) : Bytes :=
  -- The hash input is the left child followed immediately by the right child.
  (Sha256.hash (ByteArray.mk (left ++ right))).data

/-- Cache depth for common capacities, with larger trees computed on demand. -/
abbrev maxZeroDepth : Nat :=
  -- Sixty-four cached levels cover common capacities without limiting larger mathematical trees.
  64

/--
Roots of the all-zero perfect trees up to a depth, each built from the one below.

Building each entry from its predecessor keeps the table linear in the depth.
Building each from scratch would make it exponential.
-/
def zeroRootsUpTo : Nat → Array Bytes
  -- The table opens with the zero node, which is the empty subtree of no depth.
  | 0 => #[zeroChunk]
  | depth + 1 =>
    let below := zeroRootsUpTo depth
    -- One level up, both children are the empty subtree of the level below.
    let deepest := below[depth]?.getD zeroChunk
    below.push (combine deepest deepest)

/--
Roots of the all-zero perfect trees, indexed by depth.

Index zero is the zero node.
Each following entry doubles the number of zero leaves covered.
-/
def zeroRoots : Array Bytes :=
  -- Build each cached depth once so empty subtrees can be answered by lookup.
  zeroRootsUpTo maxZeroDepth

/-- The table holds one entry per depth up to the one it was built for. -/
theorem zeroRootsUpTo_size (built : Nat) : (zeroRootsUpTo built).size = built + 1 := by
  induction built with
  -- The table opens with the zero node alone.
  | zero => rfl
  -- Every level after that pushes exactly one entry on the end.
  | succ built ih => simp [zeroRootsUpTo, ih]

/--
Every entry of the table is the zero subtree of its own depth.

This is what lets an empty subtree be answered by lookup.
The answer still means what the specification says it means.
-/
theorem zeroRootsUpTo_get (built : Nat) :
    ∀ depth, depth ≤ built →
      (zeroRootsUpTo built)[depth]?.getD zeroChunk = zeroRoot combine zeroChunk depth := by
  induction built with
  | zero =>
    -- Only depth zero fits, and the table opens with the zero node itself.
    intro depth holds
    have : depth = 0 := by omega
    subst this
    rfl
  | succ built ih =>
    intro depth holds
    have size : (zeroRootsUpTo built).size = built + 1 := zeroRootsUpTo_size built
    -- Either the entry was already there before this level, or it is the one just added.
    rcases Nat.lt_or_ge depth (built + 1) with below | at_top
    · -- A shallower entry was already in the table, and pushing on the end left it alone.
      have : depth ≠ (zeroRootsUpTo built).size := by omega
      simp only [zeroRootsUpTo, Array.getElem?_push, if_neg this]
      exact ih depth (by omega)
    · -- The new entry folds the previous one against itself, which is the level above.
      have top : depth = built + 1 := by omega
      subst top
      simp only [zeroRootsUpTo, Array.getElem?_push, if_pos size.symm, Option.getD_some, zeroRoot]
      -- The entry below it is already known to be the subtree one level down.
      rw [ih built (Nat.le_refl built)]

/-- Reading the table at any depth it covers gives the zero subtree of that depth. -/
theorem zeroRoots_get {depth : Nat} (covered : depth ≤ maxZeroDepth) :
    zeroRoots[depth]?.getD zeroChunk = zeroRoot combine zeroChunk depth :=
  -- The table was built through the requested depth, so its entry has the proved recursive meaning.
  zeroRootsUpTo_get maxZeroDepth depth covered

/-- Root of an all-zero subtree at any depth, using cached roots where available. -/
def zeroSubtree (depth : Nat) : Bytes :=
  -- SSZ capacities are natural numbers, so a cache boundary cannot limit the tree.
  match zeroRoots[depth]? with
  | some root => root
  | none => zeroRoot combine zeroChunk depth

/-- Cached and uncached zero roots both describe the same perfect tree. -/
theorem zeroSubtree_eq (depth : Nat) :
    zeroSubtree depth = zeroRoot combine zeroChunk depth := by
  -- Inside the cache, the table theorem identifies the entry.
  by_cases covered : depth ≤ maxZeroDepth
  · have present : depth < zeroRoots.size := by
      simpa [zeroRoots, zeroRootsUpTo_size] using Nat.lt_succ_of_le covered
    simpa [zeroSubtree, Array.getElem?_eq_getElem present] using zeroRoots_get covered
  · -- Beyond the cache, the recursive definition supplies the root directly.
    have absent : zeroRoots[depth]? = none := Array.getElem?_eq_none (by
      simp only [zeroRoots, zeroRootsUpTo_size]
      omega)
    simp [zeroSubtree, absent]

/-- Bytes right-padded to a node boundary and split into nodes. -/
def packBytes (data : Bytes) : Array Bytes :=
  -- Each position covers one node-width window, with only the last window needing zeros.
  Array.ofFn (n := (data.size + bytesPerChunk - 1) / bytesPerChunk) fun position =>
    let start := position.val * bytesPerChunk
    let piece := data.extract start (start + bytesPerChunk)
    piece ++ Array.replicate (bytesPerChunk - piece.size) 0

/-- Packing uses the smallest number of whole nodes that can hold the bytes. -/
@[simp] theorem packBytes_size (data : Bytes) :
    (packBytes data).size = (data.size + bytesPerChunk - 1) / bytesPerChunk := by
  -- One array position is allocated for each window, including a final partial window.
  simp only [packBytes, Array.size_ofFn]

/-- Every packed node has exactly the width of a hash operand. -/
theorem packBytes_chunk_size (data : Bytes) (i : Nat) (within : i < (packBytes data).size) :
    ((packBytes data)[i]).size = bytesPerChunk := by
  -- An extracted window is at most one node wide, and zeros fill exactly its missing bytes.
  simp only [packBytes, Array.getElem_ofFn, Array.size_append, Array.size_replicate,
    Array.size_extract]
  omega

/-- Bytes inside the payload retain their positions when divided into nodes. -/
theorem packBytes_get (data : Bytes) (i j : Nat) (node : i < (packBytes data).size)
    (position : j < bytesPerChunk) (payload : i * bytesPerChunk + j < data.size) :
    ((packBytes data)[i])[j]'(by rw [packBytes_chunk_size]; exact position) =
      data[i * bytesPerChunk + j] := by
  -- A position inside the original message lies in the extracted window, before its zero padding.
  simp only [packBytes, Array.getElem_ofFn]
  rw [Array.getElem_append_left (by simp only [Array.size_extract]; omega)]
  simp only [Array.getElem_extract]

/-- Positions beyond the original message are zero-filled in the final node. -/
theorem packBytes_padding (data : Bytes) (i j : Nat) (node : i < (packBytes data).size)
    (position : j < bytesPerChunk) (padding : data.size ≤ i * bytesPerChunk + j) :
    ((packBytes data)[i])[j]'(by rw [packBytes_chunk_size]; exact position) = 0 := by
  -- After the extracted bytes, the remaining positions belong to the explicit zero suffix.
  simp only [packBytes, Array.getElem_ofFn]
  rw [Array.getElem_append_right (by simp only [Array.size_extract]; omega)]
  exact Array.getElem_replicate _

/--
Root of the perfect tree of a given depth, over nodes starting at a position.

Positions past the data are zero.
A subtree wholly past it is answered from the table rather than folded.
-/
def subtreeAt (chunks : Array Bytes) (depth start : Nat) : Bytes :=
  -- A subtree beginning past the data holds nothing but zeros.
  if start ≥ chunks.size then zeroSubtree depth
  else
    match depth with
    -- At the bottom the node is the leaf itself.
    | 0 => padded zeroChunk chunks start
    -- Higher up the two halves are folded, the right one starting a half-width along.
    | d + 1 => combine (subtreeAt chunks d start) (subtreeAt chunks d (start + 2 ^ d))

/--
Cached subtree evaluation agrees with the full recursive tree.

An empty subtree may therefore use a table lookup without changing its mathematical root.
-/
theorem subtreeAt_eq_subtreeRoot (chunks : Array Bytes) :
    ∀ depth, ∀ start,
      subtreeAt chunks depth start
        = subtreeRoot combine depth (fun i => padded zeroChunk chunks (start + i)) := by
  intro depth
  -- The walk descends a level at a time, so the starting position stays quantified over.
  induction depth with
  | zero =>
    intro start
    unfold subtreeAt
    -- Either the leaf lies past the data, or it is read from it.
    split
    · -- Past the data, the lookup answers with the zero node the leaf would have read.
      rename_i past
      rw [zeroSubtree_eq]
      have absent : chunks[start]? = none := Array.getElem?_eq_none (by omega)
      simp [subtreeRoot, zeroRoot, padded, absent]
    · simp [subtreeRoot, padded]
  | succ depth ih =>
    intro start
    unfold subtreeAt
    -- Either the whole subtree lies past the data, or part of it does not.
    split
    · -- A subtree wholly past the data is the empty tree of its depth, by the lemma above.
      rename_i past
      rw [zeroSubtree_eq]
      exact (subtreeRoot_padded_past_data combine zeroChunk (depth + 1) chunks (by omega)).symm
    · -- Otherwise both sides fold the same two halves, and the shift lines up.
      show combine (subtreeAt chunks depth start) (subtreeAt chunks depth (start + 2 ^ depth)) = _
      -- Each half is the same claim one level down, which the hypothesis gives twice.
      rw [subtreeRoot, ih start, ih (start + 2 ^ depth)]
      -- The right half starts a half-width along, which is the shift the fold applies.
      have shift : (fun i => padded zeroChunk chunks (start + 2 ^ depth + i))
          = fun i => padded zeroChunk chunks (start + (i + 2 ^ depth)) := by
        funext position
        congr 1
        omega
      rw [shift]

/--
Root of the bounded tree over a node sequence.

The capacity is the declared one where the type has a limit.
Where it has none, the capacity is the node count.
-/
def merkleizeBounded (chunks : Array Bytes) (limit : Option Nat) : Except Err Bytes := do
  match limit with
  | some capacity =>
    -- A capacity under the data would silently drop nodes.
    if capacity < chunks.size then throw (.merkleizeLimit chunks.size capacity)
    return subtreeAt chunks (depthFor capacity) 0
  | none => return subtreeAt chunks (depthFor chunks.size) 0

/--
A subtree is its two halves combined, whatever sits under it.

This is the law a branch rests on: knowing one child and its sibling gives the parent.
-/
theorem subtreeAt_split (chunks : Array Bytes) {depth : Nat}
    (start : Nat) :
    subtreeAt chunks (depth + 1) start
      = combine (subtreeAt chunks depth start) (subtreeAt chunks depth (start + 2 ^ depth)) := by
  -- Either part of the subtree reaches the data, or none of it does.
  rw [subtreeAt]
  split
  · -- Wholly past the data, so both halves read the zero subtree one level down.
    rename_i past
    rw [zeroSubtree_eq]
    have rightPast : start + 2 ^ depth ≥ chunks.size :=
      Nat.le_trans past (Nat.le_add_right start (2 ^ depth))
    have leftZero : subtreeAt chunks depth start = zeroSubtree depth := by
      rw [subtreeAt.eq_def, if_pos past]
    have rightZero : subtreeAt chunks depth (start + 2 ^ depth)
        = zeroSubtree depth := by
      rw [subtreeAt.eq_def, if_pos rightPast]
    rw [leftZero, rightZero, zeroSubtree_eq]
    rfl
  · rfl

/-- The bounded tree over a whole node array, read as the specification states it. -/
theorem subtreeAt_zero_eq (chunks : Array Bytes) {depth : Nat} :
    subtreeAt chunks depth 0 = subtreeRoot combine depth (padded zeroChunk chunks) := by
  rw [subtreeAt_eq_subtreeRoot chunks depth 0]
  -- Starting at position zero, the shift the walk applies is no shift at all.
  congr 1
  funext position
  congr 1
  omega

/--
Merkleizing one window of the leaves gives the subtree standing over that window.

A proof reads a run of leaves rather than the whole tree, and this is why doing so is sound.
-/
theorem merkleizeBounded_window (chunks : Array Bytes) {depth : Nat}
    (start : Nat) :
    merkleizeBounded (chunks.extract start (start + 2 ^ depth)) (some (2 ^ depth))
      = .ok (subtreeAt chunks depth start) := by
  -- A window is no wider than it was asked to be, so the capacity check passes.
  have span : (chunks.extract start (start + 2 ^ depth)).size ≤ 2 ^ depth := by
    rw [Array.size_extract]
    generalize 2 ^ depth = width
    omega
  simp only [merkleizeBounded, if_neg (Nat.not_lt.mpr span), depthFor_pow, pure, Except.pure,
    Except.ok.injEq]
  -- Both sides are the specification's tree, over leaf supplies that have yet to be compared.
  rw [subtreeAt_zero_eq _, subtreeAt_eq_subtreeRoot chunks depth start]
  refine subtreeRoot_congr combine depth ?_
  intro position below
  simp only [padded, Array.getElem?_extract]
  split
  · rfl
  · -- A position the window does not reach is one the whole tree reads as zero too.
    rename_i outside
    revert below outside
    generalize 2 ^ depth = width
    intro below outside
    have absent : chunks[start + position]? = none := Array.getElem?_eq_none (by omega)
    simp [absent]

/--
Merkleizing a short buffer gives the root of one filled out with zeros.

This is the shortcut every implementation takes, stated about the code that takes it.
-/
theorem merkleizeBounded_append_zeros (chunks : Array Bytes)
    {count capacity : Nat}
    (room : chunks.size + count ≤ capacity) :
    merkleizeBounded (chunks ++ Array.replicate count zeroChunk) (some capacity)
      = merkleizeBounded chunks (some capacity) := by
  -- The padded buffer is longer, but still inside the capacity, so neither side refuses.
  have grown : (chunks ++ Array.replicate count zeroChunk).size = chunks.size + count := by
    simp
  simp only [merkleizeBounded, grown, if_neg (by omega : ¬ capacity < chunks.size + count),
    if_neg (by omega : ¬ capacity < chunks.size)]
  -- Both roots are then the specification's, over leaf supplies already known equal.
  rw [subtreeAt_zero_eq _, subtreeAt_zero_eq _, padded_append_zeros]

/-- An empty buffer merkleizes to the zero subtree of the width it was given. -/
theorem merkleizeBounded_empty {capacity : Nat} :
    merkleizeBounded #[] (some capacity)
      = .ok (zeroRoot combine zeroChunk (depthFor capacity)) := by
  -- An empty buffer is under any capacity, so the merkleizer answers rather than refusing.
  simp only [merkleizeBounded, Array.size_empty, if_neg (by omega : ¬ capacity < 0)]
  -- Every leaf then reads as zero, which is the constant supply the table entry stands for.
  rw [subtreeAt_zero_eq _, ← subtreeRoot_const_zero combine zeroChunk]
  rfl

/--
Root of the progressive tree over a node sequence, per EIP-7916.

A right-leaning spine of binary subtrees, closed by a zero node.
Successive levels hold 1, 4, 16, and then 64 nodes, so capacity grows with the data.

    root
     +-- chunks 0 ..< 1, as a binary subtree of width 1
     `-- everything past them
          +-- chunks 1 ..< 5, as a binary subtree of width 4
          `-- everything past them
               +-- chunks 5 ..< 21, as a binary subtree of width 16
               `-- the zero node that closes the spine

A node keeps its index as later ones are appended.
Its branch must still be updated to authenticate the new root after an append.
-/
def merkleizeProgressive (chunks : List Bytes) (level : Nat := 0) : Bytes :=
  -- An exhausted input closes the spine with a plain zero node, not a zero subtree.
  if chunks.isEmpty then zeroChunk
  else
    let width := 4 ^ level
    -- Left child: this level's nodes as a binary subtree, zero-padded to the level width.
    let here := subtreeAt (chunks.take width).toArray (depthFor width) 0
    -- Right child: everything past this level, in a level four times as wide.
    combine here (merkleizeProgressive (chunks.drop width) (level + 1))
termination_by chunks.length
decreasing_by
  have : 0 < 4 ^ level := Nat.pos_of_neZero (4 ^ level)
  simp only [List.length_drop]
  have : 0 < chunks.length := by
    cases chunks with
    | nil => simp_all
    | cons _ _ => simp
  omega

/-- The executable progressive tree agrees with the hash-independent definition. -/
theorem merkleizeProgressive_eq (chunks : List Bytes) (level : Nat) :
    merkleizeProgressive chunks level = progressiveRoot combine zeroChunk chunks level := by
  -- Both definitions consume one level's leaves before continuing down the spine.
  induction chunks, level using merkleizeProgressive.induct with
  | case1 chunks level empty =>
    simp [merkleizeProgressive, progressiveRoot, empty]
  | case2 chunks level nonempty width ih =>
    rw [merkleizeProgressive, progressiveRoot, if_neg nonempty, if_neg nonempty]
    -- The bounded subtree equivalence is valid at every depth.
    dsimp only
    rw [subtreeAt_zero_eq, ih]

/--
Root of the progressive tree, starting from a level of a given width.

The width is a power of four.
It is the capacity of whichever level the caller reached.
-/
def merkleizeProgressiveFrom (chunks : Array Bytes) (width : Nat) : Bytes :=
  -- A power-of-four width has twice its progressive level as its binary logarithm.
  merkleizeProgressive chunks.toList (Nat.log2 width / 2)

/-- Hashing a subtree root against the word its shape mixes in, as the right child. -/
def mixIn (root word : Bytes) : Bytes :=
  -- The metadata word occupies the right child, so its position is unambiguous.
  combine root word

/--
The low 256 bits of a count, written as one little-endian node.
Counts below 2^256 are represented exactly.
Larger natural numbers are truncated and fall outside the length-binding theorem's domain.
-/
def lengthWord (count : Nat) : Bytes :=
  -- Thirty-two little-endian bytes retain exactly the low 256 bits.
  uintBytes bytesPerChunk count

/-- The layout a progressive container mixes in, one bit per position, lowest first. -/
def activeFieldsWord (active : List Bool) : Bytes :=
  -- The same low-bit-first packing as bitfields, padded to one complete node.
  packBits active.toArray bytesPerChunk

end Ssz
