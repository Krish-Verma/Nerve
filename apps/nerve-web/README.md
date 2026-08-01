# `nerve-web` — the visual explorer

The interface `nerve serve` serves. It is built here and **compiled into the `nerve` binary**, so
running it needs no Node, no build step and no network — only the repository being indexed.

Nothing in this directory is distributed as source. `npm run build` produces `dist/`, checks it,
and copies it into `crates/nerve-server/assets/`, where `include_bytes!` compiles it in.

## What it shows

| View | Route | Answers |
|---|---|---|
| Overview | `#/overview` | Is this index still true, and what is in it? |
| Symbols | `#/search?q=…` | Where is the thing I am thinking of? |
| Entity · Relations | `#/entity/<id>/relations` | What connects to this? |
| Entity · **Evidence** | `#/entity/<id>/evidence` | **Why does Nerve believe any of it?** |
| Entity · Neighbourhood | `#/entity/<id>/graph` | What does one, two, three hops out look like? |
| Entity · Neighbourhood | `#/entity/<id>/graph?to=…` | How are these two connected? |
| Entity · Source | `#/entity/<id>/source` | What does the file actually say? |
| Gaps | `#/gaps` | What could Nerve **not** work out? |

The **evidence inspector is the centrepiece**. An assertion is written as a sentence, and each
observation behind it hangs off a spine carrying the extractor's id and version, the evidence
source type and directness in plain English, the exact `file:line` that was read, and a freshness
check shown as the arithmetic it is — the hash recorded when the observation was made, the hash
the file has right now, and a verdict. There is no `confidence: float` anywhere in this app,
because there is none in the model.

## Constraints this app is built under

- **Runtime dependencies are `react` and `react-dom`, and nothing else.** No UI kit, chart
  library, graph library, state manager, CSS framework or icon package. The graph is hand-rolled
  SVG. This is a licence-surface decision (`docs/plans/slice-04-visual-explorer.md` P5) and it is
  **enforced by `tools/lint.mjs`**, not merely documented. Vite and TypeScript are build-time only
  and are recorded as not distributed in `third_party/LICENSES.md`.
- **THREAT-MODEL T5.** Repository content is hostile — `<img src=x onerror=alert(1)>` is a legal
  filename and a legal identifier, and the fixtures contain exactly that. Every repository string
  is rendered as a React child or an attribute value, which React escapes. `dangerouslySetInnerHTML`,
  `innerHTML`, `eval` and `document.write` are rejected by `tools/lint.mjs`.
- **The CSP has no `unsafe-inline`.** The server sends `default-src 'none'; script-src 'self';
  style-src 'self'`. The build must therefore emit no inline `<script>` body, no `<style>` block
  and no remote origin. `vite.config.ts` is written to hold that line and `tools/embed.mjs`
  **re-reads the emitted HTML and refuses to embed it if it does not**, rather than trusting the
  configuration. `crates/nerve-server/src/assets.rs` asserts the same property a third time, from
  Rust, against the bytes that shipped. **Never weaken the policy to make a build pass.**
- **Offline.** No CDN, no remote font, no external anything. System font stacks only.
- **The graph is always a bounded neighbourhood of a focused entity**, never "the repository".
  `truncated`, `omitted_nodes` and `frontier_nodes` are drawn, not hidden.
- **The session token lives in memory for the life of the tab.** It is never copied into
  `localStorage` or `sessionStorage` — `nerve serve` promises it never reaches disk, and both of
  those persist for session restore. `tools/lint.mjs` rejects them.

## Layout

```
index.html                 the served document; the only one
vite.config.ts             plugin-free, fixed output names, nothing inlined
src/main.tsx               mounts, and does nothing else
src/App.tsx                shell: bar, rail, routes, and the no-token gate
src/routing.ts             hash routing — the fragment never reaches the server
src/api/{client,types}.ts  the one place this app talks to the server, and the wire shapes
src/hooks.ts               loading / ready / error, with the error carried through intact
src/format.ts              formatting, and the plain-English gloss for every vocabulary term
src/vocab.ts               plain-English readings of the indexer's reason codes
src/search.ts              what to offer when the index matched nothing  (tested)
src/graph/layout.ts        radial layout: distance from the centre is depth  (tested)
src/ui/parts.tsx           the shared vocabulary of the interface
src/ui/Omnibox.tsx         the search field, reachable from anywhere with `/`
src/views/*.tsx            one file per question the interface answers
src/styles/nerve.css       "field notebook, after dark" — see the header comment
tools/lint.mjs             the security and licence gate
tools/embed.mjs            re-checks the built bytes, then copies them into the Rust crate
```

## Working on it

```bash
npm --prefix apps/nerve-web install
npm --prefix apps/nerve-web run check    # typecheck + lint + test
npm --prefix apps/nerve-web run build    # vite build, then verify, then embed
cargo build --release                    # picks the new assets up through include_bytes!
```

There is no dev server workflow. `npm run dev` exists but the app is only ever exercised the way
users meet it — built, embedded, and served by `nerve serve`, because that is the only
configuration where the real Content-Security-Policy applies.

## Design

The direction is set out at the top of `src/styles/nerve.css`. Two rules carry it:

1. **Colour is a claim.** Hue is reserved for what the evidence says — fresh, stale, absent,
   unresolved. Navigation, headings, panels, counts and the graph carry no hue at all, so the eye
   can trust colour to mean exactly one thing.
2. **The literal voice is monospace.** Names, paths, ids and hashes come out of the repository
   verbatim and are set in the system monospace face. Nerve's own words — explanations, empty
   states, verdicts — are set in the system sans. The reader can always tell which is speaking.
