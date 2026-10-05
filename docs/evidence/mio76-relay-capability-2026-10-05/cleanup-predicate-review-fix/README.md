# Cleanup evidence predicate correction

The independent Standards review found that Python treats `False` as integer zero.
The canonical physical-case receipt validator could therefore accept false exit
or retirement counts, and could accept a negative late socket count.

The narrow correction requires an actual integer process exit of zero and actual
nonnegative integer RTC association, pump, baseline socket and late socket counts.
It retains the ownership booleans and the late socket comparison. Unknown or
malformed observations remain unqualified; numeric zero remains valid, and
negative/nonzero process exits keep their original failure meaning.

The offline evidence suite first failed six counterexamples across 15 tests, then
passed all 15 after the correction. Python compilation passed. The byte-exact
compressed red/green logs and before/after source hashes are pinned in
[receipt.json](receipt.json). No SDK, service, build, device, public or STUN case was
rerun. The 29 previously reviewed controlled evidence files and their captured
actual integer counts remain unchanged.
