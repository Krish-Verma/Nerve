// Reachable only through the `"./*"` wildcard key, which Nerve declines to expand. The file is
// real, which is what makes `pkg-map/deep` a published false negative rather than an absence.
export const deep = "pkg-map deep";
