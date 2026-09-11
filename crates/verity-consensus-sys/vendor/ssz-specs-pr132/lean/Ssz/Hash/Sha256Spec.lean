import Ssz.Hash.Sha256Laws

/-!
A modular-bitvector model of the SHA-256 schedule and compression rounds.
The equations follow [FIPS 180-4](https://csrc.nist.gov/files/pubs/fips/180-4/final/docs/fips180-4.pdf), sections 4.1.2 and 6.2.2.
-/

namespace Ssz.Sha256.Spec

/-- A mathematical word whose arithmetic is modulo 2^32. -/
abbrev Word := BitVec 32

/-- The eight working words named by the compression equations. -/
structure State where
  /-- The first working word. -/
  a : Word
  /-- The second working word. -/
  b : Word
  /-- The third working word. -/
  c : Word
  /-- The fourth working word. -/
  d : Word
  /-- The fifth working word. -/
  e : Word
  /-- The sixth working word. -/
  f : Word
  /-- The seventh working word. -/
  g : Word
  /-- The eighth working word. -/
  h : Word
  deriving DecidableEq

/-- Interpret the executable state as eight mathematical words. -/
def interpret (words : Vector UInt32 8) : State :=
  -- Preserve all eight positions while replacing machine words by their exact 32-bit patterns.
  ⟨words[0].toBitVec, words[1].toBitVec, words[2].toBitVec, words[3].toBitVec,
    words[4].toBitVec, words[5].toBitVec, words[6].toBitVec, words[7].toBitVec⟩

/-- Schedule mixing of the word fifteen positions back. -/
def small0 (word : Word) : Word :=
  -- The independent equation uses true rotations rather than a pair of machine shifts.
  word.rotateRight 7 ^^^ word.rotateRight 18 ^^^ (word >>> 3)

/-- Schedule mixing of the word two positions back. -/
def small1 (word : Word) : Word :=
  -- The two rotations and the zero-filling shift are the second schedule-mixing equation.
  word.rotateRight 17 ^^^ word.rotateRight 19 ^^^ (word >>> 10)

/-- Round mixing of the first working word. -/
def large0 (word : Word) : Word :=
  -- The three mathematical rotations follow the first working-word equation.
  word.rotateRight 2 ^^^ word.rotateRight 13 ^^^ word.rotateRight 22

/-- Round mixing of the fifth working word. -/
def large1 (word : Word) : Word :=
  -- The fifth working-word equation uses three different rotation distances.
  word.rotateRight 6 ^^^ word.rotateRight 11 ^^^ word.rotateRight 25

/-- One mathematical round, using modular sums and bitwise selection. -/
def step (state : State) (constant word : Word) : State :=
  -- FIPS 180-4 separates the choice contribution from the majority contribution.
  let chosen := (state.e &&& state.f) ^^^ ((~~~state.e) &&& state.g)
  let majority := (state.a &&& state.b) ^^^ (state.a &&& state.c) ^^^ (state.b &&& state.c)
  -- All five contributions are added modulo 2^32 before updating the first and fifth words.
  let first := state.h + large1 state.e + chosen + constant + word
  let second := large0 state.a + majority
  -- The six remaining words move one position while the two sums enter at the front.
  ⟨first + second, state.a, state.b, state.c, state.d + first, state.e, state.f, state.g⟩

/-- The declarative recurrence, independent of the executable growing schedule buffer. -/
def messageWord (input : Vector Word 16) (position : Nat) : Word :=
  -- The first sixteen positions read the block directly.
  -- Every later position depends only on older words.
  if early : position < 16 then input[position]
  else
    messageWord input (position - 16) + small0 (messageWord input (position - 15)) +
      messageWord input (position - 7) + small1 (messageWord input (position - 2))
termination_by position

/-- Four input bytes form one big-endian word, with the first byte most significant. -/
def inputWord (block : ByteArray) (start : Nat) : Word :=
  -- The four byte positions carry weights 2^24, 2^16, 2^8, and 1.
  (BitVec.ofNat 32 (block.get! start).toNat) <<< 24 |||
    (BitVec.ofNat 32 (block.get! (start + 1)).toNat) <<< 16 |||
    (BitVec.ofNat 32 (block.get! (start + 2)).toNat) <<< 8 |||
    BitVec.ofNat 32 (block.get! (start + 3)).toNat

/-- A block consists of sixteen consecutive big-endian words. -/
def inputWords (block : ByteArray) (start : Nat) : Vector Word 16 :=
  -- Consecutive words start four bytes apart, covering one complete 64-byte block.
  Vector.ofFn fun position => inputWord block (start + 4 * position.val)

/-- The chaining state accumulates the working state after all rounds. -/
def addState (initial working : State) : State :=
  -- Feed-forward addition preserves each working word position and wraps modulo 2^32.
  ⟨initial.a + working.a, initial.b + working.b, initial.c + working.c,
    initial.d + working.d, initial.e + working.e, initial.f + working.f,
    initial.g + working.g, initial.h + working.h⟩

/-- Successive mathematical states, beginning with the incoming chaining state. -/
-- The explicit constant table is shared with the executable, interpreted as modular words.
def rounds (input : Vector Word 16) (initial : State) : (count : Nat) → count ≤ 64 → State
  -- A round prefix grows by applying the next constant and scheduled word to the preceding state.
  | 0, _ => initial
  | count + 1, bound => step (rounds input initial count (by omega))
      (roundConstants[count]'(by omega)).toBitVec (messageWord input count)

/-- One block's sixty-four rounds followed by componentwise feed-forward addition. -/
def compression (input : Vector Word 16) (initial : State) : State :=
  -- Sixty-four rounds finish the working state before feed-forward adds the incoming state.
  addState initial (rounds input initial 64 (by omega))

/-- The standard's initial eight words, interpreted as modular bitvectors. -/
def initial : State :=
  -- These are the eight independently written initial words from FIPS 180-4, section 5.3.3.
  ⟨0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19⟩

/-- The least whole-byte padding that leaves eight bytes at the end of a block. -/
def zeroCount (size : Nat) : Nat :=
  -- The desired delimiter boundary is byte 55 modulo 64, before the eight-byte length suffix.
  (119 - size % 64) % 64

/-- Padding with the bit length expressed directly as eight base-256 digits. -/
def padded (message : ByteArray) : ByteArray :=
  -- The closing bit occupies its own byte because SSZ hashes byte-aligned messages.
  ⟨message.data ++ #[0x80] ++ Array.replicate (zeroCount message.size) 0 ++
    Array.ofFn fun i : Fin 8 =>
      UInt8.ofNat ((message.size * 8 / 2 ^ (56 - 8 * i.val)) % 256)⟩

/-- The state after the first consecutive blocks of a padded message. -/
def blocks (message : ByteArray) (initial : State) : Nat → State
  -- Each new block consumes the chaining state produced by all preceding blocks.
  | 0 => initial
  | count + 1 => compression (inputWords message (64 * count)) (blocks message initial count)

/-- The state words in the order in which they appear in the digest. -/
def State.words (state : State) : Vector Word 8 :=
  -- Digest ordering follows the eight working-word positions, with no permutation.
  #v[state.a, state.b, state.c, state.d, state.e, state.f, state.g, state.h]

/-- The mathematical digest consists of each state word's four big-endian base-256 digits. -/
def digest (state : State) : ByteArray :=
  -- Each word contributes four bytes, selecting its most significant base-256 digit first.
  ⟨Array.ofFn fun i : Fin 32 =>
    UInt8.ofNat ((state.words[i.val / 4]'(by omega)).toNat / 2 ^ (24 - 8 * (i.val % 4)) % 256)⟩

/-- SHA-256 as padding, successive modular-bitvector compressions, and base-256 output digits. -/
def hash (message : ByteArray) : ByteArray :=
  -- Padding determines a whole block count, and the final chaining state determines the digest.
  let input := padded message
  digest (blocks input initial (input.size / 64))

end Ssz.Sha256.Spec

namespace Ssz.Sha256

/-- The schedule's first mixing function agrees with mathematical rotation and logical shift. -/
theorem sigma0_refines (word : UInt32) : (sigma0 word).toBitVec = Spec.small0 word.toBitVec := by
  -- Interpreting shifts and bitwise joins recovers the same two mathematical rotations.
  simp [sigma0, rotr, Spec.small0, BitVec.rotateRight_def]

/-- The second schedule mixing function agrees with the mathematical bitvector equation. -/
theorem sigma1_refines (word : UInt32) : (sigma1 word).toBitVec = Spec.small1 word.toBitVec := by
  -- The machine operations preserve the second schedule equation bit for bit.
  simp [sigma1, rotr, Spec.small1, BitVec.rotateRight_def]

/-- The first round mixing function agrees with three mathematical rotations. -/
theorem bigSigma0_refines (word : UInt32) : (bigSigma0 word).toBitVec = Spec.large0 word.toBitVec := by
  -- Each joined pair of shifts is the corresponding mathematical rotation.
  simp [bigSigma0, rotr, Spec.large0, BitVec.rotateRight_def]

/-- The fifth-word mixing function agrees with three mathematical rotations. -/
theorem bigSigma1_refines (word : UInt32) : (bigSigma1 word).toBitVec = Spec.large1 word.toBitVec := by
  -- The three rotation distances are all below the 32-bit word width.
  simp [bigSigma1, rotr, Spec.large1, BitVec.rotateRight_def]

/-- One executable round implements the independent modular-bitvector state transition. -/
theorem round_refines (state : Vector UInt32 8) (constant word : UInt32) :
    Spec.interpret (round state constant word) =
      Spec.step (Spec.interpret state) constant.toBitVec word.toBitVec := by
  -- Word interpretation preserves modular addition, bitwise selection, and all eight output positions.
  simp [Spec.interpret, round, Spec.step, choose, majority, bigSigma0_refines, bigSigma1_refines]

/-- Executable byte loading agrees with the model's big-endian word formation. -/
theorem wordAt_refines (block : ByteArray) (start : Nat) :
    (wordAt block start).toBitVec = Spec.inputWord block start := by
  -- Widening a byte preserves its value before the four big-endian contributions are joined.
  have byte (value : UInt8) : value.toBitVec.setWidth 32 = BitVec.ofNat 32 value.toNat := by
    apply BitVec.eq_of_toNat_eq
    simp
  simp [wordAt, Spec.inputWord, byte]

/-- Every executable schedule word satisfies the independent recursive bitvector model. -/
theorem schedule_refines (block : ByteArray) (start : Nat) :
    ∀ position, (within : position < 64) →
      ((schedule block start)[position]).toBitVec =
        Spec.messageWord (Spec.inputWords block start) position := by
  -- Strong induction permits the four different backward references in the schedule recurrence.
  intro position
  induction position using Nat.strongRecOn with
  | ind position ih =>
    intro within
    by_cases early : position < 16
    -- The first sixteen words have no recursive dependency and come directly from the block.
    · rw [schedule_input block start position early, wordAt_refines, Spec.messageWord]
      simp [early, Spec.inputWords]
    -- Every backward reference is smaller than the current position, so the induction covers all four.
    · rw [schedule_recurrence block start position (by omega) within]
      simp only [UInt32.toBitVec_add, sigma0_refines, sigma1_refines]
      rw [ih (position - 16) (by omega) (by omega), ih (position - 15) (by omega) (by omega),
        ih (position - 7) (by omega) (by omega), ih (position - 2) (by omega) (by omega)]
      rw [Spec.messageWord.eq_def (Spec.inputWords block start) position, dif_neg early]

/-- The executable round loop follows the model after every prefix of the sixty-four rounds. -/
private theorem rounds_refines (block : ByteArray) (start : Nat) (state : Vector UInt32 8) :
    ∀ count, (bounded : count ≤ 64) →
      Spec.interpret (((List.finRange 64).take count).foldl
        (fun current i => round current roundConstants[i] (schedule block start)[i]) state) =
      Spec.rounds (Spec.inputWords block start) (Spec.interpret state) count bounded := by
  intro count
  induction count with
  | zero => intro _; rfl
  | succ count ih =>
    intro bounded
    -- Appending one index extends the executable fold by exactly one mathematical round.
    rw [List.take_succ_eq_append_getElem (by simp; omega), List.foldl_append]
    simp only [List.foldl_cons, List.foldl_nil, List.getElem_finRange, Fin.cast_mk]
    rw [round_refines, ih (by omega)]
    simp only [Spec.rounds, Fin.getElem_fin]
    rw [schedule_refines block start count (by omega)]

/-- Feed-forward addition is componentwise addition modulo 2^32. -/
private theorem interpret_add (left right : Vector UInt32 8) :
    Spec.interpret (left.zipWith (· + ·) right) =
      Spec.addState (Spec.interpret left) (Spec.interpret right) := by
  -- Machine-word addition and mathematical bitvector addition both wrap modulo 2^32.
  simp [Spec.interpret, Spec.addState]

/-- Every executable compression implements all sixty-four FIPS bitvector rounds and feed-forward. -/
theorem compress_refines (state : Vector UInt32 8) (block : ByteArray) (start : Nat) :
    Spec.interpret (compress state block start) =
      Spec.compression (Spec.inputWords block start) (Spec.interpret state) := by
  -- Schedule refinement and round-prefix induction cover arbitrary input and chaining state.
  simp only [compress, interpret_add, Spec.compression]
  have rounds := rounds_refines block start state 64 (by omega)
  rw [List.take_of_length_le (by simp)] at rounds
  exact congrArg (Spec.addState (Spec.interpret state)) rounds

/-- The independently stated initial words agree with the executable initialization. -/
theorem initial_refines : Spec.interpret initialState = Spec.initial := by
  -- The independently written tables contain the same eight 32-bit patterns.
  rfl

/-- The two padding formulas choose the same least nonnegative whole-byte count. -/
theorem paddingZeros_refines (size : Nat) : paddingZeros size = Spec.zeroCount size := by
  -- Both formulas select the same residue from zero through sixty-three.
  unfold paddingZeros Spec.zeroCount
  omega

/-- Length encoding agrees with natural-number base-256 digits throughout the standard's domain. -/
private theorem lengthBytes_refines (size : Nat) (admitted : size < 2 ^ 61) :
    (lengthBytes size).data = Array.ofFn (fun i : Fin 8 =>
      UInt8.ofNat ((size * 8 / 2 ^ (56 - 8 * i.val)) % 256)) := by
  -- Equal eight-byte lengths reduce array equality to equality of the eight encoded digits.
  apply Array.ext
  · exact (lengthBytes_size size).trans Array.size_ofFn.symm
  · intro position left right
    have within : position < 8 := by simpa using left
    -- The admitted length bound prevents overflow when converting bytes to a 64-bit bit count.
    have digit := lengthBytes_digit size admitted ⟨position, within⟩
    rw [Array.getElem_ofFn]
    apply UInt8.toNat_inj.mp
    simpa [ByteArray.getElem_eq_getElem_data] using digit

/-- Executable padding agrees byte for byte with the independent mathematical padding. -/
theorem pad_refines (message : ByteArray) (admitted : message.size < 2 ^ 61) :
    pad message = Spec.padded message := by
  -- The original bytes, delimiter, zeros, and exact length digits agree region by region.
  apply ByteArray.ext
  simp only [pad, Spec.padded, ByteArray.data_append, ByteArray.data_push,
    paddingZeros_refines, lengthBytes_refines _ admitted, Array.push_eq_append]

/-- The entire executable block fold follows the model after any number of complete blocks. -/
theorem blocks_refine (message : ByteArray) (state : Vector UInt32 8) (count : Nat) :
    Spec.interpret ((List.range count).foldl
      (fun current block => compress current message (64 * block)) state) =
    Spec.blocks message (Spec.interpret state) count := by
  induction count with
  | zero => rfl
  | succ count ih =>
    -- The next message block extends each state evolution by one proved compression.
    rw [List.range_succ, List.foldl_append]
    simp only [List.foldl_cons, List.foldl_nil, compress_refines, ih, Spec.blocks]

/-- Each mathematical state word is the bitvector interpretation of the corresponding machine word. -/
private theorem interpret_word (state : Vector UInt32 8) (position : Nat) (within : position < 8) :
    (Spec.interpret state).words[position] = state[position].toBitVec := by
  -- The named state fields and the executable vector use the same eight positions.
  have positions : position = 0 ∨ position = 1 ∨ position = 2 ∨ position = 3 ∨
      position = 4 ∨ position = 5 ∨ position = 6 ∨ position = 7 := by omega
  rcases positions with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl <;>
    simp [Spec.interpret, Spec.State.words]

/-- Extracting one machine-word byte agrees with selecting a base-256 digit. -/
private theorem word_digit_refines (word : UInt32) (position : Nat) (within : position < 4) :
    (word >>> (24 - 8 * UInt32.ofNat position)).toUInt8 =
      UInt8.ofNat ((word.toNat / 2 ^ (24 - 8 * position)) % 256) := by
  -- There are only four byte positions, so the position itself cannot overflow a machine word.
  have positionNat : (UInt32.ofNat position).toNat = position :=
    UInt32.toNat_ofNat_of_lt' (by change position < 4294967296; omega)
  -- Multiplying a byte position by eight gives a bit offset between zero and twenty-four.
  have productNat : (8 * UInt32.ofNat position).toNat = 8 * position := by
    simp only [UInt32.toNat_mul, positionNat]
    change (8 * position) % 4294967296 = 8 * position
    exact Nat.mod_eq_of_lt (by omega)
  -- Subtracting that offset from twenty-four never underflows.
  have shiftNat : (24 - 8 * UInt32.ofNat position).toNat = 24 - 8 * position := by
    rw [UInt32.toNat_sub_of_le]
    · simp only [productNat]; rfl
    · apply UInt32.le_iff_toNat_le.mpr
      rw [productNat]
      change 8 * position ≤ 24
      omega
  -- The shift stays below the word width, so the machine's masked shift amount is unchanged.
  apply UInt8.toNat_inj.mp
  simp only [UInt32.toNat_toUInt8, UInt32.toNat_shiftRight, shiftNat,
    Nat.mod_eq_of_lt (show 24 - 8 * position < 32 by omega), Nat.shiftRight_eq_div_pow,
    UInt8.toNat_ofNat']
  change _ % 256 = (_ % 256) % 256
  rw [Nat.mod_mod]

/-- Executable digest output agrees byte for byte with the mathematical word digits. -/
theorem digest_refines (state : Vector UInt32 8) :
    digest state = Spec.digest (Spec.interpret state) := by
  -- Compare all thirty-two output bytes through their word position and big-endian digit.
  apply ByteArray.ext
  apply Array.ext
  · simp [digest, Spec.digest]
  · intro position left right
    have within : position < 32 := by simpa [digest] using left
    simp only [digest, Spec.digest, Array.getElem_ofFn]
    -- The quotient selects a state word and the remainder selects one of its four bytes.
    rw [word_digit_refines _ _ (Nat.mod_lt _ (by decide)),
      interpret_word state (position / 4) (by omega), UInt32.toNat_toBitVec]

/-- SHA-256 execution agrees with the independent modular-bitvector model on every admitted message. -/
theorem hash_refines (message : ByteArray) (admitted : message.size < 2 ^ 61) :
    hash message = Spec.hash message := by
  -- Padding, every compression, and output-digit conversion are each refined separately.
  simp only [hash, Spec.hash, digest_refines, blocks_refine, initial_refines,
    pad_refines message admitted]

end Ssz.Sha256
