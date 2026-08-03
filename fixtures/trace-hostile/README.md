# trace-hostile fixture

Thirteen hostile `nerve-trace/v1` artifacts, **one concern per file**, read by
`crates/nerve-index/tests/trace.rs`. Every one must be **refused and counted, disclosing nothing**.

These are attacks, not assertions. A trace artifact is untrusted input exactly as an LCOV report is
(`docs/THREAT-MODEL.md` T9), and no real tracer can emit a traversal path, an oversized record,
malformed UTF-8 or a prompt-injection payload — which is precisely why the artifacts here are
hand-written. Generating them from a producer would make the security half of Slice 11a untestable.

The tree these are aimed at is `fixtures/trace-basic`; the tests copy the artifacts into it, so every
path that is *not* an attack resolves and the refusal is attributable to the rule rather than to luck.

| file | concern | required outcome |
|---|---|---|
| `traversal-dotdot.jsonl` | `../` in a caller path, a callee path, and mid-path (`src/../src/../../`) | `path-refused` ×3, no edge, nothing about the target stored |
| `traversal-backslash.jsonl` | `..\` — on Unix `\` is not a separator, so `std::path::Components` never sees the `..` | `path-refused` ×2. This is the 8b-i defect restated: the shared refusal is syntactic and splits on both separators |
| `traversal-absolute.jsonl` | a leading `/`, and a UNC `\\server\share` | `path-refused` ×3 |
| `oversized-file.jsonl` | an artifact past `MAX_ARTIFACT_BYTES` | `artifact-too-large`, refused **whole and unread**, zero edges |
| `oversized-record.jsonl` | one line past `MAX_RECORD_BYTES` | `record-too-large` on that line only; the well-formed record after it still lands |
| `oversized-string.jsonl` | a `test_id` past `MAX_STRING_BYTES` | `string-too-long`, that record refused, the next one kept |
| `deep-nesting.jsonl` | 50 nested arrays under an unknown record key | `nesting-too-deep`, **measured before `serde_json` is called** |
| `duplicate-run-id.jsonl` | `run_id` replayed from `trace-basic/trace/bound.jsonl`, with a different edge and `count: 9999` | `run-id-conflict` counted and reported; **nothing already recorded is overwritten** |
| `malformed-utf8.jsonl` | raw `0xff 0xfe 0x80` inside a `test_id` | `invalid-utf8-line`, that line only |
| `sql-injection.jsonl` | `'); DROP TABLE observation; --` in `run_id`, `test_framework`, `test_id` and `worker` | stored as bound-parameter data or not at all; every table still present, the graph unchanged in shape |
| `fts5-syntax.jsonl` | FTS5 operators (`NEAR()`, `*`, `^`, `-`, quotes) in `run_id` and `test_id` | inert; `entity_fts` never sees them because no entity is created from artifact text |
| `prompt-injection.jsonl` | instruction text in `producer` and `test_id` | inert data; it reaches no MCP surface, and `nerve why` labels it as evidence rather than acting on it |
| `cross-repository.jsonl` | `repository_root_name: "some-other-project"` | `other-repository`, artifact refused **whole**; two otherwise-valid records produce nothing |
| `state-substitution.jsonl` | a plausible 40-hex `git_commit` and 64-hex `content_merkle` for a tree that is not this one | binding reported `stale`, **never `bound` and never `unverified`** — an attacker cannot upgrade a trace's binding by asserting a state |
| `header-unknown-key.jsonl` | `"paths_are_absolute": true` | `header-unknown-key`, artifact refused whole. A header key Nerve does not understand may change the meaning of the whole file |

## Tokens the test expands, and why they are not committed as bytes

Three files carry a placeholder rather than the literal payload:

| token | file | what the test substitutes | why not committed |
|---|---|---|---|
| `__PAD_ARTIFACT__` | `oversized-file.jsonl` | enough bytes to pass `MAX_ARTIFACT_BYTES` | a 32 MiB fixture must not be committed (`CLAUDE.md` §9) |
| `__PAD_RECORD__` | `oversized-record.jsonl` | enough bytes to pass `MAX_RECORD_BYTES` | an 8 KiB wall of one character is noise in a diff, and the intent is clearer as a token |
| `__PAD_STRING__` | `oversized-string.jsonl` | one byte past `MAX_STRING_BYTES` | same |
| `__INVALID_UTF8__` | `malformed-utf8.jsonl` | the bytes `0xff 0xfe 0x80` | a file that is not valid UTF-8 cannot be written or reviewed as text |

Each substitution is derived from the bound constant it attacks, so tightening a bound cannot leave
its attack testing nothing.

## `duplicate-run-id` is counted, not refused — and the brief and the plan disagree here

The Slice 11a brief lists a duplicate run id among the cases that must be *refused*. Plan §7's table
says the opposite, explicitly:

> | a corrected artifact with the same `run_id` | **imported, and the conflict is reported.** Nothing is silently overwritten |

The plan is followed, because refusing would discard a corrected artifact — the case §7 names — and
because refusal buys nothing an attacker cares about: the conflict is reported, the earlier evidence
is intact, and the import exits non-zero. What the attack must not achieve is *overwriting*, and that
is what the test asserts.
