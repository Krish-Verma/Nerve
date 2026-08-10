# contracts-exports

The C2 fixture: one repository (`host`) that declares six local npm dependencies and imports
through them, and six neighbouring checkouts whose own `package.json` files carry every export
shape the rule reads and every shape it declines.

`ground_truth.json` is the hand-written oracle. It was written before the resolver existed, it is
never generated, and `crates/nerve-index/tests/contract_precision.rs` grades C2 against it on its
own table. **That table is never summed with C1's or C3's.**

Registered in the harness: `pkg-map`, `pkg-string`, `pkg-legacy`, `twin-a`, `twin-b`.
Deliberately not registered: `pkg-unregistered` — a real, indexed, adjacent Nerve repository that
`host` both depends on by path and imports by name, and that must still produce zero links.
