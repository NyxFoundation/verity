import Ssz.Type.Default
import Ssz.Codec.Fits

/-! Defaults belong to their declared value domains. -/

namespace Ssz

/-- Every successful default is admissible, including defaults of nested fields. -/
theorem default_fits (shape : Desc) :
    ∀ value, shape.default = .ok value → Fits shape value := by
  -- Default construction and field collection are proved together, preserving both counts and domains.
  induction shape using Desc.default.induct
    (motive_2 := fun fields => ∀ values, Desc.defaultFields fields = .ok values →
      fields.length = values.length ∧ ∀ pair ∈ fields.zip values, Fits pair.1 pair.2) with
  -- False is always admitted by the boolean domain.
  | case1 =>
    intro value made
    cases made
    exact .bool false
  -- Zero is below every positive power-of-two integer bound.
  | case2 width =>
    intro value made
    cases made
    exact .uint (Nat.pow_pos (by decide))
  -- Replicated bytes and bits have exactly the declared fixed length.
  | case3 length =>
    intro value made
    cases made
    exact .byteVector (by simp)
  | case4 length =>
    intro value made
    cases made
    exact .bitVector (by simp)
  -- Empty variable arrays satisfy every natural-number capacity.
  | case5 limit =>
    intro value made
    cases made
    exact .byteList (by simp)
  | case6 limit =>
    intro value made
    cases made
    exact .bitList (by simp)
  | case7 =>
    intro value made
    cases made
    exact .progressiveBitList
  -- An empty element sequence has no child-domain obligations.
  | case8 element limit =>
    intro value made
    cases made
    exact .list (by simp) (by simp)
  | case9 element =>
    intro value made
    cases made
    exact .progressiveList (by simp)
  | case10 element length ih =>
    intro value made
    -- Replication preserves admissibility of the element default at every position.
    cases one : element.default with
    | error fault => simp [Desc.default, one, Bind.bind, Except.bind] at made
    | ok held =>
      simp [Desc.default, one, Bind.bind, Except.bind, pure, Except.pure] at made
      subst value
      exact .vector (by simp) (fun item member => (List.mem_replicate.mp member).2 ▸ ih held one)
  | case11 names fields ih =>
    intro value made
    -- Each field contributes one default, in declaration order.
    cases parts : Desc.defaultFields fields with
    | error fault => simp [Desc.default, parts, Bind.bind, Except.bind] at made
    | ok values =>
      simp [Desc.default, parts, Bind.bind, Except.bind, pure, Except.pure] at made
      subst value
      exact .container (ih values parts).1 (ih values parts).2
  | case12 active names fields ih =>
    intro value made
    -- Gaps carry no values, so admissibility uses the same field pairing as a container.
    cases parts : Desc.defaultFields fields with
    | error fault => simp [Desc.default, parts, Bind.bind, Except.bind] at made
    | ok values =>
      simp [Desc.default, parts, Bind.bind, Except.bind, pure, Except.pure] at made
      subst value
      exact .progressiveContainer (ih values parts).1 (ih values parts).2
  -- A union has no default, so successful construction is impossible in this case.
  | case13 selectors options =>
    intro value made
    cases made
  -- An empty field suffix contributes zero values and no unmatched fields.
  | case14 =>
    rename_i values made
    cases made
    simp
  | case15 field rest ihHead ihTail =>
    rename_i values made
    -- Successful collection requires both the first default and all later defaults.
    cases head : field.default with
    | error fault => simp [Desc.defaultFields, head, Bind.bind, Except.bind] at made
    | ok value =>
      cases tail : Desc.defaultFields rest with
      | error fault => simp [Desc.defaultFields, head, tail, Bind.bind, Except.bind] at made
      | ok others =>
        simp [Desc.defaultFields, head, tail, Bind.bind, Except.bind, pure, Except.pure] at made
        subst values
        -- The suffix preserves its length and each field-value pairing.
        obtain ⟨sized, each⟩ := ihTail others tail
        refine ⟨by simp [sized], ?_⟩
        intro pair member
        -- Every pair belongs either to the first field or to the already-admissible suffix.
        rcases List.mem_cons.mp member with first | later
        · exact first ▸ ihHead value head
        · exact each pair later

end Ssz
