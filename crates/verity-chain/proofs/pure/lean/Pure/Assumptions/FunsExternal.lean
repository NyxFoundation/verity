-- [verity_chain]: external functions.
-- Seeded by hax from Extraction/FunsExternal_Template.lean: fill the holes.
-- hax never modifies this file; after re-extraction, compare it against the
-- regenerated template to see what changed.
import Aeneas
import CoreModels
import Pure.Extraction.Types
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
open verity_chain

/-- Debug formatting is opaque std; the body is not a consensus function. -/
axiom slot_clock.SlotClock.Insts.CoreFmtDebug.fmt :
  slot_clock.SlotClock → core.fmt.Formatter → RustM ((core.result.Result
    Unit core.fmt.Error) × core.fmt.Formatter × (core.fmt.Formatter →
    core.fmt.Formatter))

/-- leanSpec / verity-types constants. `ok` of the transcribed literals. -/
def verity_types.config.INTERVALS_PER_SLOT : RustM Std.U64 := ok 5#u64

def verity_types.config.MILLISECONDS_PER_SLOT : RustM Std.U64 := ok 4000#u64

def verity_types.config.MILLISECONDS_PER_INTERVAL : RustM Std.U64 := ok 800#u64

def verity_types.config.HISTORICAL_ROOTS_LIMIT : RustM Std.Usize := ok 262144#usize

