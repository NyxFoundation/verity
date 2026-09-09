import Ssz.Hash.Sha256

/-! The published SHA-256 tables, derived from prime roots rather than trusted as literals. -/

namespace Ssz.Sha256

/--
Whether a number below 324 is prime.

Trial division stops at 17, and every composite below 18 squared has a divisor that small.
The bound is what keeps the check exact while leaving it cheap enough to evaluate.
-/
def smallPrime (candidate : Nat) : Bool :=
  2 ≤ candidate && (List.range 18).all fun divisor =>
    divisor < 2 || divisor * divisor > candidate || candidate % divisor != 0

/-- The primes below a bound, in order, for bounds no larger than 324. -/
def smallPrimes (bound : Nat) : List Nat := (List.range bound).filter smallPrime

/-- Largest number below 18 whose square does not exceed the given one. -/
def wholeSquareRoot (value : Nat) : Nat :=
  (List.range 18).foldl (fun best candidate =>
    if candidate * candidate ≤ value then candidate else best) 0

/-- Largest number below 8 whose cube does not exceed the given one. -/
def wholeCubeRoot (value : Nat) : Nat :=
  (List.range 8).foldl (fun best candidate =>
    if candidate ^ 3 ≤ value then candidate else best) 0

/--
Whether a word holds the first 32 bits after the point in the square root of a prime.

    root = floor(sqrt(prime)) * 2^32 + word

The word is those bits exactly when squaring that root brackets the prime scaled by 2^64.
Squaring is what avoids ever taking a root, so the check is plain integer arithmetic.
-/
def squareRootFraction (prime word : Nat) : Bool :=
  let root := wholeSquareRoot prime * 2 ^ 32 + word
  root ^ 2 ≤ prime * 2 ^ 64 && prime * 2 ^ 64 < (root + 1) ^ 2

/--
Whether a word holds the first 32 bits after the point in the cube root of a prime.

    root = floor(cbrt(prime)) * 2^32 + word

The word is those bits exactly when cubing that root brackets the prime scaled by 2^96.
-/
def cubeRootFraction (prime word : Nat) : Bool :=
  let root := wholeCubeRoot prime * 2 ^ 32 + word
  root ^ 3 ≤ prime * 2 ^ 96 && prime * 2 ^ 96 < (root + 1) ^ 3

/-- The eight chaining words, as plain numbers. -/
def initialWords : List Nat := initialState.toList.map UInt32.toNat

/-- The sixty-four round constants, as plain numbers. -/
def roundWords : List Nat := roundConstants.toList.map UInt32.toNat

/--
Each chaining word is the square-root fraction of its prime, and there are eight of each.

FIPS 180-4 section 5.3.3 fixes them this way, so a typo in the table cannot survive this.
-/
theorem initialState_from_primes :
    (smallPrimes 20).length = 8 ∧ initialWords.length = 8 ∧
      ((smallPrimes 20).zip initialWords).all
        (fun pair => squareRootFraction pair.1 pair.2) = true := by
  decide

/--
Each round constant is the cube-root fraction of its prime, and there are sixty-four of each.

FIPS 180-4 section 4.2.2 fixes them this way, so a typo in the table cannot survive this.
-/
theorem roundConstants_from_primes :
    (smallPrimes 312).length = 64 ∧ roundWords.length = 64 ∧
      ((smallPrimes 312).zip roundWords).all
        (fun pair => cubeRootFraction pair.1 pair.2) = true := by
  decide

end Ssz.Sha256
