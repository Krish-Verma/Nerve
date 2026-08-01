---
title: The md-docs fixture
generated: false
---

# md-docs

The document-ingestion fixture for Slice 5a. Every construct in the scanner's supported subset
appears here at least once, so that a scanner regression shows up as a changed section count
rather than as a silently missing citation.

Setext level one
================

A `code span` in prose, and a fence that must not contribute structure:

```ts
# not a heading
const heading = '## also not a heading';
```

Setext level two
----------------

An indented code block, which is likewise not structure:

    # not a heading either
    Title
    =====

## Nesting

### Deep

#### Deeper

## Repeated

## Repeated

Two sibling sections with identical text. They are two sections, distinguished by ordinal, not
one section observed twice.
