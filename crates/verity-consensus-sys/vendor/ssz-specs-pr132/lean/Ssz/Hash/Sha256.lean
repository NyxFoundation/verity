/-!
SHA-256 for byte-aligned messages, following [FIPS 180-4](https://csrc.nist.gov/files/pubs/fips/180-4/final/docs/fips180-4.pdf).
-/

namespace Ssz.Sha256

/--
The sixty-four round constants.

Each is the first thirty-two bits of a cube root's fractional part.
The roots taken are those of the first sixty-four primes.
-/
def roundConstants : Vector UInt32 64 :=
  -- The published cube-root fractions provide one fixed additive constant per round.
  #v[
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
  0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
  0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
  0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
  0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2]

/--
The eight starting words of the state.

Each is the first thirty-two bits of a square root's fractional part.
The roots taken are those of the first eight primes.
-/
def initialState : Vector UInt32 8 :=
  -- The published square-root fractions seed the eight chaining words before the first block.
  #v[
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
  0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]

/-- A word rotated right, with the bits leaving the bottom re-entering at the top. -/
def rotr (x : UInt32) (n : UInt32) : UInt32 :=
  -- The disjoint shifted pieces put the low bits back into the high positions.
  (x >>> n) ||| (x <<< (32 - n))

/-- The message-schedule mixing applied to the word fifteen places back. -/
def sigma0 (x : UInt32) : UInt32 :=
  -- Two rotations spread bits around the word, while the logical shift introduces zeros.
  rotr x 7 ^^^ rotr x 18 ^^^ (x >>> 3)

/-- The message-schedule mixing applied to the word two places back. -/
def sigma1 (x : UInt32) : UInt32 :=
  -- The later schedule contribution uses a different rotation pair and zero-filling shift.
  rotr x 17 ^^^ rotr x 19 ^^^ (x >>> 10)

/-- The round mixing applied to the first working word. -/
def bigSigma0 (x : UInt32) : UInt32 :=
  -- Three rotations mix the first working word without discarding any input bits.
  rotr x 2 ^^^ rotr x 13 ^^^ rotr x 22

/-- The round mixing applied to the fifth working word. -/
def bigSigma1 (x : UInt32) : UInt32 :=
  -- The fifth working word uses its own three rotation distances.
  rotr x 6 ^^^ rotr x 11 ^^^ rotr x 25

/-- Bit by bit, the second word where the first is set and the third where it is not. -/
def choose (x y z : UInt32) : UInt32 :=
  -- Each selector bit chooses exactly one of the two candidate bits.
  (x &&& y) ^^^ ((~~~x) &&& z)

/-- Bit by bit, whichever value at least two of the three words agree on. -/
def majority (x y z : UInt32) : UInt32 :=
  -- The pairwise intersections leave a bit set exactly when at least two inputs set it.
  (x &&& y) ^^^ (x &&& z) ^^^ (y &&& z)

/-- Reading four bytes of a block as one big-endian word. -/
def wordAt (block : ByteArray) (start : Nat) : UInt32 :=
  -- The first byte read is the most significant, so later bytes shift the total up.
  (block.get! start).toUInt32 <<< 24 |||
  (block.get! (start + 1)).toUInt32 <<< 16 |||
  (block.get! (start + 2)).toUInt32 <<< 8 |||
  (block.get! (start + 3)).toUInt32

/-- The first words of the schedule, with every earlier reference proved in bounds. -/
def schedulePrefix (block : ByteArray) (start : Nat) : (count : Nat) → Vector UInt32 count
  | 0 => .emptyWithCapacity 64
  | count + 1 =>
    let previous := schedulePrefix block start count
    -- FIPS 180-4, section 6.2.2: sixteen input words precede the four-term recurrence.
    let next := if first : count < 16 then wordAt block (start + 4 * count)
      else previous[count - 16]'(by omega) + sigma0 (previous[count - 15]'(by omega)) +
        previous[count - 7]'(by omega) + sigma1 (previous[count - 2]'(by omega))
    -- Appending leaves every earlier schedule word unchanged for later recurrence references.
    previous.push next

/-- The sixty-four schedule words consumed by one block's rounds. -/
def schedule (block : ByteArray) (start : Nat) : Vector UInt32 64 :=
  -- One block requires sixteen loaded words and forty-eight recurrence words.
  schedulePrefix block start 64

/-- One SHA-256 round, preserving the eight working words. -/
def round (state : Vector UInt32 8) (constant word : UInt32) : Vector UInt32 8 :=
  -- FIPS 180-4, section 6.2.2, names the eight working words in order.
  let a := state[0]
  let b := state[1]
  let c := state[2]
  let d := state[3]
  let e := state[4]
  let f := state[5]
  let g := state[6]
  let h := state[7]
  -- Two temporary sums feed the first and fifth words.
  let t1 := h + bigSigma1 e + choose e f g + constant + word
  let t2 := bigSigma0 a + majority a b c
  -- The remaining six words shift by one position.
  #v[t1 + t2, a, b, c, d + t1, e, f, g]

/-- The state after absorbing one sixty-four-byte block. -/
def compress (state : Vector UInt32 8) (block : ByteArray) (start : Nat) : Vector UInt32 8 :=
  -- Parse and extend one block before the sixty-four rounds consume its words.
  let words := schedule block start
  -- Each round consumes exactly one schedule word and the constant at the same position.
  let working := (List.finRange 64).foldl
    (fun current i => round current roundConstants[i] words[i]) state
  -- The block's effect is added to the previous state, word by word modulo 2^32.
  state.zipWith (· + ·) working

/-- The number of whole zero bytes between the closing bit and the length. -/
def paddingZeros (size : Nat) : Nat :=
  -- Reserve one delimiter byte and eight length bytes before rounding up to a 64-byte block.
  (64 - (size + 9) % 64) % 64

/-- The message length in bits, encoded as eight big-endian bytes. -/
def lengthBytes (size : Nat) : ByteArray :=
  -- FIPS 180-4 admits fewer than 2^64 bits, so an admitted length fits this word.
  let bits : UInt64 := (UInt64.ofNat size) * 8
  -- The byte shifts are 56, 48, 40, 32, 24, 16, 8, and 0, selecting the most significant byte first.
  ⟨Array.ofFn fun i : Fin 8 => (bits >>> (56 - 8 * UInt64.ofNat i.val)).toUInt8⟩

/--
The message followed by a one bit, zeros, and its own length in bits.

FIPS 180-4, section 5.1.1, places the length in the last eight bytes of a full block.
The standard's message domain is fewer than 2^61 bytes.
-/
def pad (message : ByteArray) : ByteArray :=
  -- One byte carries the closing bit, including its seven low zero bits.
  message.push 0x80 ++ ⟨Array.replicate (paddingZeros message.size) 0⟩ ++
    lengthBytes message.size

/-- The final state written as thirty-two bytes, most significant byte first in each word. -/
def digest (state : Vector UInt32 8) : ByteArray :=
  -- Each group of four digest bytes reads one state word.
  ⟨Array.ofFn fun i : Fin 32 =>
    (state[i.val / 4]'(by omega) >>> (24 - 8 * UInt32.ofNat (i.val % 4))).toUInt8⟩

/-- The thirty-two byte digest of a message. -/
def hash (message : ByteArray) : ByteArray :=
  -- The delimiter and length suffix make every block complete before compression starts.
  let padded := pad message
  -- Blocks are absorbed in order, preserving an eight-word state throughout.
  let state := (List.range (padded.size / 64)).foldl
    (fun current block => compress current padded (64 * block)) initialState
  digest state

end Ssz.Sha256
