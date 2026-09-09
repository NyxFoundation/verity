import Conformance

/-! Checking the Lean implementation against what the Python specification produces. -/

open Conformance

/--
Runs released fixtures or a generated differential corpus.

Reports every discovered disagreement and exits unsuccessfully if any check failed.
-/
def main (args : List String) : IO UInt32 := do
  match args with
  -- A generated corpus, handed over by the parity test one batch at a time.
  | ["--diff", file] =>
    let (passed, failures) ← runDiff file
    -- Every disagreement is printed, not just the first, so one run shows the shape of it.
    for failure in failures do
      IO.eprintln failure
    IO.println s!"{passed} checks passed, {failures.size} failed"
    return if failures.isEmpty then 0 else 1
  -- Otherwise the released fixtures, from the directory given or the default one.
  | _ =>
    let root : System.FilePath := args.head?.getD "../fixtures"
    -- A missing fixture directory is a failed run rather than an empty successful test suite.
    if !(← root.pathExists) then
      IO.eprintln s!"no fixtures at {root}"
      return 1
    let tally ← run root
    -- All fixture failures are printed before the final success count and exit status.
    for failure in tally.failures do
      IO.eprintln failure
    IO.println s!"{tally.passed} passed, {tally.failures.size} failed"
    return if tally.failures.isEmpty then 0 else 1
