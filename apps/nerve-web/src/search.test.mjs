// The other pure part of the UI: what to offer when the index matched nothing.
//
// These assertions encode the store's real matching rule (prefix terms, ANDed). If that rule
// ever changes in `nerve-store`, these tests are the place the interface finds out.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { splitIdentifier, widenings } from './search.ts';

test('an identifier splits the way a reader reads it', () => {
  assert.deepEqual(splitIdentifier('sameNameDifferentScopes'), [
    'same',
    'Name',
    'Different',
    'Scopes',
  ]);
  assert.deepEqual(splitIdentifier('parse_http_response'), ['parse', 'http', 'response']);
  assert.deepEqual(splitIdentifier('parseHTTPResponse'), ['parse', 'HTTP', 'Response']);
  assert.deepEqual(splitIdentifier('src/shapes.ts'), ['src', 'shapes', 'ts']);
  assert.deepEqual(splitIdentifier('area'), ['area']);
  assert.deepEqual(splitIdentifier(''), []);
});

test('a hostile identifier is split as text, not interpreted', () => {
  assert.deepEqual(splitIdentifier('<img src=x onerror=alert(1)>'), [
    'img',
    'src',
    'x',
    'onerror',
    'alert',
    '1',
  ]);
});

test('the first word is offered first, because terms are ANDed prefixes', () => {
  const offered = widenings('callThroughMissingImport');
  assert.equal(offered[0].query, 'call');
  assert.match(offered[0].why, /first word/);
  assert.ok(
    offered.some((item) => item.query === 'Through'),
    'a middle word is offered on its own, since it may name something else',
  );
});

test('nothing is offered for an empty query, and never more than five', () => {
  assert.deepEqual(widenings('   '), []);
  assert.ok(widenings('oneTwoThreeFourFiveSixSevenEight').length <= 5);
});

test('a suggestion is never the query that already failed, and never repeats', () => {
  const offered = widenings('area');
  assert.ok(!offered.some((item) => item.query.toLowerCase() === 'area'));
  const queries = offered.map((item) => item.query);
  assert.equal(new Set(queries).size, queries.length);
});

test('a single long word is offered as a shorter prefix', () => {
  const offered = widenings('normalize');
  assert.ok(offered.some((item) => item.query === 'normali'));
});
