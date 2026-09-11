import Ssz.Codec.BindingCongruence

/-! Successful value computations expose their layouts without assuming layout injectivity. -/

namespace Ssz
open CommitmentTree

/-- A successful computation uses every leaf its actual layout materializes. -/
theorem valueTreeAt_view {budget : Nat} {shape : Desc} {value : Value} {tree : CommitmentTree}
    (success : valueTreeAt (budget + 1) shape value = .ok tree) :
    ∃ layout trees, merkleLayout shape value = .ok layout ∧
      layoutTreesAt budget layout = .ok trees ∧
      (∀ capacity, layout.limit = some capacity → trees.size ≤ capacity) ∧
      tree = (content trees layout.limit).withMixin layout.mixin := by
  -- Overall success excludes both an invalid layout and a failed nested computation.
  cases laid : merkleLayout shape value with
  | error fault => simp [valueTreeAt, laid, Bind.bind, Except.bind] at success
  | ok layout =>
    -- Each selected position must have produced its own finite computation.
    cases materialized : layoutTreesAt budget layout with
    | error fault => simp [valueTreeAt, laid, materialized, Bind.bind, Except.bind] at success
    | ok trees =>
      refine ⟨layout, trees, rfl, materialized, ?_, ?_⟩
      -- A bounded tree can succeed only when its capacity includes every materialized position.
      · intro capacity named
        simp only [valueTreeAt, laid, materialized, named, Bind.bind, Except.bind] at success
        split at success
        · simp only [throw, throwThe, MonadExceptOf.throw] at success
          cases success
        · omega
      -- The successful result is the selected contents tree followed by its optional mixing word.
      · cases limit : layout.limit with
        | none =>
          simpa [valueTreeAt, laid, materialized, limit, Bind.bind, Except.bind,
            pure, Except.pure, content] using success.symm
        | some capacity =>
          simp only [valueTreeAt, laid, materialized, limit, Bind.bind, Except.bind] at success
          split at success
          · simp only [throw, throwThe, MonadExceptOf.throw] at success
            cases success
          · simpa [pure, Except.pure, Except.bind, content, limit] using success.symm

/-- An established layout gives the exact successful finite computation. -/
theorem valueTreeAt_of_layout {budget : Nat} {shape : Desc} {value : Value}
    {layout : MerkleLayout} {trees : Array CommitmentTree}
    (laid : merkleLayout shape value = .ok layout)
    (materialized : layoutTreesAt budget layout = .ok trees)
    (room : ∀ capacity, layout.limit = some capacity → trees.size ≤ capacity) :
    valueTreeAt (budget + 1) shape value =
      .ok ((content trees layout.limit).withMixin layout.mixin) := by
  -- A successful layout and complete leaf materialization leave only the capacity choice.
  cases limit : layout.limit with
  | none => simp [valueTreeAt, laid, materialized, limit, Bind.bind, Except.bind,
      pure, Except.pure, content]
  -- The supplied capacity bound excludes the only remaining tree-construction error.
  | some capacity =>
    simp [valueTreeAt, laid, materialized, limit, Bind.bind, Except.bind,
      pure, Except.pure, content, Nat.not_lt.mpr (room capacity limit)]

end Ssz
