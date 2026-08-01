# The guide

A document may reference another document: [back to the readme](../README.md). Documents are in
the set a destination resolves against, unlike the set a module specifier resolves against.

## Reference-style links

A reference-style link to [the app module][app] is carried by its **definition**, which is where
the destination is written and therefore where the citation points.

[app]: ../src/app.ts

## Angle-bracket links

An explicitly relative angle-bracket destination is a link: <../src/util.ts>.

A root-relative one is deliberately not, because `</div>` is the closing half of every HTML tag
pair and the two are indistinguishable without knowing whether `div` names a directory.
