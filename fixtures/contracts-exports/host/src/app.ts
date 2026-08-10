// The importing module C2 reads. Every specifier below is written deliberately and every one of
// them appears in ../ground_truth.json with a verdict — including the two that are not C2
// declarations at all, so that the count of declarations is stated rather than left to be derived.
//
// Nothing here is executed and nothing here needs to type-check: Nerve parses the file and caches
// the specifiers, and the specifiers are the declaration.

import { root } from "pkg-map";
import { sub } from "pkg-map/sub";
import { cond } from "pkg-map/cond";
import { browserOnly } from "pkg-map/only-browser";
import { blocked } from "pkg-map/blocked";
import { deep } from "pkg-map/deep";
import { escaped } from "pkg-map/escape";
import { gone } from "pkg-map/gone";
import { table } from "pkg-map/data";
import { stringRoot } from "pkg-string";
import { stringSub } from "pkg-string/sub";
import { legacy } from "pkg-legacy";
import { legacySub } from "pkg-legacy/sub";
import { nobody } from "pkg-unregistered";
import { aliased } from "pkg-aliased/thing";
import { twin } from "pkg-twin";
import { useState } from "react";
import { helper } from "./local";

export function app(): string {
  return [
    root,
    sub,
    cond,
    browserOnly,
    blocked,
    deep,
    escaped,
    gone,
    table,
    stringRoot,
    stringSub,
    legacy,
    legacySub,
    nobody,
    aliased,
    twin,
    useState,
    helper,
  ].join("");
}
