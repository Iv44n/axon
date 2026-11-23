---
type: decision
status: accepted
updated: 2026-11-22
related: [../../.claude/skills/axon-data-oriented-design/SKILL.md, 0005-damage-tracking.md]
---
# 0009 · Data-oriented design; no caches

**Decision:** the data layout is designed first. Flat arrays instead of hash tables, indices
instead of pointers, hot data packed into the bits of the index, struct-of-arrays or
array-of-structs chosen from the access pattern. No cache is added, and the existing ones are
removed by redesigning the data so the work is cheap or unnecessary. The rules and the
questions to ask live in the `axon-data-oriented-design` skill. Decided by the user on
2026-11-22.

**Why:** RAM and CPU are the project's goal, and a cache hides a design that is too slow
while costing RAM for the copy and bugs for the invalidation. First application, 2026-11-22:
the per-row instance cache plus its flat mirror became a fixed-stride cell buffer indexed by
`row*cols+col`, written per damaged row run, with underline, strikeout and cursor as flags in
the high bits of the tile index. No CPU-side copy, one cell per position, no `rc` field
(position is the index), and the frame is byte-identical to the previous renderer on 5 scenes.

**Rejected:** caching as a general speedup tool; keeping the `HashMap` for glyph lookup.

**Revisit if:** a measurement shows a redesign cannot meet a budget and a cache is the only
way. Then a new ADR with the data.
