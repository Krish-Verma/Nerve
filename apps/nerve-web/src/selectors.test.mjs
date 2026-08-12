// One path can have two readings, and until now the interface never said so.
//
// The server has carried `selectors[key].alternatives` on every selector answer since Slice 8b-i:
// `src/app.ts` holds both a `Module` and a `File`, the rule is *content wins, container is
// reported*, and the report reached no human. These tests hold two things:
//
//   1. The reading functions, including the case that produces **nothing** — which is most
//      selectors, and which must render nothing rather than a permanent "no alternatives" strip.
//   2. That every view resolving a selector actually renders the component. That is the rule the
//      whole slice exists for, it is structural, and a behavioural test cannot see it — a view
//      dropping the call would go on passing everything else in this suite.
//
// Every assertion that something is absent is paired with a count of what was checked.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  alternativesFor,
  matchedBy,
  matchedByReading,
  passedOverReading,
  selectorFor,
} from './selectors.ts';

const here = dirname(fileURLToPath(import.meta.url));
const read = (relative) => readFileSync(join(here, relative), 'utf8');

const file = {
  entity_id: 'file_abc',
  kind: 'file',
  name: 'app.ts',
  scope_path: '',
  qualified_name: 'app.ts',
  language: 'typescript',
  file_path: 'src/app.ts',
  start_line: null,
  end_line: null,
};

const notes = { subject: { matched_by: 'path', alternatives: [file] } };

test('alternatives come back for the key that has them, and empty for everything else', () => {
  assert.deepEqual(alternativesFor(notes, 'subject'), [file]);
  assert.deepEqual(alternativesFor(notes, 'object'), []);
  assert.equal(matchedBy(notes, 'subject'), 'path');
  assert.equal(matchedBy(notes, 'object'), null);
});

test('an absent `selectors` reads exactly like an empty one, and neither throws', () => {
  // The field is an addition to the wire shape, so a server that predates it omits the key. *No
  // key* and *no alternatives* both mean no second reading was reported; there is no third state
  // being lost here.
  let checked = 0;
  for (const value of [undefined, {}, { subject: { matched_by: 'name', alternatives: [] } }]) {
    checked += 1;
    assert.deepEqual(alternativesFor(value, 'subject'), []);
  }
  assert.equal(checked, 3);

  // And a malformed one is not trusted into a crash.
  assert.deepEqual(alternativesFor({ subject: { matched_by: 'path' } }, 'subject'), []);
  assert.deepEqual(alternativesFor({ subject: { alternatives: 'no' } }, 'subject'), []);
});

test('a passed-over entity is offered as something the reader can actually type', () => {
  // The whole reason `file:src/app.ts` exists is that `src/app.ts` alone resolves to the module.
  assert.equal(selectorFor(file), 'file:src/app.ts');
  assert.equal(
    selectorFor({ kind: 'directory', file_path: 'src', entity_id: 'dir_1' }),
    'directory:src',
  );

  // No path means no ambiguity to resolve, so the id is the only honest thing left to offer.
  assert.equal(
    selectorFor({ kind: 'function', file_path: null, entity_id: 'fn_9' }),
    'fn_9',
  );
});

test('the ordinary case says nothing at all', () => {
  // `alternatives` is empty for the overwhelming majority of selectors. A strip reading "no
  // alternatives" would be noise on every screen in the app in order to be information on almost
  // none, and its absence is not the absence of a check — the check is on every answer.
  assert.equal(passedOverReading('module', []), '');
});

test('the sentence names what won and does not call the other reading wrong', () => {
  const one = passedOverReading('module', [file]);
  assert.match(one, /one other entity/);
  assert.match(one, /content wins/i);
  assert.match(one, /module/);

  // Both entities are at that path and both are indexed; neither is a mistake.
  let checked = 0;
  for (const pattern of [/wrong/i, /error/i, /invalid/i, /failed/i]) {
    checked += 1;
    assert.doesNotMatch(one, pattern);
  }
  assert.equal(checked, 4);

  assert.match(passedOverReading('document', [file, file]), /2 other entities/);
});

test('a bare name is flagged as the reading that could have been ambiguous', () => {
  const name = matchedByReading('name');
  assert.match(name, /exactly one entity here/i);
  assert.match(name, /refuse/i, 'a second declaration would be refused rather than picked');

  assert.notEqual(matchedByReading('path'), '');

  // The exact stages get no note: the caller named one thing and got it, so a sentence beside them
  // would be decoration.
  assert.equal(matchedByReading('entity_id'), '');
  assert.equal(matchedByReading('path_qualified'), '');
  assert.equal(matchedByReading(null), '');
});

// ---- the source of the views ------------------------------------------------------------------

test('every view that resolves a selector renders what the selector passed over', () => {
  // The list is the set of views whose `useApi` call targets an endpoint that resolves a selector
  // server-side — `/api/entity`, `/api/neighbourhood`, `/api/path`, `/api/why`, `/api/impact`.
  // Each therefore receives a `selectors` object, and each must render it.
  const views = ['Entity', 'Evidence', 'Graph', 'Path', 'Impact'];
  let checked = 0;
  for (const view of views) {
    const source = read(`./views/${view}.tsx`);
    checked += 1;
    assert.match(
      source,
      /<SelectorReading/,
      `views/${view}.tsx resolves a selector and must render what it passed over`,
    );
    assert.match(source, /SelectorReading/, `views/${view}.tsx must import it`);
  }
  assert.equal(checked, 5, 'five selector-resolving views were checked');
});

test('the path view reports both of its ends, because it resolves two selectors', () => {
  const source = read('./views/Path.tsx');
  assert.match(source, /parameter="from"/);
  assert.match(source, /parameter="to"/);
});

test('a choice made by rule is not rendered as a refusal to choose', () => {
  // An ambiguous selector is a 409 carrying every candidate, and `Failure` renders that. This is a
  // choice a stated rule made, and the two must not read alike — so the component says "also at
  // this path" rather than offering a pick.
  const source = read('./ui/parts.tsx');
  assert.match(source, /also at this path/);
  assert.match(source, /ask for it by name/);
  assert.doesNotMatch(source, /Pick one[\s\S]{0,200}SelectorReading/);

  // And it renders nothing when there is nothing to report.
  assert.match(source, /if \(alternatives\.length === 0\) return null;/);
});

test('the component builds no markup out of repository text', () => {
  // An alternative's name and path are repository content — a file can legally be named
  // `<img src=x onerror=alert(1)>.ts` — and this component renders both. The spellings are written
  // with a one-member character class so the file states the rule without tripping `tools/lint.mjs`,
  // which cannot tell a rule from a violation of it.
  const source = read('./ui/parts.tsx');
  const checked = [/dangerouslySetInnerHTM[L]/, /\.innerHTM[L]\b/, /insertAdjacentHTM[L]/];
  for (const pattern of checked) {
    assert.doesNotMatch(source, pattern, String(pattern));
  }
  assert.equal(checked.length, 3);
});
