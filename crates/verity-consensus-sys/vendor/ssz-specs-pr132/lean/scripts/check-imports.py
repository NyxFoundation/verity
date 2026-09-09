"""Check that every SSZ module is reachable from the audited entry point."""

import re
import sys
from pathlib import Path

# Only specification modules belong to the axiom audit; executable test readers are separate.
root = Path(__file__).resolve().parents[1]
modules = {
    ".".join(path.relative_to(root).with_suffix("").parts): path
    for path in (root / "Ssz").rglob("*.lean")
}
modules["Ssz"] = root / "Ssz.lean"

# Follow transitive imports, so a proof can be included through its natural parent module.
pending = ["Ssz"]
visited: set[str] = set()
while pending:
    module = pending.pop()
    if module in visited or module not in modules:
        continue
    visited.add(module)
    for line in modules[module].read_text().splitlines():
        if match := re.match(r"^(?:public\s+)?import\s+(.+)", line):
            pending.extend(match[1].split("--", 1)[0].split())

# An unimported file could otherwise escape both the default build and the axiom audit.
missing = sorted(modules.keys() - visited)
if missing:
    sys.exit("SSZ modules missing from the audited import closure:\n" + "\n".join(missing))
