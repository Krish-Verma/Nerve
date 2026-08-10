# contracts-fuzzy — the pair that must produce nothing

Two checkouts, `left` and `right`, built as adjacent sibling directories. They share:

- the same npm package name (`shared-name` in both `package.json` files),
- the same Python distribution name (`shared-name` in both `pyproject.toml` files),
- a registry entry: `right` is explicitly registered as `left`'s neighbour.

They do **not** share a declaration. Neither manifest names the other by path, by `file:`, by
`workspace:`, by a direct reference or by a path table.

The expected answer is **zero contract links**, for both C1 and C3. §9.7 of
`docs/plans/slice-13-cross-repository-contracts.md` requires fuzzy linking to be asserted absent
rather than assumed absent, and this fixture is what that assertion is made over: same name, same
parent directory, registered neighbour, no declaration — and nothing.

The registration is what stops the test being vacuous. A scan that produced zero links because it
had no neighbour to link to would prove nothing about name matching.
