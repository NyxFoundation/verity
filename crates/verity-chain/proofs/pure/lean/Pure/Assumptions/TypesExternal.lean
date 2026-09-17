-- [verity_chain]: external types.
-- Seeded by hax from Extraction/TypesExternal_Template.lean: fill the holes.
-- hax never modifies this file; after re-extraction, compare it against the
-- regenerated template to see what changed.
import Aeneas
import CoreModels
open CoreModels Aeneas
open Aeneas.Std hiding namespace core alloc
open RustM ControlFlow Error
open Std.Do
set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
set_option linter.style.whitespace false
set_option linter.style.setOption false
set_option linter.style.longLine false

/- You can set the `maxHeartbeats` value with the `-max-heartbeats` CLI option -/
set_option maxHeartbeats 1000000

/- You can set the `maxRecDepth` value with the `-max-recdepth` CLI option -/
set_option maxRecDepth 2048

/-- Cross-crate `verity-types` newtypes. Not extracted: Charon start-from
    stays inside `verity-chain`. Values match `crates/verity-types`. -/
@[reducible]
def verity_types.primitives.Slot := Std.U64

@[reducible]
def verity_types.primitives.ValidatorIndex := Std.U64

@[reducible]
def verity_types.primitives.Interval := Std.U64

structure verity_types.checkpoint.Checkpoint where
  root : Array Std.U8 32#usize
  slot : verity_types.primitives.Slot

axiom core.time.Duration : Type

axiom libssz_types.bitlist.SszBitlist (N : Std.Usize) : Type

inductive libssz_types.error.TypeError where
| InvalidLength : Std.Usize → Std.Usize → libssz_types.error.TypeError
| OverCapacity : Std.Usize → Std.Usize → libssz_types.error.TypeError
| Custom : String → libssz_types.error.TypeError

