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

All 95 transitive dependencies are permissively licensed. Nothing is copyleft-encumbered for
distribution purposes:

- `r-efi 6.0.0` offers `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; we take MIT. It is a UEFI
  target dependency of `getrandom` and is not linked on Linux, macOS or Windows.
- `unicode-ident 1.0.24` is `(MIT OR Apache-2.0) AND Unicode-3.0`. Unicode-3.0 is a
  permissive data licence covering the Unicode character tables.
- `foldhash 0.1.5` is Zlib; `arrayref 0.3.9` is BSD-2-Clause. Both are permissive.
- `blake3` and `constant_time_eq` offer CC0-1.0 among their options.

`libsqlite3-sys` vendors SQLite itself, which is **public domain**.

## Direct dependencies

| Crate | Version | License | Used by | Purpose |
|---|---|---|---|---|
| `anyhow` | 1.0.104 | MIT OR Apache-2.0 | nerve-cli | Error context at the binary boundary |
| `blake3` | 1.8.5 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | nerve-core, nerve-index | Entity/occurrence/assertion ids, content hashes, state merkle |
| `clap` | 4.6.4 | MIT OR Apache-2.0 | nerve-cli | Command-line parsing (derive feature) |
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

### Notes on version pinning

`tree-sitter-typescript 0.23.2` is the newest release compatible with `tree-sitter 0.25.10`;
`tree-sitter-javascript` is at 0.25.0. Both expose grammars as `LanguageFn` constants
(`LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX`, `LANGUAGE`) rather than the older
`language_typescript()` functions, and both resolve through `tree-sitter-language 0.1.7`.

## Complete transitive list

| Crate | Version | License |
|---|---|---|
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
| `crossbeam-deque` | 0.8.7 | MIT OR Apache-2.0 |
| `crossbeam-epoch` | 0.9.20 | MIT OR Apache-2.0 |
| `crossbeam-utils` | 0.8.22 | MIT OR Apache-2.0 |
| `equivalent` | 1.0.2 | Apache-2.0 OR MIT |
| `errno` | 0.3.14 | MIT OR Apache-2.0 |
| `fallible-iterator` | 0.3.0 | MIT/Apache-2.0 |
| `fallible-streaming-iterator` | 0.1.9 | MIT/Apache-2.0 |
| `fastrand` | 2.5.0 | Apache-2.0 OR MIT |
| `find-msvc-tools` | 0.1.9 | MIT OR Apache-2.0 |
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

Total: 95 third-party crates (`nerve-core`, `nerve-store`, `nerve-index`, `nerve-server` and
`nerve-cli` are this workspace and are excluded).

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
