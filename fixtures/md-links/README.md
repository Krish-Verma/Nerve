---
title: md-links
---

Before any heading, a link to [the guide](./docs/guide.md). Its source is the **document**, not a
section: a link written before the first heading belongs to no section, and inventing one for it
would put an entity in the graph that this file does not contain.

# md-links

The corpus behind the measured precision of `md-structural`'s link resolution. The ground truth
is `expected.json`; `README-fixture.md` explains what each entry is for.

## Resolving to a file

An inline link to [the app module](./src/app.ts).

A root-relative link to [the utility module](/src/util.ts). Root-relative is resolved against the
repository root, not against this file's directory.

A path whose name contains a space, written the only way CommonMark allows a space to appear in
a destination: [the spaced guide](<./docs/my guide.md>).

## Resolving to a symbol

A line anchor into a top-level function: [describe](./src/util.ts#L2).

A line anchor into a method nested inside a class resolves to the **innermost** symbol covering
that line, which is the method and not the class that holds it:
[Describer.describe](./src/util.ts#L13).

A line range names its first line, so this is the same symbol again, cited from a second place:
[the same method](./src/util.ts#L13-L14).
