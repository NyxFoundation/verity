# Data Representation Across Zones

How Verity represents a value in the host language is not a single decision made once for the whole codebase. It tracks the verification guarantee the value currently lives under. The same protocol quantity — a slot number, a validator index — is a precise, bounded type inside the proven core and a plain machine integer at the unproven edge, with an explicit conversion where the two meet. This page states that policy and the reason it follows from the [design philosophy](../design-philosophy.md).

This is a consequence of beliefs Verity already holds, not a new rule. It is written down here because it decides the shape of every container and every arithmetic operation that follows.

## The principle

Representation precision follows the guarantee. A type that a proof reasons about carries its invariants in the type; a type that only shuttles bytes between the network and the parser does not need to. Because Verity's verification boundary is a first-class, *moving* part of the architecture rather than a fixed line, the right representation is decided per zone — and is expected to change for a given component when that component crosses the boundary.

## In Verity Consensus (the proven core)

Inside the proven core, protocol quantities are concrete, named types, not raw integers. A slot is a `Slot`, a validator index is a `ValidatorIndex`, and the two cannot be confused or transposed, because the type system forbids it. This is the same discipline the leanSpec reference applies in Python, where `Slot` is a distinct subtype of a bounded unsigned integer rather than an alias for it — and Verity holds the proven core to at least that standard.

Two beliefs force this choice:

- **Explicitness over cleverness.** The implementation must read like the Lean 4 model it corresponds to. The model distinguishes a slot from an index; so does the code. A raw `u64` standing for three unrelated quantities loosens exactly the correspondence the proof depends on.
- **Arithmetic that mirrors proven invariants.** Overflow, underflow, and truncation are conditions the model rules out, not surprises caught downstream. Numeric operations in the core are fallible and explicit where the bound matters, so that the arithmetic corresponds one-to-one with the guarantees the proof discharges. A silently wrapping `u64` cannot make that correspondence.

This is also why the core does not lean on macro-generated type machinery to conjure its containers: structure that a macro hides is structure a reviewer and a proof must recover by hand.

## At the edges (Runtime Shell / I/O Edge)

Outside the boundary — networking, storage, the RPC surface — none of this applies. Code there exists to feed the core clean, well-typed inputs and to carry its outputs to the network; no proof reasons about its internals. Plain machine integers and a conventional, derive-driven SSZ stack are entirely appropriate, and the edge is free to look like an ordinary high-quality Rust client. Precision here would buy nothing and cost ergonomics.

## At the boundary

Values do not drift across the boundary untyped. An input arriving from the edge is validated and converted — promoted — into the core's domain types at the point it enters; a value leaving the core is lowered back to its wire representation on the way out. The boundary is where range checks and well-formedness checks live, so that everything inside the core has already earned its type.

This costs nothing in conformance, because **the wire format is independent of the host representation.** A `Slot` newtype and a raw `u64` serialize to byte-identical SSZ. leanSpec fixes the serialized shape — field order and encoding are consensus-critical, since they determine what gets hashed and signed — and Verity matches that shape exactly regardless of how richly it types the value in memory. Rich domain types in the core therefore buy proof alignment for free, without moving a single byte on the wire.

## Because the boundary moves

The boundary is designed to move: serialization may be pulled into the proven core once it is verified, and a component may be pushed back out if proving demands a different target. Moving a component across is meant to be a re-binding, not a redesign. Representing a boundary-adjacent value in the provable shape from the start is what makes that true — when the component is promoted, its types are already what the core requires, and nothing downstream has to be rewritten to absorb the change.

## How the existing clients compare

The Rust Lean clients make the trade-off concrete, and they consistently choose the edge style throughout — because they are uniformly unverified, and because pervasive newtypes in Rust carry real boilerplate. Verity differs only where it must: inside the proven core, where the guarantee changes the calculus.

| Concern | leanSpec (Python) | ethlambda (Rust) | ream, Lean stack (Rust) |
|---|---|---|---|
| Slot / proposer index | distinct `Slot` / index newtypes, with domain methods | raw `u64` | raw `u64` |
| Hash | dedicated `Bytes32` type | `H256` newtype (no bound checks) | `B256` (alloy) |
| Collection length bound | bounded SSZ list | library-typed | type-level (`VariableList<_, N>`, `BitList`) |
| Value range / type checking | enforced (bounded ints, strict models) | none beyond the type | none beyond the derive |
| SSZ mechanism | bespoke SSZ type hierarchy | `libssz` derives | `ethereum_ssz` + `tree_hash` derives |
| Domain newtypes for scalars | yes | no | no |

References for the shapes above: ethlambda's `crates/common/types/src/block.rs` and `checkpoint.rs`; ream's `crates/common/consensus/lean/src/block.rs` and `state.rs`; leanSpec's `src/lean_spec/types/slot.py`, `uint.py`, and `checkpoint.py`.

The reading is straightforward. The edge style is right for an unverified client and right for Verity's own unproven zones. It is the wrong default for Verity Consensus, where the proof is the product — and there, the leanSpec discipline, or stricter, is the one that pays off. ream's habit of putting collection capacities in the type is the part of the edge style worth carrying inward: a length bound expressed in the type is exactly the kind of invariant the core wants to state once and rely on everywhere.
