import Ssz.Type.Default
import Ssz.Type.Valid

/-! Structural equality and recognition of default values. -/

namespace Ssz

/-- Comparing nested values succeeds exactly when the values are equal. -/
theorem Value.beq_eq (left right : Value) : Value.beq left right = true ↔ left = right := by
  -- Nested induction proves the value comparison and its list traversal together.
  induction left using Value.rec
    (motive_2 := fun left => ∀ right, Value.beqList left right = true ↔ left = right)
    generalizing right with
  | bool | uint | bytes | bits => cases right <;> simp [Value.beq]
  | seq elements ih => cases right <;> simp [Value.beq, ih]
  | union selector data ih => cases right <;> simp [Value.beq, ih]
  | nil => rename_i right; cases right <;> simp [Value.beqList]
  | cons head tail ihHead ihTail =>
      rename_i right
      cases right <;> simp [Value.beqList, ihHead, ihTail]

-- Structural comparison supplies both reflexivity and sound equality for boolean reasoning.
instance : LawfulBEq Value where
  rfl := (Value.beq_eq _ _).mpr rfl
  eq_of_beq := (Value.beq_eq _ _).mp

/-- Comparing nested declarations succeeds exactly when the declarations are equal. -/
theorem Desc.beq_eq (left right : Desc) : Desc.beq left right = true ↔ left = right := by
  -- Nested induction covers field lists and union options along with their enclosing types.
  induction left using Desc.rec
    (motive_2 := fun left => ∀ right, Desc.beqList left right = true ↔ left = right)
    generalizing right with
  | bool | uint | byteVector | byteList | bitVector | bitList | progressiveBitList =>
      cases right <;> simp [Desc.beq]
  | vector element length ih | list element length ih =>
      cases right <;> simp [Desc.beq, ih]
  | progressiveList element ih => cases right <;> simp [Desc.beq, ih]
  | container names fields ih => cases right <;> simp [Desc.beq, ih]
  | progressiveContainer active names fields ih =>
      cases right <;> simp [Desc.beq, ih, and_assoc]
  | compatibleUnion selectors options ih => cases right <;> simp [Desc.beq, ih]
  | nil => rename_i right; cases right <;> simp [Desc.beqList]
  | cons head tail ihHead ihTail =>
      rename_i right
      cases right <;> simp [Desc.beqList, ihHead, ihTail]

-- Declaration comparison can be used as ordinary equality throughout the specification.
instance : LawfulBEq Desc where
  rfl := (Desc.beq_eq _ _).mpr rfl
  eq_of_beq := (Desc.beq_eq _ _).mp

/-- A value is zeroed exactly when constructing the type's default returns that value. -/
@[simp] theorem Desc.isZero_eq_true (shape : Desc) (value : Value) :
    shape.isZero value = .ok true ↔ shape.default = .ok value := by
  -- A successful default is compared structurally, while an error passes through unchanged.
  unfold Desc.isZero
  cases shape.default <;>
    simp [Bind.bind, Except.bind, pure, Except.pure]

/-- The default recognizer returns false exactly for a different value of a defaultable type. -/
theorem Desc.isZero_eq_false (shape : Desc) (value : Value) :
    shape.isZero value = .ok false ↔
      ∃ zero, shape.default = .ok zero ∧ zero ≠ value := by
  -- Failure to construct a default is an error, never a negative comparison.
  unfold Desc.isZero
  cases shape.default <;> simp [Bind.bind, Except.bind, pure, Except.pure]

/-- Default construction and default recognition report the same failure. -/
@[simp] theorem Desc.isZero_eq_error (shape : Desc) (value : Value) (fault : Err) :
    shape.isZero value = .error fault ↔ shape.default = .error fault := by
  -- Comparison cannot produce a new failure after a default has been constructed.
  unfold Desc.isZero
  cases shape.default <;> simp [Bind.bind, Except.bind, pure, Except.pure]

/-- Every type has at least one level, including a type with no nested components. -/
theorem Desc.nesting_pos (shape : Desc) : 0 < shape.nesting := by
  -- Every constructor contributes one level above any nested components.
  cases shape <;> simp [Desc.nesting]

/-- One remaining comparison step suffices to recognize the same declaration. -/
@[simp] theorem compatibleAt_self (budget : Nat) (shape : Desc) :
    compatibleAt (budget + 1) shape shape = true := by
  -- Structural equality takes the first branch before any nested comparison is needed.
  rw [compatibleAt.eq_def]
  simp

/-- Every declaration is compatible with itself. -/
@[simp] theorem isCompatible_self (shape : Desc) : isCompatible shape shape = true := by
  -- The public comparison allocates a positive budget even for a type with no children.
  have positive := shape.nesting_pos
  unfold isCompatible
  cases total : shape.nesting + shape.nesting with
  | zero => omega
  | succ budget => exact compatibleAt_self budget shape

end Ssz
