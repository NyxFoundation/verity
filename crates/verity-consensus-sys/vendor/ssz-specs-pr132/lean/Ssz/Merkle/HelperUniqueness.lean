import Ssz.Merkle.HelperFrontier

/-! A multiproof assigns exactly one supplied node to each claimed or helper index. -/

namespace Ssz

private theorem dedup_info (indices : List Nat) :
    List.Sublist indices.eraseDups indices ∧ indices.eraseDups.Nodup := by
  -- Keep the first occurrence and remove all equal later entries before recursing.
  cases indices with
  | nil => simp
  | cons head rest =>
    rw [List.eraseDups_cons]
    -- Removing later copies makes the retained head distinct from every recursively retained element.
    have filtered := dedup_info (rest.filter fun item => !item == head)
    constructor
    · exact List.Sublist.cons_cons _ (filtered.1.trans (List.filter_sublist))
    · rw [List.nodup_cons]
      refine ⟨?_, filtered.2⟩
      simp
termination_by indices.length
decreasing_by have := List.length_filter_le (fun item => !item == head) rest; simp_all; omega

/-- Duplicate-index rejection leaves each claim with exactly one supplied value. -/
theorem rejectRelated_nodup {indices : List Nat}
    (accepted : rejectRelated indices = .ok ()) : indices.Nodup := by
  -- An unchanged length after deduplication means no claim was repeated.
  by_cases same : indices.eraseDups.length = indices.length
  · have unchanged := (dedup_info indices).1.eq_of_length same
    rw [← unchanged]
    exact (dedup_info indices).2
  · cases indices with
    | nil => simp at same
    | cons head rest =>
      simp only [List.length_cons] at same
      simp [rejectRelated, same, Bind.bind, Except.bind, throw, throwThe, MonadExceptOf.throw] at accepted

/-- Sorting the deduplicated helper frontier preserves uniqueness. -/
theorem getHelperIndices_nodup {indices helpers : List Nat}
    (built : getHelperIndices indices = .ok helpers) : helpers.Nodup := by
  -- Filtering and sorting preserve the uniqueness established by deduplication.
  unfold getHelperIndices at built
  cases accepted : rejectRelated indices with
  | error fault => simp [accepted, Bind.bind, Except.bind] at built
  | ok checked =>
    cases checked
    cases collected : collectPathIndices indices with
    | error fault => simp [accepted, collected, Bind.bind, Except.bind] at built
    | ok paths =>
      simp [accepted, collected, Bind.bind, Except.bind, pure, Except.pure] at built
      subst helpers
      apply List.Perm.nodup (List.mergeSort_perm _ _).symm
      exact (dedup_info (paths.map gindexSibling)).2.filter _

/-- Claimed nodes and required helpers never assign two values to the same index. -/
theorem getHelperIndices_all_nodup {indices helpers : List Nat}
    (built : getHelperIndices indices = .ok helpers) : (indices ++ helpers).Nodup := by
  -- Claims lie on their own paths, whereas helpers are explicitly excluded from every claim path.
  obtain ⟨_, valid, frontier⟩ := getHelperIndices_frontier built
  have claims : indices.Nodup := by
    unfold getHelperIndices at built
    cases accepted : rejectRelated indices with
    | error fault => simp [accepted, Bind.bind, Except.bind] at built
    | ok checked => cases checked; exact rejectRelated_nodup accepted
  -- Uniqueness of the combined frontier also requires the claim and helper sets to be disjoint.
  rw [List.nodup_append]
  refine ⟨claims, getHelperIndices_nodup built, ?_⟩
  intro claim claimed helper supplied same
  subst helper
  exact (frontier claim |>.mp supplied).2 (claim_mem_paths valid claimed)

end Ssz
