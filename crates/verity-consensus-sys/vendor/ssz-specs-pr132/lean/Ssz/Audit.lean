import Lean

/-! Reject nonstandard axioms anywhere in the specification or its proofs. -/

open Lean Elab Command in
elab "audit_ssz" : command => do
  -- Audit imported declarations as well as local ones, so new proofs need no registry entry.
  let environment ← getEnv
  for (name, _) in environment.constants.toList do
    -- Private helper declarations are checked under their original namespace as well.
    let publicName := (privateToUserName? name).getD name
    if (`Ssz).isPrefixOf publicName then
      -- These are Lean's standard logical axioms, not assumptions about SSZ or SHA-256.
      let axioms ← Lean.collectAxioms name
      -- Every transitive axiom dependency must belong to the three standard logical foundations.
      for assumption in axioms do
        unless [``propext, ``Classical.choice, ``Quot.sound].contains assumption do
          throwError "{name} depends on forbidden axiom {assumption}"
