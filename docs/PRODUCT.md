# Nerve — Product Definition

## What Nerve is

A fully local, offline-first software-understanding system. Point it at a repository and it
builds a living **evidence graph** that humans and coding agents can inspect.

```bash
cd my-project
nerve init
nerve index
nerve serve     # (Slice 4)
```

## The promise

> Every conclusion is backed by inspectable evidence, scoped to a repository state, and capable
> of saying when the evidence is missing, stale, uncertain, or contradictory.

Nerve is not trying to be the biggest graph or support the most languages. It is trying to be
the one you can trust, because it tells you what it does not know.

## Users

- **Developers** exploring or changing unfamiliar code, visually and from the terminal.
- **Coding agents** requesting compact, grounded, cited context instead of grepping.

Neither is secondary.

## Questions Nerve should answer

How does this component work · How does data move through this system · What calls this symbol ·
What does this symbol call · What will this change affect · Which tests exercise this code ·
Which paths appear untested · Why was this designed this way · Which documentation no longer
matches the code · Which relationships are directly proven, inferred, test-observed, or
runtime-observed · What changed between two repository states · **What does Nerve not know** ·
What should a developer verify before changing this.

## Principles

1. **Offline-first.** No cloud account, API key, external model, telemetry, analytics, source
   upload, or network connection after dependencies are installed.
2. **Trust through evidence.** Directly extracted, type-resolved, framework-inferred,
   test-covered, runtime-observed, document-stated, human-confirmed, and LLM-derived claims are
   never silently presented as equally certain.
3. **Humans and agents.** Four first-class surfaces: CLI, visual application, MCP server, CI.
4. **One core, many surfaces.** All surfaces call the same application services and schemas.
   Business logic is never duplicated in a surface.
5. **Honest insufficiency.** "I don't have enough evidence, here is what to look at" is a
   first-class successful answer, not a failure.

## Non-goals

Cloud collaboration · enterprise permissions · maximum language count · multimodal ingestion ·
telemetry-driven product analytics · being a linter, a test runner, or an IDE.

## Long-term surface

Repository overview · architectural communities · packages and modules · functions and classes ·
call and dependency paths · test-coverage relationships · documentation relationships ·
change impact · evidence provenance · contradictions · unknown and unresolved relationships ·
index freshness · historical change · runtime observations.
