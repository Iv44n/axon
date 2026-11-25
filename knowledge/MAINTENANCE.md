# How the tree grows

Read this only if you are going to write in `knowledge/`. The goal: an agent finds what it
needs by reading 3 small files (root → branch → leaf), not the whole tree.

## Shape
- **Branch** = folder with an `INDEX.md`. **Leaf** = one `.md` file on a single topic.
- `INDEX.md` only navigates: one line per child, `- [Title](path) — hook`. The hook
  (≤ 15 words) must be enough to decide whether to open the child. No content.
- Leaf ≤ 80 lines. If it grows past that, turn it into a branch: a folder with the same name,
  an `INDEX.md` and smaller leaves. Update the links that pointed to it.
- Index ≤ 25 entries. If it grows past that, group the children into sub-branches.
- Lowercase names with hyphens (`fallback-fonts.md`). ADRs: `NNNN-title.md`, in the sub-branch of their topic under
  `decisions/` (`core`, `interface`, `workspace`); the number is global.

## Leaf header
    ---
    type: decision | task | risk | log | reference
    status: (see below)
    updated: YYYY-MM-DD
    related: [relative/path.md]
    ---

Statuses: decision → `proposed`, `accepted`, `superseded-by-NNNN` · risk → `open`,
`mitigated`, `accepted` · task → `pending`, `in-progress`, `done` · log and
reference → `current`, `obsolete`.

## Rules
1. New node = file + its line in the parent `INDEX.md`, in the same change. New branch:
   also in the root `INDEX.md`.
2. No duplication. If it already lives in another leaf, a skill or the code, link to it.
3. Decisions are never rewritten: a change is a new ADR, and the old one becomes
   `superseded-by-NNNN`.
4. Logs only grow, newest entry on top.
5. Absolute dates (2026-11-22), never relative ones.
6. When a task is finished: tick it in `plan/next-steps.md`, update `plan/phases.md` if a
   phase changes, and log measurements if relevant.
7. If a leaf contradicts the code, the code wins: fix the leaf.
8. Knowledge that fits no branch → a new branch, not a loose leaf in the root.

## ADR template
    ---
    type: decision
    status: accepted
    updated: YYYY-MM-DD
    related: []
    ---
    # NNNN · Title phrased as a decision

    **Decision:** one or two sentences.
    **Why:** the reasons, with data when available.
    **Rejected:** alternatives and why not.
    **Revisit if:** the concrete condition that would reopen the decision.
