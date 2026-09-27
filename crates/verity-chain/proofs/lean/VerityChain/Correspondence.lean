import LeanSpec.Forks.Lstar.Slot
import VerityChain.Funs

open Aeneas Aeneas.Std RustM

namespace verity_chain.correspondence

/-- Convert the Aeneas model of Rust `u64` to formal-leanSpec's `UInt64`. -/
def toLeanSlot (slot : Std.U64) : LeanSpec.Slot :=
  UInt64.ofNat slot.val

@[simp]
theorem toLeanSlot_toNat (slot : Std.U64) : (toLeanSlot slot).toNat = slot.val := by
  simp [toLeanSlot]

@[simp]
theorem toLeanSlot_lt_iff (left right : Std.U64) :
    toLeanSlot left < toLeanSlot right ↔ left.val < right.val := by
  rw [UInt64.lt_iff_toNat_lt]
  simp

def u64Difference (left right : Std.U64) : Std.U64 :=
  ⟨BitVec.ofNat 64 (left.val - right.val)⟩

@[simp]
theorem u64Difference_val (left right : Std.U64) :
    (u64Difference left right).val = left.val - right.val := by
  have hbound : left.val - right.val < 2 ^ 64 :=
    (Nat.sub_le left.val right.val).trans_lt left.hBounds
  change (BitVec.ofNat 64 (left.val - right.val)).toNat = left.val - right.val
  rw [BitVec.toNat_ofNat, Nat.mod_eq_of_lt hbound]

theorem u64_sub_eq (left right : Std.U64) (h : right.val ≤ left.val) :
    left - right = ok (u64Difference left right) := by
  change UScalar.sub left right = _
  simp [UScalar.sub, u64Difference, Nat.not_lt.mpr h]

def u128RemainderTwo (value : Std.U128) : Std.U128 :=
  ⟨BitVec.umod value.bv (2#u128).bv⟩

@[simp]
theorem u128RemainderTwo_val (value : Std.U128) :
    (u128RemainderTwo value).val = value.val % 2 := by
  change (value.bv % (2#u128).bv).toNat = value.val % 2
  rw [BitVec.toNat_umod]
  simp

theorem u128_rem_two_eq (value : Std.U128) :
    value % 2#u128 = ok (u128RemainderTwo value) := by
  change UScalar.rem value 2#u128 = _
  simp [UScalar.rem, u128RemainderTwo]

@[simp]
theorem u128RemainderTwo_eq_one_iff (value : Std.U128) :
    u128RemainderTwo value = 1#u128 ↔ value.val % 2 = 1 := by
  constructor
  · intro heq
    have hval := congrArg UScalar.val heq
    simpa using hval
  · intro hval
    apply Std.U128.bv_eq_imp_eq
    apply BitVec.eq_of_toNat_eq
    simpa using hval

theorem u128_add_eq (left right : Std.U128)
    (h : left.val + right.val < 2 ^ UScalarTy.U128.numBits) :
    left + right = ok (UScalar.ofNatCore (left.val + right.val) h) := by
  change UScalar.add left right = _
  have h' : left.val + right.val < 340282366920938463463374607431768211456 := by
    norm_num [UScalarTy.U128_numBits_eq] at h ⊢
    exact h
  simp [UScalar.add, UScalar.tryMk, UScalar.tryMkOpt, UScalar.check_bounds, h']

theorem u128_mul_eq (left right : Std.U128)
    (h : left.val * right.val < 2 ^ UScalarTy.U128.numBits) :
    left * right = ok (UScalar.ofNatCore (left.val * right.val) h) := by
  change UScalar.mul left right = _
  have h' : left.val * right.val < 340282366920938463463374607431768211456 := by
    norm_num [UScalarTy.U128_numBits_eq] at h ⊢
    exact h
  simp [UScalar.mul, UScalar.tryMk, UScalar.tryMkOpt, UScalar.check_bounds, h']

/-- The extracted perfect-square helper computes with formal-leanSpec's square root. -/
theorem isPerfectSquare_eq (value : Std.U128) :
    justification.is_perfect_square value =
      ok (decide (LeanSpec.Slot.isqrt value.val * LeanSpec.Slot.isqrt value.val = value.val)) := by
  simp [justification.is_perfect_square, core.num.U128.isqrt, models.u128Isqrt]
  rw [u128_mul_eq]
  · simp
    constructor
    · intro heq
      have hval := congrArg UScalar.val heq
      simpa using hval
    · intro hval
      apply Std.U128.bv_eq_imp_eq
      apply BitVec.eq_of_toNat_eq
      simpa using hval
  · exact (LeanSpec.Slot.isqrt_le value.val).trans_lt value.hBounds

private theorem pronicBranch_eq (delta : Std.U64) :
    (do
      let i2 ← 4#u128 * core.convert.num.FromU128U64.from delta
      let discriminant ← i2 + 1#u128
      let isSquare ← justification.is_perfect_square discriminant
      if isSquare = true then do
        let root ← core.num.U128.isqrt discriminant
        let parity ← root % 2#u128
        ok (decide (parity = 1#u128))
      else ok false) =
    ok ((LeanSpec.Slot.isqrt (4 * delta.val + 1) *
        LeanSpec.Slot.isqrt (4 * delta.val + 1) == 4 * delta.val + 1) &&
      (LeanSpec.Slot.isqrt (4 * delta.val + 1) % 2 == 1)) := by
  rw [u128_mul_eq]
  · rw [bind_tc_ok, u128_add_eq]
    · rw [bind_tc_ok, isPerfectSquare_eq, bind_tc_ok]
      simp
      by_cases hSquare : LeanSpec.Slot.isqrt (4 * delta.val + 1) *
          LeanSpec.Slot.isqrt (4 * delta.val + 1) = 4 * delta.val + 1
      · rw [if_pos hSquare]
        simp [core.num.U128.isqrt, models.u128Isqrt, hSquare]
        rw [u128_rem_two_eq, bind_tc_ok]
        simp
        exact (Bool.beq_eq_decide_eq
          (LeanSpec.Slot.isqrt (4 * delta.val + 1) % 2) (1 : Nat)).symm
      · rw [if_neg hSquare]
        simp [hSquare]
    · simp
      have hdelta := delta.hBounds
      norm_num [UScalarTy.U64_numBits_eq, UScalarTy.U128_numBits_eq] at hdelta ⊢
      omega
  · simp
    have hdelta := delta.hBounds
    norm_num [UScalarTy.U64_numBits_eq, UScalarTy.U128_numBits_eq] at hdelta ⊢
    omega

/-- The extracted Rust predicate agrees with formal-leanSpec after finalization. -/
theorem isJustifiableAfter_eq
    (slot finalized : Std.U64) (h : finalized.val ≤ slot.val) :
    justification.is_justifiable_after slot finalized =
      ok (LeanSpec.Slot.isJustifiableAfter (toLeanSlot finalized) (toLeanSlot slot)) := by
  have hnot : ¬ slot.val < finalized.val := by omega
  have hnotScalar : ¬slot < finalized := by
    simpa [UScalar.lt_equiv] using hnot
  have hleanNot : ¬toLeanSlot slot < toLeanSlot finalized := by
    simpa using hnot
  rw [show LeanSpec.Slot.isJustifiableAfter (toLeanSlot finalized) (toLeanSlot slot) =
      LeanSpec.Slot.justifiableDelta (slot.val - finalized.val) by
    unfold LeanSpec.Slot.isJustifiableAfter
    rw [if_neg hleanNot]
    simp]
  unfold justification.is_justifiable_after
  rw [if_neg hnotScalar, u64_sub_eq slot finalized h, bind_tc_ok]
  have hWindow :
      u64Difference slot finalized ≤ justification.IMMEDIATE_JUSTIFICATION_WINDOW ↔
        slot.val - finalized.val ≤ 5 := by
    rw [UScalar.le_equiv]
    simp [justification.IMMEDIATE_JUSTIFICATION_WINDOW]
  by_cases hImmediate : slot.val - finalized.val ≤ 5
  · rw [if_pos (hWindow.mpr hImmediate)]
    simp [LeanSpec.Slot.justifiableDelta, LeanSpec.Slot.immediateJustificationWindow,
      hImmediate]
  · rw [if_neg (fun hWindowValue => hImmediate (hWindow.mp hWindowValue))]
    simp only [lift]
    rw [bind_tc_ok, isPerfectSquare_eq, bind_tc_ok]
    simp [LeanSpec.Slot.justifiableDelta, LeanSpec.Slot.immediateJustificationWindow,
      hImmediate]
    by_cases hSquare :
        LeanSpec.Slot.isqrt (slot.val - finalized.val) *
          LeanSpec.Slot.isqrt (slot.val - finalized.val) = slot.val - finalized.val
    · rw [if_pos hSquare]
      simp [hSquare]
    · rw [if_neg hSquare, pronicBranch_eq]
      rw [show (LeanSpec.Slot.isqrt (slot.val - finalized.val) *
          LeanSpec.Slot.isqrt (slot.val - finalized.val) ==
            slot.val - finalized.val) = false by
        exact beq_eq_false_iff_ne.mpr hSquare]
      simp

/-- The extracted Rust predicate rejects slots before the finalized boundary. -/
theorem isJustifiableAfter_before_finalized
    (slot finalized : Std.U64) (h : slot.val < finalized.val) :
    justification.is_justifiable_after slot finalized = ok false := by
  unfold justification.is_justifiable_after
  rw [if_pos (by simpa [UScalar.lt_equiv] using h)]

end verity_chain.correspondence
