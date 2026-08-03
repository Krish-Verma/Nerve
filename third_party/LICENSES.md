# Third-party dependencies

Every crate reachable from this workspace, with its exact resolved version, SPDX license
expression, and why it is here. CLAUDE.md §1 requires this file to be complete.

Versions come from `Cargo.lock`; licenses come from `cargo metadata`. Regenerate the facts
with:

```bash
cargo metadata --format-version 1 | \
  python3 -c "import json,sys; m=json.load(sys.stdin); [print(p['name'], p['version'], p.get('license')) for p in sorted(m['packages'], key=lambda p: p['name'])]"
```

## Clean-room statement

No dependency here is a code-knowledge-graph, code-intelligence, or code-search product.
Every entry is a foundational library: a parser, a database, a hasher, a filesystem walker,
an argument parser, a serializer, or an error/utility crate. This is the allowance in
CLAUDE.md §1 ("permissively licensed foundational libraries") and nothing more.

## Licensing posture

All 101 transitive dependencies are permissively licensed. Nothing is copyleft-encumbered for
distribution purposes:

- `r-efi 6.0.0` offers `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; we take MIT. It is a UEFI
  target dependency of `getrandom` and is not linked on Linux, macOS or Windows.
- `unicode-ident 1.0.24` is `(MIT OR Apache-2.0) AND Unicode-3.0`. Unicode-3.0 is a
  permissive data licence covering the Unicode character tables.
- `foldhash 0.1.5` is Zlib; `arrayref 0.3.9` is BSD-2-Clause. Both are permissive.
- `blake3` and `constant_time_eq` offer CC0-1.0 among their options.
- `adler2 2.0.1` offers `0BSD OR MIT OR Apache-2.0`; we take MIT. 0BSD is permissive with no
  attribution requirement at all.
- `miniz_oxide 0.8.9` offers `MIT OR Zlib OR Apache-2.0`; we take MIT.

`libsqlite3-sys` vendors SQLite itself, which is **public domain**.

## Direct dependencies

| Crate | Version | License | Used by | Purpose |
|---|---|---|---|---|
| `anyhow` | 1.0.104 | MIT OR Apache-2.0 | nerve-cli | Error context at the binary boundary |
| `blake3` | 1.8.5 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | nerve-core, nerve-index | Entity/occurrence/assertion ids, content hashes, state merkle |
| `clap` | 4.6.4 | MIT OR Apache-2.0 | nerve-cli | Command-line parsing (derive feature) |
| `flate2` | 1.1.9 | MIT OR Apache-2.0 | nerve-index | zlib inflate for Git objects (Slice 12a). `default-features = false, features = ["rust_backend"]` — pure Rust, no C |
| `ignore` | 0.4.31 | Unlicense OR MIT | nerve-index | Directory walk honouring `.gitignore`, `.ignore`, `.nerveignore` |
| `rusqlite` | 0.37.0 | MIT | nerve-store | SQLite bindings; `bundled` feature statically links SQLite with FTS5 |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | all | Serialization derives |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 | all | Canonical dump, `--json` output, entity `meta` and observation `details` |
| `signal-hook` | 0.4.4 | MIT OR Apache-2.0 | nerve-cli | SIGINT/SIGTERM handling so `nerve serve` stops gracefully |
| `thiserror` | 2.0.19 | MIT OR Apache-2.0 | nerve-core, nerve-store, nerve-index | Error type derives |
| `tiny_http` | 0.12.0 | MIT OR Apache-2.0 | nerve-server | Blocking HTTP/1.1 server for the loopback query surface |
| `toml` | 0.9.12 | MIT OR Apache-2.0 | nerve-index | `.nerve/config.toml` |
| `tree-sitter` | 0.25.10 | MIT | nerve-index | Incremental parser runtime |
| `tree-sitter-javascript` | 0.25.0 | MIT | nerve-index | JavaScript / JSX grammar |
| `tree-sitter-python` | 0.25.0 | MIT | nerve-index | Python grammar (Slice 9a) |
| `tree-sitter-typescript` | 0.23.2 | MIT | nerve-index | TypeScript and TSX grammars |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 | dev-dependency | Temporary directories in tests |

### The Slice 4a HTTP surface, and what it cost

`nerve serve` is a loopback-only, single-user, read-only server with nine JSON endpoints and
some embedded static assets. Tokio + axum + tower + hyper would have pulled roughly 80–100
crates into a workspace that had 89, and introduced an async execution model into a codebase
that is deliberately serial for determinism
(`docs/plans/slice-04-visual-explorer.md` §P1).

`tiny_http 0.12.0` was adopted instead, with `default-features = false` so no TLS stack is
compiled in. Measured cost, `cargo tree -p nerve-server`:

| Crate | Version | License | Why it is here |
|---|---|---|---|
| `tiny_http` | 0.12.0 | MIT OR Apache-2.0 | The server itself |
| `ascii` | 1.1.0 | Apache-2.0 OR MIT | Header values, as ASCII strings |
| `chunked_transfer` | 1.5.0 | MIT OR Apache-2.0 | Chunked encoding. Unused: `nerve-server` sets `chunked_threshold` to `usize::MAX`, so every response is `Content-Length`-framed |
| `httpdate` | 1.0.3 | MIT OR Apache-2.0 | `Date` header formatting |

`log 0.4.33` is also a `tiny_http` dependency and was already in the tree via `ignore`.
The optional `openssl`, `rustls`, `rustls-pemfile` and `zeroize` features are **not** enabled;
serving over TLS on loopback would be theatre, and `rustls` is on the forbidden list below.

`signal-hook 0.4.4` gives `nerve serve` a graceful `Ctrl-C`. It brings one crate,
`signal-hook-registry 1.4.8`; both its other dependencies (`libc`, `errno`) were already
present. `ctrlc` was measured as the alternative and rejected: on macOS it pulls
`nix`, `dispatch2`, `objc2`, `objc2-encode`, `block2`, `bitflags` and `cfg_aliases` — seven
crates for one signal.

**Net: 89 → 95 crates, all `MIT OR Apache-2.0`.**

### The Slice 9a Python grammar, and what it cost

`tree-sitter-python 0.25.0`, **MIT**, from the same `tree-sitter` organisation as the two
grammars already here. Measured cost: the workspace went from **100 to 101** `[[package]]`
entries in `Cargo.lock` — 95 to 96 third-party crates. **One crate, and only one**: its two
dependencies, `tree-sitter-language 0.1.7` and `cc 1.4.0`, were already in the tree, and its
`0.25` line matches `tree-sitter 0.25.10` so no existing grammar needed a version bump.

**`tree-sitter-stack-graphs-python` was rejected on clean-room grounds**, and the distinction is
the one CLAUDE.md §1 draws. A bare grammar is a *parser*: it turns bytes into a syntax tree and
answers no question about what a name means. Stack-graphs is a *name-resolution and
code-navigation engine* — a competing code-intelligence implementation — and depending on one
would make Nerve's answers someone else's. Nerve resolves Python names itself
(`crates/nerve-index/src/pyresolve.rs`), as it already does for TS/JS.

### The Slice 12a decompressor, and what it cost

Every Git object is zlib-deflated — measured, `78 01`, in
`docs/plans/slice-12-git-object-access-analysis.md` §2 — and neither the standard library nor any
of the 101 packages already here has an inflate. So reading Git history at all required exactly one
new capability, and the question was which dependency to spend on it.

**`flate2 1.1.9`, `MIT OR Apache-2.0`, with `default-features = false, features = ["rust_backend"]`.**
The feature selection is load-bearing rather than tidy: every other backend `flate2` offers
(`zlib`, `zlib-ng`, `zlib-ng-compat`, `cloudflare_zlib`, `libz-sys`, `miniz-sys`) is a C library with
a build script, and `rust_backend` is `miniz_oxide`, which is pure Rust.
`crates/nerve-index/tests/gitobj.rs::the_inflate_backend_is_pure_rust` asserts the selection from
the manifest **and** asserts that no C zlib appears in `Cargo.lock`, so a C backend arriving
transitively is caught even if the feature list still reads correctly.

**Measured cost: `Cargo.lock` went from 101 to 106 `[[package]]` entries — five crates, not the
three the analysis estimated.** `git diff Cargo.lock`, in full:

| Crate | Version | License | Why it is here |
|---|---|---|---|
| `flate2` | 1.1.9 | MIT OR Apache-2.0 | The inflate itself |
| `miniz_oxide` | 0.8.9 | MIT OR Zlib OR Apache-2.0 | The pure-Rust DEFLATE implementation `rust_backend` selects |
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | The Adler-32 checksum in a zlib wrapper; a dependency of `miniz_oxide` |
| `simd-adler32` | 0.3.10 | MIT | The same checksum, vectorised. **Not estimated**: `miniz_oxide`'s `simd` feature is not a default, but `flate2` enables it, so it is compiled |
| `crc32fast` | 1.5.0 | MIT OR Apache-2.0 | gzip's CRC-32. **Not estimated**: unused by Nerve, which reads only zlib streams, but `flate2`'s `miniz_oxide` feature enables it unconditionally and it is not separable |

The two the estimate missed are both checksum crates pulled in by feature unification rather than by
choice, which is exactly why `CLAUDE.md` §1 asks for the measurement instead of the estimate.

`crc32fast` has a **build script**, and it was inspected rather than assumed: it runs
`rustc --version` and emits one `cargo:rustc-cfg` for a stabilised ARM CRC-32 intrinsic. There is no
C compilation and no C source — `find` for `*.c`, `*.h`, `*.cc` across all five crates returns
nothing. A build script that probes the compiler is the same category as `cc`, which has been in the
tree since the first tree-sitter grammar; it is build-time only, and
`crates/nerve-cli/tests/no_subprocess.rs` scans `crates/*/src/**`, which is Nerve's own product code.

**`gix` and `git2` were both rejected**, and the reasoning is in
`docs/plans/slice-12-git-object-access-analysis.md` §5. In short: `gix` is a facade over ~30
sub-crates and would have grown the tree by roughly half, including `gix-url` and transport code;
`git2` links `libgit2`, which ships its own HTTP transport. Keeping a network-capable Git
implementation in the tree and asserting by test that it is never used is a weaker guarantee than not
having one. Nerve reads the packfile format itself instead
(`crates/nerve-index/src/gitobj/`), which is a documented, stable file format of the same kind this
codebase already reads by hand.

**Net: 96 → 101 third-party crates; `Cargo.lock` 101 → 106 packages. All five permissive.**

### Notes on version pinning

`tree-sitter-typescript 0.23.2` is the newest release compatible with `tree-sitter 0.25.10`;
`tree-sitter-javascript` and `tree-sitter-python` are at 0.25.0. All three expose grammars as
`LanguageFn` constants (`LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX`, `LANGUAGE`) rather than the older
`language_typescript()` functions, and all three resolve through `tree-sitter-language 0.1.7`.

## Complete transitive list

| Crate | Version | License |
|---|---|---|
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 (we take MIT) |
| `aho-corasick` | 1.1.4 | Unlicense OR MIT |
| `anstream` | 1.0.0 | MIT OR Apache-2.0 |
| `anstyle` | 1.0.14 | MIT OR Apache-2.0 |
| `anstyle-parse` | 1.0.0 | MIT OR Apache-2.0 |
| `anstyle-query` | 1.1.5 | MIT OR Apache-2.0 |
| `anstyle-wincon` | 3.0.11 | MIT OR Apache-2.0 |
| `anyhow` | 1.0.104 | MIT OR Apache-2.0 |
| `arrayref` | 0.3.9 | BSD-2-Clause |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 |
| `ascii` | 1.1.0 | Apache-2.0 OR MIT |
| `bitflags` | 2.13.1 | MIT OR Apache-2.0 |
| `blake3` | 1.8.5 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| `bstr` | 1.13.0 | MIT OR Apache-2.0 |
| `cc` | 1.4.0 | MIT OR Apache-2.0 |
| `cfg-if` | 1.0.4 | MIT OR Apache-2.0 |
| `chunked_transfer` | 1.5.0 | MIT OR Apache-2.0 |
| `clap` | 4.6.4 | MIT OR Apache-2.0 |
| `clap_builder` | 4.6.2 | MIT OR Apache-2.0 |
| `clap_derive` | 4.6.4 | MIT OR Apache-2.0 |
| `clap_lex` | 1.1.0 | MIT OR Apache-2.0 |
| `colorchoice` | 1.0.5 | MIT OR Apache-2.0 |
| `constant_time_eq` | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| `cpufeatures` | 0.3.0 | MIT OR Apache-2.0 |
| `crc32fast` | 1.5.0 | MIT OR Apache-2.0 |
| `crossbeam-deque` | 0.8.7 | MIT OR Apache-2.0 |
| `crossbeam-epoch` | 0.9.20 | MIT OR Apache-2.0 |
| `crossbeam-utils` | 0.8.22 | MIT OR Apache-2.0 |
| `equivalent` | 1.0.2 | Apache-2.0 OR MIT |
| `errno` | 0.3.14 | MIT OR Apache-2.0 |
| `fallible-iterator` | 0.3.0 | MIT/Apache-2.0 |
| `fallible-streaming-iterator` | 0.1.9 | MIT/Apache-2.0 |
| `fastrand` | 2.5.0 | Apache-2.0 OR MIT |
| `find-msvc-tools` | 0.1.9 | MIT OR Apache-2.0 |
| `flate2` | 1.1.9 | MIT OR Apache-2.0 |
| `foldhash` | 0.1.5 | Zlib |
| `getrandom` | 0.4.3 | MIT OR Apache-2.0 |
| `globset` | 0.4.19 | Unlicense OR MIT |
| `hashbrown` | 0.15.5 | MIT OR Apache-2.0 |
| `hashbrown` | 0.17.1 | MIT OR Apache-2.0 |
| `hashlink` | 0.10.0 | MIT OR Apache-2.0 |
| `heck` | 0.5.0 | MIT OR Apache-2.0 |
| `httpdate` | 1.0.3 | MIT OR Apache-2.0 |
| `ignore` | 0.4.31 | Unlicense OR MIT |
| `indexmap` | 2.14.0 | Apache-2.0 OR MIT |
| `is_terminal_polyfill` | 1.70.2 | MIT OR Apache-2.0 |
| `itoa` | 1.0.18 | MIT OR Apache-2.0 |
| `libc` | 0.2.189 | MIT OR Apache-2.0 |
| `libsqlite3-sys` | 0.35.0 | MIT (vendored SQLite is public domain) |
| `linux-raw-sys` | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `log` | 0.4.33 | MIT OR Apache-2.0 |
| `memchr` | 2.8.3 | Unlicense OR MIT |
| `miniz_oxide` | 0.8.9 | MIT OR Zlib OR Apache-2.0 (we take MIT) |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 |
| `once_cell_polyfill` | 1.70.2 | MIT OR Apache-2.0 |
| `pkg-config` | 0.3.33 | MIT OR Apache-2.0 |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 |
| `quote` | 1.0.47 | MIT OR Apache-2.0 |
| `r-efi` | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later (we take MIT) |
| `regex` | 1.13.1 | MIT OR Apache-2.0 |
| `regex-automata` | 0.4.16 | MIT OR Apache-2.0 |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 |
| `rusqlite` | 0.37.0 | MIT |
| `rustix` | 1.1.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `same-file` | 1.0.6 | Unlicense/MIT |
| `serde` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 |
| `serde_spanned` | 1.1.1 | MIT OR Apache-2.0 |
| `shlex` | 2.0.1 | MIT OR Apache-2.0 |
| `signal-hook` | 0.4.4 | MIT OR Apache-2.0 |
| `signal-hook-registry` | 1.4.8 | MIT OR Apache-2.0 |
| `simd-adler32` | 0.3.10 | MIT |
| `smallvec` | 1.15.2 | MIT OR Apache-2.0 |
| `streaming-iterator` | 0.1.9 | MIT OR Apache-2.0 |
| `strsim` | 0.11.1 | MIT |
| `syn` | 3.0.3 | MIT OR Apache-2.0 |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 |
| `thiserror` | 2.0.19 | MIT OR Apache-2.0 |
| `thiserror-impl` | 2.0.19 | MIT OR Apache-2.0 |
| `tiny_http` | 0.12.0 | MIT OR Apache-2.0 |
| `toml` | 0.9.12+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_datetime` | 0.7.5+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_parser` | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| `toml_writer` | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| `tree-sitter` | 0.25.10 | MIT |
| `tree-sitter-javascript` | 0.25.0 | MIT |
| `tree-sitter-language` | 0.1.7 | MIT |
| `tree-sitter-python` | 0.25.0 | MIT |
| `tree-sitter-typescript` | 0.23.2 | MIT |
| `unicode-ident` | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `utf8parse` | 0.2.2 | Apache-2.0 OR MIT |
| `vcpkg` | 0.2.15 | MIT/Apache-2.0 |
| `walkdir` | 2.5.0 | Unlicense/MIT |
| `winapi-util` | 0.1.11 | Unlicense OR MIT |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 |
| `windows-sys` | 0.61.2 | MIT OR Apache-2.0 |
| `winnow` | 0.7.15 | MIT |
| `winnow` | 1.0.4 | MIT |
| `zmij` | 1.0.23 | MIT |

Total: 101 third-party crates (`nerve-core`, `nerve-store`, `nerve-index`, `nerve-server` and
`nerve-cli` are this workspace and are excluded). `Cargo.lock` therefore holds 106 `[[package]]`
entries; the authority for that number is `grep -c '^name = ' Cargo.lock`, not this line.

## npm — `apps/nerve-web`, the visual explorer (Slice 4b)

An npm tree cannot be audited package by package the way a Cargo tree can, so the surface is
held down instead of audited up. The decision and its reasoning are in
`docs/plans/slice-04-visual-explorer.md` P5.

The split below is the important one, and it is **enforced, not merely documented**:
`apps/nerve-web/tools/lint.mjs` fails the build if `package.json` declares any runtime
dependency other than `react` and `react-dom`, or declares one that is never imported.

Regenerate these facts with:

```bash
node -e "const l=require('./apps/nerve-web/package-lock.json');
for (const [p,i] of Object.entries(l.packages)) if (p.startsWith('node_modules/'))
  console.log(p.slice(13), i.version, i.license, i.dev?'build-time':'DISTRIBUTED')"
```

### Distributed — compiled into the `nerve` binary

These three are bundled by Vite into `crates/nerve-server/assets/assets/nerve.js`, which
`include_bytes!` compiles into the executable. They ship to users.

| Package | Version | License | Purpose |
|---|---|---|---|
| `react` | 19.2.0 | MIT | The view layer |
| `react-dom` | 19.2.0 | MIT | DOM rendering |
| `scheduler` | 0.27.0 | MIT | React's cooperative scheduler; a transitive dependency of `react-dom`, not chosen directly |

**Three packages, all MIT.** No UI kit, no chart library, no graph library, no state manager,
no CSS framework, no icon package, no date library, no HTTP client. The neighbourhood graph is
hand-rolled SVG (`apps/nerve-web/src/graph/layout.ts`); the stylesheet is written by hand and
uses system font stacks, because the Content-Security-Policy forbids a remote origin and a
webfont would be another distributed dependency for no legibility gain.

### Build-time only — **not distributed**

67 further packages are reachable from `devDependencies`. **None of them is in the shipped
bytes**: they run on a developer's machine to produce `dist/`, and the `nerve` binary contains
no trace of them. They are recorded for completeness, not because they are distributed.

| Package | Version | License | Purpose |
|---|---|---|---|
| `vite` | 7.3.6 | MIT | Bundler. Configured plugin-free — see below |
| `typescript` | 5.9.3 | Apache-2.0 | Type checking (`tsc --noEmit`); emits nothing |
| `@types/react` | 19.2.2 | MIT | Type declarations |
| `@types/react-dom` | 19.2.2 | MIT | Type declarations |
| `esbuild` + 26 `@esbuild/*` platform binaries | 0.28.1 | MIT | Transform, via Vite |
| `rollup` + 25 `@rollup/rollup-*` platform binaries | 4.62.3 | MIT | Bundling, via Vite |
| `postcss` | 8.5.25 | MIT | CSS pipeline, via Vite |
| `nanoid` | 3.3.16 | MIT | Via postcss |
| `picocolors` | 1.1.1 | ISC | Via postcss/vite |
| `source-map-js` | 1.2.1 | BSD-3-Clause | Via postcss |
| `@types/estree` | 1.0.9 | MIT | Via rollup |
| `csstype` | 3.2.3 | MIT | Via `@types/react` |
| `fdir` · `picomatch` · `tinyglobby` | 6.5.0 · 4.0.5 · 0.2.17 | MIT | File globbing, via Vite |
| `fsevents` | 2.3.3 | MIT | Optional macOS file watcher, via Vite's dev server, which this project never runs |

The 51 `@esbuild/*` and `@rollup/rollup-*` entries are per-platform prebuilt binaries; exactly
one is installed on any given machine (`@esbuild/darwin-arm64` and `@rollup/rollup-darwin-arm64`
here). They are counted individually above because the lockfile lists them individually.

**All 70 packages are permissively licensed**: MIT, ISC, BSD-3-Clause, or Apache-2.0. Nothing is
copyleft.

### What `apps/nerve-web` deliberately does not use

- **`@vitejs/plugin-react`** — it exists for dev-server Fast Refresh, which this project never
  runs because the app is only ever served from the Rust binary. It would pull Babel into a tree
  that then needs licence review. `esbuild`'s automatic JSX transform covers the need.
- **A linter (`eslint` and its plugin tree)** — `tsc --strict` with `noUnusedLocals`,
  `noUncheckedIndexedAccess` and `verbatimModuleSyntax` catches what matters, and the rules that
  are actually load-bearing here are security rules, not style rules. Those are enforced by
  `tools/lint.mjs`, which is 120 lines of Node with no dependencies.
- **A test framework** — `node --test` runs the layout and search tests directly, and Node strips
  the types on import, so no transform and no framework is needed.
- **Any runtime package beyond React** — see the enforced list above.

## Dependencies deliberately absent

Enforced by `crates/nerve-cli/tests/no_network.rs`, which parses `cargo metadata` and fails
if any of these becomes reachable:

`tokio` · `reqwest` · `hyper` · `ureq` · `curl` · `native-tls` · `rustls` · `socket2`

Also absent by design: any async runtime (`tokio`, `async-std`, `smol`,
`futures-executor`), and anything whose name contains `telemetry`, `analytics`, or `sentry`.

`tiny_http` does not weaken that list, and the `no_network` test still passes unmodified.
It is a **listener**, not a client: it accepts connections on a socket this process bound to
`127.0.0.1` and has no code path that originates an outbound connection, resolves a name, or
speaks TLS. CLAUDE.md §2's guarantee is that Nerve makes no network calls and uploads no
source; a loopback socket the user opened deliberately, on their own machine, is the surface
SECURITY.md has always described as arriving "from Slice 4".

Not adopted, with reasons:

- **`rand`** — `nerve init` needs 32 random bytes once. `/dev/urandom` is the operating
  system interface for that, and a whole crate tree for one read is not worth the surface.
- **`globset`** (as a direct dependency) — it arrives transitively via `ignore`, but the
  secret deny-list needs only `*` wildcards, which is a 20-line matcher with its own tests.
- **`chrono` / `time`** — SQLite formats the timestamps it stores; `nerve init` formats one
  RFC 3339 string with `std::time` plus a civil-date conversion.
- **snapshot-testing crates** — golden tests compare plain JSON files that a human can read
  in a diff.
