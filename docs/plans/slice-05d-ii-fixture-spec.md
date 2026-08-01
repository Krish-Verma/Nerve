# Slice 5d-ii — supersession fixture corpus, exact specification

**Authored by the orchestrator, not by the implementer.** The measurement corpus and its ground
truth are the oracle. `docs/plans/slice-15-real-world-validation.md` §1 records why this matters:
fixtures written by the same mind that wrote the resolver measure self-consistency, not accuracy.
The implementer of 5d-ii writes code to satisfy this corpus and does not author it.

Create `fixtures/md-supersession/` with **exactly** these files and **exactly** this content.
Transcribe verbatim — this is a transcription task, not a design task. Do not add, reword, tidy,
reflow or "improve" anything. Trailing newline at end of every file. No front matter anywhere.

---

## `README.md`

```
# md-supersession

A corpus for `Document SUPERSEDES Document`. Every case is deliberate; see
`docs/plans/slice-05d-ii-fixture-spec.md` for what each file is for.
```

## `docs/decisions/ADR-0001-original.md`

```
# ADR-0001 — The original decision

**Status:** Superseded · **Superseded by:** [ADR-0002](ADR-0002-replacement.md)

The decision that ADR-0002 replaces. Stated from the superseded side.
```

## `docs/decisions/ADR-0002-replacement.md`

```
# ADR-0002 — The replacement decision

**Status:** Accepted · **Supersedes:** [ADR-0001](ADR-0001-original.md)

The same edge as ADR-0001 states, from the other side. One assertion, two observations.
```

## `docs/decisions/ADR-0003-bare-identifier.md`

```
# ADR-0003 — Supersession by bare identifier

**Status:** Accepted · **Supersedes:** ADR-0004

No link. The target is named by its identifier alone.
```

## `docs/decisions/ADR-0004-bare-target.md`

```
# ADR-0004 — The target named without a link

**Status:** Superseded

Named by ADR-0003 using a bare identifier.
```

## `docs/decisions/ADR-0005-section-form.md`

```
# ADR-0005 — Supersession stated in a section

**Status:** Superseded

## Superseded by

[ADR-0006](ADR-0006-section-target.md)
```

## `docs/decisions/ADR-0006-section-target.md`

```
# ADR-0006 — The section-form target

**Status:** Accepted

Named by the `## Superseded by` section of ADR-0005.
```

## `docs/decisions/ADR-0007-cycle-a.md`

```
# ADR-0007 — Cycle, first link

**Status:** Accepted · **Supersedes:** ADR-0008

Part of a three-ADR cycle. Each edge is individually evidenced.
```

## `docs/decisions/ADR-0008-cycle-b.md`

```
# ADR-0008 — Cycle, second link

**Status:** Accepted · **Supersedes:** ADR-0009

Part of a three-ADR cycle.
```

## `docs/decisions/ADR-0009-cycle-c.md`

```
# ADR-0009 — Cycle, third link

**Status:** Accepted · **Supersedes:** ADR-0007

Closes the cycle back to ADR-0007.
```

## `docs/decisions/ADR-0010-self.md`

```
# ADR-0010 — Supersedes itself

**Status:** Accepted · **Supersedes:** ADR-0010

A document cannot replace itself. This is unresolved, not an edge.
```

## `docs/decisions/ADR-0011-missing-target.md`

```
# ADR-0011 — Names a target that does not exist

**Status:** Accepted · **Supersedes:** ADR-9999

There is no ADR-9999 in this corpus.
```

## `docs/decisions/ADR-0012-first.md`

```
# ADR-0012 — First document carrying this identifier

**Status:** Accepted

One of two files in this corpus whose identifier parses to ADR-0012.
```

## `notes/ADR-0012-second.md`

```
# ADR-0012 — Second document carrying this identifier

**Status:** Accepted

The file name matches ADR-<digits>, so this is an ADR wherever it sits.
```

## `docs/decisions/ADR-0013-ambiguous-reference.md`

```
# ADR-0013 — Names an ambiguous target

**Status:** Accepted · **Supersedes:** ADR-0012

Two documents carry that identifier. Nerve refuses to choose.
```

## `docs/decisions/ADR-0014-empty-field.md`

```
# ADR-0014 — The field is present and empty

**Status:** Accepted · **Supersedes:**

Nothing follows the field.
```

## `docs/decisions/ADR-0015-external.md`

```
# ADR-0015 — Names an external target

**Status:** Accepted · **Supersedes:** [ADR-1](https://example.com/adr-1)

An external destination is counted and never fetched, and never becomes an entity.
```

## `docs/decisions/ADR-0016-traversal.md`

```
# ADR-0016 — Names a target outside the repository

**Status:** Accepted · **Supersedes:** [escape](../../../../../etc/passwd)

The destination escapes the repository root and must be refused.
```

## `docs/decisions/ADR-0017-prose-only.md`

```
# ADR-0017 — Says the word in prose only

**Status:** Accepted

This decision supersedes ADR-0001 in spirit, and the phrase `Supersedes: ADR-0001`
appears here as a code span. Neither is a field. No edge may be emitted.
```

## `docs/decisions/ADR-0018-status-only.md`

```
# ADR-0018 — Superseded status with no target

**Status:** Superseded

Nothing names what replaced it. A status is not a target.
```

## `docs/decisions/ADR-0019-contradiction-target.md`

```
# ADR-0019 — Superseded but still marked Accepted

**Status:** Accepted

ADR-0020 supersedes this document, yet this document still claims to be accepted.
```

## `docs/decisions/ADR-0020-contradicts.md`

```
# ADR-0020 — Supersedes a document that still claims Accepted

**Status:** Accepted · **Supersedes:** [ADR-0019](ADR-0019-contradiction-target.md)

The contradiction is checkable by string comparison and needs no semantics.
```

## `docs/decisions/ADR-0021-chain-head.md`

```
# ADR-0021 — Head of a two-hop chain

**Status:** Accepted · **Supersedes:** [ADR-0002](ADR-0002-replacement.md)

ADR-0021 supersedes ADR-0002, which supersedes ADR-0001.
```

## `docs/decisions/ADR-0022-code-block.md`

```
# ADR-0022 — The field appears inside a fenced code block

**Status:** Accepted

The block below is documentation of the syntax, not a use of it.

```markdown
**Supersedes:** ADR-0001
```

No edge may be emitted from a fenced code region.
```

*(Transcriber: the file above contains a nested fenced block. Write the file so that its own
content is the four-backtick-free text shown, i.e. the file contains a ```` ```markdown ```` fence
containing `**Supersedes:** ADR-0001` and a closing ```` ``` ````.)*

## `notes/design-note.md`

```
# A design note that is not an ADR

**Supersedes:** [the old note](old-note.md)

Supersession evidence is explicit here, and the file is not an ADR. The field is what
makes it evidence, not the file name.
```

## `notes/old-note.md`

```
# The old note

Replaced by `design-note.md`.
```

---

## Why each file exists

| File | Case |
|---|---|
| ADR-0001 / ADR-0002 | positive, both directions, **one assertion with two observations** |
| ADR-0003 / ADR-0004 | positive, bare-identifier target |
| ADR-0005 / ADR-0006 | positive, `## Superseded by` section form |
| ADR-0007 / 0008 / 0009 | **cycle** — three edges, all emitted, cycle detected and counted |
| ADR-0010 | self-supersession — unresolved |
| ADR-0011 | missing target — unresolved |
| ADR-0012 ×2 / ADR-0013 | **ambiguous** target — refused, never guessed |
| ADR-0014 | empty field — unparsed |
| ADR-0015 | external destination — counted, never an entity |
| ADR-0016 | traversal — refused |
| ADR-0017 | **negative**: prose and a code span, no field, no edge |
| ADR-0018 | **negative**: `Superseded` status with no target, no edge |
| ADR-0019 / ADR-0020 | status contradiction — superseded yet still `Accepted` |
| ADR-0021 | chain of two hops over ADR-0002 |
| ADR-0022 | **negative**: field inside a fenced code block, no edge |
| notes/design-note.md | positive on a **non-ADR** document, link form |

## Recorded design decision

**The supersession fields are recognised on every document, not only on ADRs.** The evidence is the
explicit field, not the file name; refusing to record `**Supersedes:** [x](y.md)` in `design-note.md`
because the file is not called `ADR-*` would be an arbitrary rule with no evidential basis. The
**bare-identifier** form still resolves only against parsed ADR identifiers, because that is the only
identifier namespace that exists.
