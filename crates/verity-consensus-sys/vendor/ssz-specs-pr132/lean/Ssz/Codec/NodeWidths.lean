import Ssz.Codec.ProofBudget
import Ssz.Merkle.Widths

/-! Every successful Merkle read returns one complete 32-byte node. -/

namespace Ssz

/-- Packed leaves and optional mixing words are complete SSZ nodes. -/
def LayoutWidth (layout : MerkleLayout) : Prop :=
  (∀ chunks, layout.leaves = .packed chunks → ∀ chunk ∈ chunks, chunk.size = bytesPerChunk) ∧
  ∀ word, layout.mixin = some word → word.size = bytesPerChunk

private theorem packed_nodes_width (data : Bytes) :
    ∀ chunk ∈ packBytes data, chunk.size = bytesPerChunk := by
  -- Packing pads even the final partial chunk to one complete hash operand.
  intro chunk member
  obtain ⟨index, inside, rfl⟩ := Array.mem_iff_getElem.mp member
  exact packBytes_chunk_size data index inside

private theorem packing_width (data : Bytes) (limit : Option Nat) (mixin : Option Bytes)
    (mixed : ∀ word, mixin = some word → word.size = bytesPerChunk) :
    LayoutWidth (.packing (packBytes data) limit mixin) := by
  -- Packed leaves and the optional word are separate complete nodes.
  constructor
  · intro chunks same
    simp [MerkleLayout.packing] at same
    subst chunks
    exact packed_nodes_width data
  · exact mixed

private theorem nesting_width (values : List (Option (Desc × Value)))
    (limit : Option Nat) (mixin : Option Bytes)
    (mixed : ∀ word, mixin = some word → word.size = bytesPerChunk) :
    LayoutWidth (.nesting values limit mixin) := by
  -- Nested leaf widths are supplied by recursive rooting rather than by this layout.
  constructor
  · intro chunks same
    cases same
  · exact mixed

private theorem fixedLeaf_width (shape : Desc) (value : Value) (layout : MerkleLayout)
    (success : fixedLeaf shape value = .ok layout) : LayoutWidth layout := by
  -- Fixed encodings are split into padded nodes and carry no mixing word.
  cases h : serialize shape value with
  | error e => simp [fixedLeaf, h, Bind.bind, Except.bind] at success
  | ok bytes =>
    simp [fixedLeaf, h, Bind.bind, Except.bind, Pure.pure, Except.pure] at success
    subst layout
    exact packing_width bytes _ none (by simp)

private theorem sequenceLayout_width (element : Desc) (values : List Value)
    (limit : Option Nat) (mixin : Option Bytes) (layout : MerkleLayout)
    (mixed : ∀ word, mixin = some word → word.size = bytesPerChunk)
    (success : sequenceLayout element values limit mixin = .ok layout) : LayoutWidth layout := by
  -- Basic elements pack bytes, while composite elements defer to their own roots.
  unfold sequenceLayout at success
  split at success
  · cases h : serializeEach element values with
    | error e => simp [h, Bind.bind, Except.bind] at success
    | ok bytes =>
      simp [h, Bind.bind, Except.bind, Pure.pure, Except.pure] at success
      subst layout
      exact packing_width _ _ _ mixed
  · simp [Pure.pure, Except.pure] at success
    subst layout
    exact nesting_width _ _ _ mixed

/-- Every successful layout supplies complete leaves and mixing words. -/
theorem merkleLayout_width (shape : Desc) (value : Value) (layout : MerkleLayout)
    (success : merkleLayout shape value = .ok layout) : LayoutWidth layout := by
  -- Every scalar, bitfield, and protocol mixing word occupies complete hash nodes.
  cases shape <;> cases value <;> simp only [merkleLayout] at success
  all_goals try solve | cases success
  -- Fixed scalar encodings are padded into complete nodes before any tree reads occur.
  all_goals try solve | exact fixedLeaf_width _ _ _ success
  all_goals try solve
    | simp only [Bind.bind, Except.bind, Pure.pure, Except.pure] at success
      subst layout
      apply packing_width
      intro word same
      simp at same
      subst word
      simp
  all_goals try solve
    | split at success <;>
        simp [Bind.bind, Except.bind, Pure.pure, Except.pure, throw, throwThe,
          MonadExceptOf.throw] at success
      subst layout
      apply packing_width
      intro word same
      simp at same
      subst word
      simp
  all_goals try solve
    | apply sequenceLayout_width _ _ _ _ _ (by intro word same; simp at same; subst word; simp) success
    | split at success <;>
        simp [Bind.bind, Except.bind, Pure.pure, Except.pure, throw, throwThe,
          MonadExceptOf.throw] at success
      apply sequenceLayout_width _ _ _ _ _ (by intro word same; simp at same; subst word; simp) success
  -- A fixed bit count determines byte packing, whose final partial node is zero-padded.
  case bitVector.bits length data =>
    split at success <;>
      simp [Bind.bind, Except.bind, Pure.pure, Except.pure, throw, throwThe,
        MonadExceptOf.throw] at success
    subst layout
    exact packing_width _ _ none (by simp)
  case progressiveBitList.bits data =>
    simp [Pure.pure, Except.pure] at success
    subst layout
    exact packing_width _ _ (some (lengthWord data.size)) (by intro word same; have eq := Option.some.inj same; rw [← eq]; simp)
  case vector.seq element length elements =>
    split at success <;> try simp [Bind.bind, Except.bind, throw, throwThe, MonadExceptOf.throw] at success
    exact sequenceLayout_width element elements (some length) none layout (by simp) success
  case list.seq element limit elements =>
    split at success <;> try simp [Bind.bind, Except.bind, throw, throwThe, MonadExceptOf.throw] at success
    exact sequenceLayout_width element elements (some limit) (some (lengthWord elements.length)) layout
      (by intro word same; have eq := Option.some.inj same; rw [← eq]; simp) success
  case container.seq names fields elements =>
    split at success <;>
      simp [Bind.bind, Except.bind, Pure.pure, Except.pure, throw, throwThe,
        MonadExceptOf.throw] at success
    subst layout
    exact nesting_width _ _ none (by simp)
  -- The active-position mask is a complete mixing word even when its logical layout is shorter.
  case progressiveContainer.seq active names fields elements =>
    cases hs : layoutSlots active fields elements with
    | error e => simp [hs, Bind.bind, Except.bind] at success
    | ok slots =>
      simp [hs, Bind.bind, Except.bind, Pure.pure, Except.pure] at success
      subst layout
      exact nesting_width _ _ (some (activeFieldsWord active)) (by intro word same; have eq := Option.some.inj same; rw [← eq]; simp)
  -- A permitted selector is extended from its one-byte encoding to a complete 32-byte word.
  case compatibleUnion.union selectors options selector data =>
    cases chosen : lookupOption selectors options selector with
    | error e => simp [chosen, Bind.bind, Except.bind] at success
    | ok child =>
      cases hw : selectorWord selector with
      | error e => simp [chosen, hw, Bind.bind, Except.bind] at success
      | ok word =>
        simp [chosen, hw, Bind.bind, Except.bind, Pure.pure, Except.pure] at success
        subst layout
        apply nesting_width
        intro actual same
        have eq := Option.some.inj same
        subst actual
        unfold selectorWord at hw
        split at hw <;> simp [Bind.bind, Except.bind, Pure.pure, Except.pure,
          throw, throwThe, MonadExceptOf.throw] at hw
        subst word
        exact lengthWord_size selector

private theorem mapM_width {α : Type} (f : α → Except Err Bytes)
    (widths : ∀ value node, f value = .ok node → node.size = bytesPerChunk)
    (values : List α) (nodes : List Bytes) (success : values.mapM f = .ok nodes) :
    ∀ node ∈ nodes, node.size = bytesPerChunk := by
  -- A successful traversal contributes one complete root for each retained value.
  induction values generalizing nodes with
  | nil =>
    simp [Pure.pure, Except.pure] at success
    subst nodes
    simp
  | cons value values ih =>
    cases head : f value with
    | error e => simp [List.mapM_cons, head, Bind.bind, Except.bind] at success
    | ok node =>
      cases tail : values.mapM f with
      | error e => simp [List.mapM_cons, head, tail, Bind.bind, Except.bind] at success
      | ok others =>
        simp [List.mapM_cons, head, tail, Bind.bind, Except.bind, Pure.pure, Except.pure] at success
        subst nodes
        intro result member
        rcases List.mem_cons.mp member with rfl | member
        · exact widths value _ head
        · exact ih others tail result member

private theorem layoutChunksAt_width (budget : Nat) (layout : MerkleLayout)
    (layoutWidth : LayoutWidth layout)
    (rootWidth : ∀ shape value node, hashTreeRootAt budget shape value = .ok node →
      node.size = bytesPerChunk)
    (start : Nat) (stop : Option Nat) (chunks : Array Bytes)
    (success : layoutChunksAt budget layout start stop = .ok chunks) :
    ∀ chunk ∈ chunks, chunk.size = bytesPerChunk := by
  -- Extracting packed leaves preserves width, and nested leaves use their successful root widths.
  cases leaves : layout.leaves with
  | packed data =>
    simp [layoutChunksAt, leaves, Pure.pure, Except.pure] at success
    subst chunks
    intro chunk member
    obtain ⟨i, inside, same⟩ := Array.mem_extract_iff_getElem.mp member
    exact layoutWidth.1 data leaves chunk (same ▸ Array.getElem_mem _)
  | nested slots =>
    let f : Option (Desc × Value) → Except Err Bytes := fun slot => match slot with
      | none => .ok zeroChunk
      | some (shape, value) => hashTreeRootAt budget shape value
    simp only [layoutChunksAt, leaves] at success
    change (do
      let nodes ← ((slots.drop start).take ((stop.getD slots.length) - start)).mapM f
      pure nodes.toArray) = .ok chunks at success
    cases h : ((slots.drop start).take ((stop.getD slots.length) - start)).mapM f with
    | error e => simp [h, Bind.bind, Except.bind] at success
    | ok nodes =>
      simp [h, Bind.bind, Except.bind, Pure.pure, Except.pure] at success
      subst chunks
      -- Every occupied slot contributes a complete child root, and every inactive slot contributes a complete zero node.
      have each := mapM_width f (by
        intro slot node made
        cases slot with
        | none => cases made; exact zeroChunk_size
        | some pair => exact rootWidth pair.1 pair.2 node made) _ _ h
      simpa using each

/-- Every successful value root is a complete hash node, even without declaration assumptions. -/
theorem hashTreeRootAt_size (budget : Nat) (shape : Desc) (value : Value) (root : Bytes)
    (success : hashTreeRootAt budget shape value = .ok root) : root.size = bytesPerChunk := by
  -- Each nesting level roots complete leaves before building its bounded tree or progressive spine.
  induction budget generalizing shape value root with
  | zero => simp [hashTreeRootAt] at success
  | succ budget ih =>
    cases laid : merkleLayout shape value with
    | error e => simp [hashTreeRootAt, laid, Bind.bind, Except.bind] at success
    | ok layout =>
      cases materialized : layoutChunksAt budget layout with
      | error e => simp [hashTreeRootAt, laid, materialized, Bind.bind, Except.bind] at success
      | ok chunks =>
        -- The successful materialization carries complete widths into either merkleization shape.
        have complete := layoutChunksAt_width budget layout (merkleLayout_width shape value layout laid)
          ih 0 none chunks materialized
        simp only [hashTreeRootAt, laid, materialized, Bind.bind, Except.bind] at success
        cases limit : layout.limit with
        | none =>
          cases mixed : layout.mixin <;>
            simp [limit, mixed, Pure.pure, Except.pure] at success
          all_goals subst root; simp
        | some capacity =>
          cases made : merkleizeBounded chunks (some capacity) with
          | error e => simp [limit, made] at success
          | ok contents =>
            cases mixed : layout.mixin <;>
              simp [limit, made, mixed, Pure.pure, Except.pure] at success
            · subst root
              exact merkleizeBounded_size complete made
            · subst root
              exact mixIn_size _ _

/-- Every successfully computed SSZ root occupies exactly 32 bytes. -/
theorem hashTreeRoot_size (shape : Desc) (value : Value) (root : Bytes)
    (success : hashTreeRoot shape value = .ok root) : root.size = bytesPerChunk := by
  -- The public root uses the same finite rooting procedure at the type's depth.
  exact hashTreeRootAt_size shape.nesting shape value root success

private theorem boundedNode_size (budget : Nat) (layout : MerkleLayout)
    (layoutWidth : LayoutWidth layout)
    (nestedWidth : ∀ shape value index node, nodeRootAt budget shape value index = .ok node →
      node.size = bytesPerChunk)
    (index depth start capacity : Nat) (node : Bytes)
    (read : boundedNode budget layout index depth start capacity = .ok node) :
    node.size = bytesPerChunk := by
  -- Binary intervals hash complete materialized leaves, and deeper reads belong to nested values.
  simp only [boundedNode] at read
  split at read
  · rename_i inside
    cases materialized : layoutChunksAt budget layout
        (start + gindexBelow index depth * (nextPow2 capacity >>> depth))
        (some (start + gindexBelow index depth * (nextPow2 capacity >>> depth) +
          (nextPow2 capacity >>> depth))) with
    | error e => simp [materialized, Bind.bind, Except.bind] at read
    | ok chunks =>
      simp only [materialized, Bind.bind, Except.bind] at read
      exact merkleizeBounded_size
        (layoutChunksAt_width budget layout layoutWidth (hashTreeRootAt_size budget) _ _ chunks materialized) read
  · cases leaves : layout.leaves with
    | packed data => simp [leaves, throw, throwThe, MonadExceptOf.throw] at read
    | nested slots =>
      simp only [leaves] at read
      split at read
      · exact nestedWidth _ _ _ node read
      · simp [throw, throwThe, MonadExceptOf.throw] at read

private theorem progressiveNode_size (budget : Nat) (layout : MerkleLayout)
    (layoutWidth : LayoutWidth layout)
    (nestedWidth : ∀ shape value index node, nodeRootAt budget shape value index = .ok node →
      node.size = bytesPerChunk)
    (index depth start capacity : Nat) (node : Bytes)
    (read : progressiveNode budget layout index depth start capacity = .ok node) :
    node.size = bytesPerChunk := by
  -- A spine root is a digest, while a turn into a level uses bounded-node width.
  simp only [progressiveNode] at read
  cases walk : spineWalk layout.leaves.count index depth start capacity with
  | error e => simp [walk, Bind.bind, Except.bind] at read
  | ok stop =>
    simp only [walk, Bind.bind, Except.bind] at read
    split at read
    · exact boundedNode_size budget layout layoutWidth nestedWidth _ _ _ _ node read
    · cases materialized : layoutChunksAt budget layout stop.leavesFrom with
      | error e => simp [materialized] at read
      | ok chunks =>
        simp [materialized, Pure.pure, Except.pure] at read
        subst node
        exact merkleizeProgressive_size _ _

/-- Every successful value-tree walk returns a complete hash node. -/
theorem nodeRootAt_size (budget : Nat) (shape : Desc) (value : Value) (index : Nat) (node : Bytes)
    (read : nodeRootAt budget shape value index = .ok node) : node.size = bytesPerChunk := by
  -- Root reads, mixing words, binary intervals, and nested descents each preserve node width.
  induction budget generalizing shape value index node with
  | zero => simp [nodeRootAt] at read
  | succ budget ih =>
    by_cases invalid : index < 1
    · simp [nodeRootAt, invalid, throw, throwThe, MonadExceptOf.throw, Bind.bind, Except.bind] at read
    -- The root path delegates to whole-value rooting before any interior position is considered.
    by_cases root : index = 1
    · simp [nodeRootAt, root] at read
      exact hashTreeRoot_size shape value node read
    cases laid : merkleLayout shape value with
    | error e => simp [nodeRootAt, invalid, root, laid, Bind.bind, Except.bind] at read
    | ok layout =>
      -- Direct mixing-word reads use the layout’s own width guarantee.
      have widths := merkleLayout_width shape value layout laid
      simp only [nodeRootAt, if_neg invalid, beq_iff_eq, if_neg root, laid, Bind.bind, Except.bind] at read
      cases hg : gindexLength index with
      | error e => simp [hg] at read
      | ok depth =>
        simp only [hg] at read
        cases mixed : layout.mixin with
        | none =>
          simp only [mixed] at read
          cases limit : layout.limit with
          | none =>
            simp only [limit] at read
            exact progressiveNode_size budget layout widths ih _ _ _ _ node read
          | some capacity =>
            simp only [limit] at read
            exact boundedNode_size budget layout widths ih _ _ _ _ node read
        | some word =>
          simp [mixed, Pure.pure, Except.pure, throw, throwThe, MonadExceptOf.throw,
            ] at read
          split at read
          · split at read
            · simp at read
              subst node
              exact widths.2 word mixed
            · cases read
          · cases limit : layout.limit with
            | none =>
              simp only [limit] at read
              exact progressiveNode_size budget layout widths ih _ _ _ _ node read
            | some capacity =>
              simp only [limit] at read
              exact boundedNode_size budget layout widths ih _ _ _ _ node read

/-- Every readable SSZ node occupies exactly 32 bytes. -/
theorem nodeRoot_size (shape : Desc) (value : Value) (index : Nat) (node : Bytes)
    (read : nodeRoot shape value index = .ok node) : node.size = bytesPerChunk := by
  -- The public walker selects a sufficient budget without changing the returned node.
  exact nodeRootAt_size (shape.nesting + index) shape value index node read

end Ssz
