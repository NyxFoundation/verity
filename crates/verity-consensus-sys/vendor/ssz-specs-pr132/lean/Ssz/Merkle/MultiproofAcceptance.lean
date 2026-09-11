import Ssz.Merkle.Authentication
import Ssz.Merkle.MultiproofTrace

/-! Accepted multiproofs have complete nodes and precisely the canonical reconstruction shape. -/

namespace Ssz

/-- Acceptance validates every node width and reconstructs exactly the expected root. -/
theorem verifyMerkleMultiproof_eq_true_iff (leaves proof : List Bytes)
    (indices : List Nat) (root : Bytes) :
    verifyMerkleMultiproof leaves proof indices root = .ok true ↔
      root.size = bytesPerChunk ∧
      (∀ node ∈ leaves, node.size = bytesPerChunk) ∧
      (∀ node ∈ proof, node.size = bytesPerChunk) ∧
      calculateMultiMerkleRoot leaves proof indices = .ok root := by
  -- Check the root, claimed nodes, and helper nodes before comparing the reconstructed root.
  by_cases width : root.size = bytesPerChunk
  · cases leafCheck : checkChunks leaves with
    | error fault =>
      -- A reported width error contradicts the claim that all supplied leaves are complete.
      have invalid : ¬∀ node ∈ leaves, node.size = bytesPerChunk := by
        intro widths
        have := (checkChunks_ok_iff leaves).mpr widths
        rw [leafCheck] at this
        cases this
      simp [verifyMerkleMultiproof, checkChunk, width, leafCheck, invalid,
        Bind.bind, Except.bind]
    | ok checked =>
      cases checked
      have leafWidths := (checkChunks_ok_iff leaves).mp leafCheck
      cases proofCheck : checkChunks proof with
      | error fault =>
        -- The same equivalence rules out a successful answer after a malformed helper node.
        have invalid : ¬∀ node ∈ proof, node.size = bytesPerChunk := by
          intro widths
          have := (checkChunks_ok_iff proof).mpr widths
          rw [proofCheck] at this
          cases this
        simp [verifyMerkleMultiproof, checkChunk, width, leafCheck, proofCheck, invalid,
          Bind.bind, Except.bind]
      | ok checked =>
        cases checked
        have proofWidths := (checkChunks_ok_iff proof).mp proofCheck
        -- After all widths pass, acceptance is exactly equality with the computed root.
        cases reconstructed : calculateMultiMerkleRoot leaves proof indices <;>
          simp [verifyMerkleMultiproof, checkChunk, width, leafCheck, proofCheck,
            reconstructed, Bind.bind, Except.bind,
            Pure.pure, Except.pure]
        exact ⟨fun same => ⟨leafWidths, proofWidths, same⟩, fun valid => valid.2.2⟩
  · simp [verifyMerkleMultiproof, checkChunk, width, Bind.bind, Except.bind]

/-- Successful reconstruction fixes the counts, helper indices, and exact level fold. -/
theorem calculateMultiMerkleRoot_shape {leaves proof : List Bytes} {indices : List Nat}
    {root : Bytes} (built : calculateMultiMerkleRoot leaves proof indices = .ok root) :
    ∃ helpers, getHelperIndices indices = .ok helpers ∧ leaves.length = indices.length ∧
      proof.length = helpers.length ∧
      foldToRoot (multiproofHeight indices helpers)
        (indices.zip leaves ++ helpers.zip proof) = .ok root := by
  -- Accepted counts make both zipped sequences retain every position in the canonical frontier.
  unfold calculateMultiMerkleRoot at built
  split at built
  · simp [throw, Bind.bind, Except.bind] at built
  · rename_i lengths
    have lengths : leaves.length = indices.length := by simpa using lengths
    cases helperIndices : getHelperIndices indices with
    | error fault => simp [helperIndices, Bind.bind, Except.bind] at built
    | ok helpers =>
      simp only [helperIndices, Bind.bind, Except.bind] at built
      split at built
      · simp [throw] at built
      · rename_i proofLengths
        have proofLengths : proof.length = helpers.length := by simpa using proofLengths
        refine ⟨helpers, rfl, lengths, proofLengths, ?_⟩
        -- The validated counts make index projection recover the entire claim-plus-helper sequence.
        have positions : (indices.zip leaves ++ helpers.zip proof).map Prod.fst = indices ++ helpers := by
          simp [List.map_fst_zip, lengths, proofLengths]
        -- The initial maximum depth then depends only on those positions, not on the node bytes.
        have height := congrArg (fun xs => (xs.map levelOf).foldl max 0) positions
        simp only [List.map_map, Function.comp_def] at height
        simpa only [height, multiproofHeight] using built

end Ssz
