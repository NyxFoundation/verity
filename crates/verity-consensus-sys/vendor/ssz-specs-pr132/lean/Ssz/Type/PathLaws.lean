import Ssz.Type.Paths

/-! Type paths compose relative generalized indices and stop at mixing words. -/

namespace Ssz

/-- An empty path selects the current root. -/
@[simp] theorem getGeneralizedIndex_nil (shape : Desc) : getGeneralizedIndex shape [] = .ok 1 := rfl

/-- A nonterminal step resolves the remaining path inside its child before splicing the indices. -/
theorem getGeneralizedIndex_cons {shape child : Desc} {step : PathStep} {index : Nat}
    (resolved : shape.resolveStep step = .ok (index, some child)) (rest : List PathStep) :
    getGeneralizedIndex shape (step :: rest) =
      (getGeneralizedIndex child rest >>= gindexConcat index) := by
  -- Resolving a child leaves the remaining relative path to be joined beneath that child.
  simp only [getGeneralizedIndex, resolved, Bind.bind, Except.bind]

/-- A mixing-word step terminates at its relative node. -/
theorem getGeneralizedIndex_terminal {shape : Desc} {step : PathStep} {index : Nat}
    (resolved : shape.resolveStep step = .ok (index, none)) :
    getGeneralizedIndex shape [step] = .ok index := by
  -- A terminal step succeeds only because there is no suffix left to traverse.
  simp [getGeneralizedIndex, resolved, Bind.bind, Except.bind, pure, Except.pure]

/-- A path cannot continue inside a mixing word. -/
theorem getGeneralizedIndex_after_terminal {shape : Desc} {step next : PathStep} {index : Nat}
    (resolved : shape.resolveStep step = .ok (index, none)) (rest : List PathStep) :
    getGeneralizedIndex shape (step :: next :: rest) = .error .noPartsMixin := by
  -- A length, selector, or active-fields word has no addressable child type.
  simp [getGeneralizedIndex, resolved, Bind.bind, Except.bind, throw, throwThe, MonadExceptOf.throw]

/-- Joining two valid node indices removes only the inner leading bit. -/
theorem gindexConcat_eq {outer inner : Nat} (outerNamed : 1 ≤ outer) (innerNamed : 1 ≤ inner) :
    gindexConcat outer inner = .ok (outer * 2 ^ Nat.log2 inner + (inner - 2 ^ Nat.log2 inner)) := by
  -- A generalized index consists of a leading root bit followed by its left and right turns.
  simp [gindexConcat, gindexDepth, Nat.not_lt.mpr outerNamed, Nat.not_lt.mpr innerNamed,
    Bind.bind, Except.bind, pure, Except.pure]

/-- Appending an empty subtree path preserves the outer node. -/
theorem gindexConcat_root_right {outer : Nat} (named : 1 ≤ outer) :
    gindexConcat outer 1 = .ok outer := by
  -- The index one has no path turns to append.
  simp [gindexConcat_eq named (Nat.le_refl 1), show Nat.log2 1 = 0 from rfl]

/-- Splicing at the root preserves the inner node. -/
theorem gindexConcat_root_left {inner : Nat} (named : 1 ≤ inner) :
    gindexConcat 1 inner = .ok inner := by
  -- The inner leading bit replaces the outer root bit without changing any path turn.
  rw [gindexConcat_eq (Nat.le_refl 1) named]
  -- The leading power of two is no larger than a valid generalized index.
  have bound := (gindexDepth_bounds named).1
  simp [Nat.add_sub_cancel' bound]

/-- Successful splicing always names a real node. -/
theorem gindexConcat_positive {outer inner result : Nat}
    (joined : gindexConcat outer inner = .ok result) : 1 ≤ result := by
  -- Neither operand may be zero, because zero has no root bit to preserve.
  by_cases outerNamed : 1 ≤ outer
  · by_cases innerNamed : 1 ≤ inner
    · rw [gindexConcat_eq outerNamed innerNamed] at joined
      cases joined
      -- The retained leading bit contributes a strictly positive value to the joined index.
      have power := Nat.two_pow_pos (Nat.log2 inner)
      have grows : 2 ^ Nat.log2 inner ≤ outer * 2 ^ Nat.log2 inner := by
        simpa using Nat.mul_le_mul_right (2 ^ Nat.log2 inner) outerNamed
      omega
    · simp [gindexConcat, gindexDepth, show inner < 1 by omega,
        show ¬outer < 1 by omega, Bind.bind, Except.bind] at joined
  · simp [gindexConcat, gindexDepth, show outer < 1 by omega, Bind.bind, Except.bind] at joined

/-- A successful splice has two valid operands and the prescribed leading-bit arithmetic. -/
theorem gindexConcat_facts {outer inner result : Nat}
    (joined : gindexConcat outer inner = .ok result) :
    1 ≤ outer ∧ 1 ≤ inner ∧ result = outer * 2 ^ Nat.log2 inner + (inner - 2 ^ Nat.log2 inner) := by
  -- Neither operand may be zero, because zero has no root bit to preserve.
  by_cases outerNamed : 1 ≤ outer
  · by_cases innerNamed : 1 ≤ inner
    · rw [gindexConcat_eq outerNamed innerNamed] at joined
      exact ⟨outerNamed, innerNamed, (Except.ok.inj joined).symm⟩
    · simp [gindexConcat, gindexDepth, show inner < 1 by omega,
        show ¬outer < 1 by omega, Bind.bind, Except.bind] at joined
  · simp [gindexConcat, gindexDepth, show outer < 1 by omega, Bind.bind, Except.bind] at joined

/-- Splicing paths adds their depths, so each child turn is retained exactly once. -/
theorem gindexConcat_depth {outer inner result : Nat}
    (joined : gindexConcat outer inner = .ok result) :
    Nat.log2 result = Nat.log2 outer + Nat.log2 inner := by
  -- The two valid operands determine both the joined index and its leading-bit bounds.
  obtain ⟨outerNamed, innerNamed, value⟩ := gindexConcat_facts joined
  obtain ⟨outerLow, outerHigh⟩ := gindexDepth_bounds outerNamed
  obtain ⟨innerLow, innerHigh⟩ := gindexDepth_bounds innerNamed
  have positive := gindexConcat_positive joined
  -- Bounding the result between successive powers of two determines its exact depth.
  apply (Nat.log2_eq_iff (by omega)).mpr
  constructor
  · rw [Nat.pow_add, value]
    exact Nat.le_trans (Nat.mul_le_mul_right _ outerLow) (Nat.le_add_right _ _)
  -- The inner suffix fits strictly below its own next leading bit.
  · calc
      result < (outer + 1) * 2 ^ Nat.log2 inner := by
        rw [Nat.pow_succ] at innerHigh
        rw [value, Nat.add_mul, Nat.one_mul]
        omega
      _ ≤ 2 ^ (Nat.log2 outer + 1) * 2 ^ Nat.log2 inner :=
        Nat.mul_le_mul_right _ (by omega)
      _ = 2 ^ (Nat.log2 outer + Nat.log2 inner + 1) := by
        simp [Nat.pow_add, Nat.pow_succ, Nat.mul_right_comm]

/-- Grouping adjacent subtree paths does not change the node they select. -/
theorem gindexConcat_assoc {first second third left right : Nat}
    (front : gindexConcat first second = .ok left)
    (suffix : gindexConcat second third = .ok right) :
    gindexConcat left third = gindexConcat first right := by
  -- Both groupings retain the same low turns below the same outer leading bit.
  obtain ⟨firstNamed, secondNamed, leftValue⟩ := gindexConcat_facts front
  obtain ⟨_, thirdNamed, rightValue⟩ := gindexConcat_facts suffix
  rw [gindexConcat_eq (gindexConcat_positive front) thirdNamed,
    gindexConcat_eq firstNamed (gindexConcat_positive suffix), gindexConcat_depth suffix,
    Nat.pow_add, leftValue, rightValue]
  have below := Nat.mul_le_mul_right (2 ^ Nat.log2 third) (gindexDepth_bounds secondNamed).1
  simp only [Nat.add_mul, Nat.mul_assoc, Nat.sub_mul]
  congr 1
  omega

/-- A terminal path step always names the right-hand mixing word. -/
theorem resolveStep_terminal_index {shape : Desc} {step : PathStep} {index : Nat}
    (resolved : shape.resolveStep step = .ok (index, none)) : index = 3 := by
  -- Only recognized mixing words return no child type, and each occupies the root’s right child.
  cases shape <;> cases step <;>
    simp only [Desc.resolveStep, Bind.bind, Except.bind, Pure.pure, Except.pure] at resolved
  all_goals repeat first | split at resolved | cases resolved
  all_goals first | rfl | exact (congrArg Prod.fst (Except.ok.inj resolved)).symm

/-- Every successful type path names a real generalized index. -/
theorem getGeneralizedIndex_positive (shape : Desc) (path : List PathStep) (index : Nat)
    (resolved : getGeneralizedIndex shape path = .ok index) : 1 ≤ index := by
  -- An empty path names one.
  -- A nonempty path either terminates at three or joins valid child indices.
  cases path with
  | nil => cases resolved; decide
  | cons step rest =>
    -- A failed first step cannot produce a successful full path.
    cases first : shape.resolveStep step with
    | error fault => simp [getGeneralizedIndex, first, Bind.bind, Except.bind] at resolved
    | ok target =>
      obtain ⟨head, next⟩ := target
      cases next with
      -- A terminal mixing word has the fixed generalized index three.
      | none =>
        have headThree := resolveStep_terminal_index first
        cases rest <;> simp [getGeneralizedIndex, first, Bind.bind, Except.bind,
          pure, Except.pure, throw, throwThe, MonadExceptOf.throw] at resolved
        omega
      -- A successful nested path finishes with a valid generalized-index splice.
      | some child =>
        rw [getGeneralizedIndex_cons first] at resolved
        cases suffix : getGeneralizedIndex child rest with
        | error fault => simp [suffix, Bind.bind, Except.bind] at resolved
        | ok tail =>
          simp only [suffix, Bind.bind, Except.bind] at resolved
          exact gindexConcat_positive resolved

/-- A resolved child followed by any resolved suffix selects their spliced generalized index. -/
theorem getGeneralizedIndex_step {shape child : Desc} {step : PathStep} {head tail whole : Nat}
    {rest : List PathStep} (resolved : shape.resolveStep step = .ok (head, some child))
    (suffix : getGeneralizedIndex child rest = .ok tail)
    (joined : gindexConcat head tail = .ok whole) :
    getGeneralizedIndex shape (step :: rest) = .ok whole := by
  -- The first step and suffix have already succeeded, leaving only their prescribed splice.
  rw [getGeneralizedIndex_cons resolved, suffix]
  exact joined

/-- A path prefix ending at a child type, together with its composed node index. -/
inductive PathThrough : Desc → List PathStep → Desc → Nat → Prop where
  /-- An empty prefix leaves the type and its root unchanged. -/
  | nil (shape : Desc) : PathThrough shape [] shape 1
  /-- A child step composes its relative index with the remaining prefix. -/
  | cons {shape child target : Desc} {step : PathStep} {rest : List PathStep}
      {head tail whole : Nat}
      (resolved : shape.resolveStep step = .ok (head, some child))
      (through : PathThrough child rest target tail)
      (joined : gindexConcat head tail = .ok whole) :
      PathThrough shape (step :: rest) target whole

/-- A prefix's composed index is the result of the executable path resolver. -/
theorem PathThrough.resolves {shape child : Desc} {path : List PathStep} {index : Nat}
    (through : PathThrough shape path child index) : getGeneralizedIndex shape path = .ok index := by
  -- Each declared prefix step supplies exactly the evidence the executable resolver needs.
  induction through with
  | nil => rfl
  | cons resolved _ joined ih => exact getGeneralizedIndex_step resolved ih joined

/-- Resolving a suffix inside the reached type gives the same result as resolving the full path. -/
theorem PathThrough.append {shape child : Desc} {path : List PathStep} {index : Nat}
    (through : PathThrough shape path child index) (rest : List PathStep) (inner whole : Nat)
    (suffix : getGeneralizedIndex child rest = .ok inner)
    (joined : gindexConcat index inner = .ok whole) :
    getGeneralizedIndex shape (path ++ rest) = .ok whole := by
  -- Associativity makes the grouping of subtree paths irrelevant.
  induction through generalizing whole with
  | nil =>
    rw [gindexConcat_root_left (getGeneralizedIndex_positive _ _ _ suffix)] at joined
    cases joined
    exact suffix
  | @cons shape child target step path head tail index resolved through front ih =>
    -- First join the remaining prefix to the suffix, then reattach the outermost step.
    let combined := tail * 2 ^ Nat.log2 inner + (inner - 2 ^ Nat.log2 inner)
    have tailJoined : gindexConcat tail inner = .ok combined :=
      gindexConcat_eq (getGeneralizedIndex_positive _ _ _ through.resolves)
        (getGeneralizedIndex_positive _ _ _ suffix)
    have result := ih combined suffix tailJoined
    apply getGeneralizedIndex_step resolved result
    rw [← gindexConcat_assoc front tailJoined]
    exact joined

end Ssz
