// Unit tests for the only part of the UI that is arithmetic rather than markup.
//
// Run by `npm test`, which is `node --test src`. Node strips the types from the imported module,
// so this needs no test framework, no transform, and no additional dependency.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  circularMean,
  edgePath,
  labelPlacement,
  layout,
  ringLabelPlacement,
} from './layout.ts';

const node = (id, depth, sortKey = id) => ({ id, depth, sortKey });

test('the focus sits at the centre and everything else sits on a ring', () => {
  const result = layout(
    [node('a', 0), node('b', 1), node('c', 1), node('d', 2)],
    [
      { source: 'a', target: 'b' },
      { source: 'a', target: 'c' },
      { source: 'b', target: 'd' },
    ],
    600,
    600,
  );

  const focus = result.byId.get('a');
  assert.equal(focus.x, 300);
  assert.equal(focus.y, 300);
  assert.equal(focus.radius, 0);

  const first = result.byId.get('b');
  const second = result.byId.get('c');
  assert.equal(first.radius, second.radius);
  assert.ok(first.radius > 0);
  assert.ok(result.byId.get('d').radius > first.radius, 'depth 2 sits outside depth 1');
  assert.equal(result.rings.length, 2);
});

test('the layout is a pure function of its input, so a screenshot is reproducible', () => {
  const nodes = [node('a', 0), node('c', 1), node('b', 1), node('d', 2)];
  const edges = [
    { source: 'a', target: 'b' },
    { source: 'a', target: 'c' },
    { source: 'c', target: 'd' },
  ];
  const first = layout(nodes, edges, 640, 480);
  const second = layout(nodes.slice().reverse(), edges.slice().reverse(), 640, 480);
  for (const id of ['a', 'b', 'c', 'd']) {
    assert.equal(first.byId.get(id).x, second.byId.get(id).x, id);
    assert.equal(first.byId.get(id).y, second.byId.get(id).y, id);
  }
});

test('a deeper node is placed near the inner node it hangs off', () => {
  // Two depth-1 nodes on opposite sides, each with one child. Each child must land on its own
  // parent's side of the ring rather than being distributed by name.
  const result = layout(
    [node('a', 0), node('l', 1), node('r', 1), node('zchild', 2), node('achild', 2)],
    [
      { source: 'a', target: 'l' },
      { source: 'a', target: 'r' },
      { source: 'l', target: 'zchild' },
      { source: 'r', target: 'achild' },
    ],
    600,
    600,
  );

  const gap = (one, two) => {
    const delta = Math.abs(one.angle - two.angle) % (Math.PI * 2);
    return Math.min(delta, Math.PI * 2 - delta);
  };
  const l = result.byId.get('l');
  const r = result.byId.get('r');
  assert.ok(
    gap(result.byId.get('zchild'), l) < gap(result.byId.get('zchild'), r),
    'the child of l stays beside l',
  );
  assert.ok(
    gap(result.byId.get('achild'), r) < gap(result.byId.get('achild'), l),
    'the child of r stays beside r',
  );
});

test('an empty neighbourhood still produces a usable frame', () => {
  const result = layout([], [], 400, 300);
  assert.deepEqual(result.nodes, []);
  assert.equal(result.rings.length, 1);
  assert.ok(result.extent >= 40);
});

test('circular mean does not average across the wrap point incorrectly', () => {
  const mean = circularMean([-Math.PI + 0.1, Math.PI - 0.1]);
  assert.ok(Math.abs(Math.abs(mean) - Math.PI) < 1e-9, `expected near ±π, got ${mean}`);
  assert.ok(Math.abs(circularMean([0.4, 0.6]) - 0.5) < 1e-9);
});

test('an edge bows towards the centre rather than crossing it', () => {
  const centre = { x: 100, y: 100 };
  const path = edgePath({ x: 40, y: 100 }, { x: 160, y: 100 }, { x: 100, y: 40 });
  assert.match(path, /^M 40 100 Q /);
  const control = path.split('Q ')[1].split(' ');
  assert.ok(Number(control[1]) < 100, 'the control point is pulled to the centre side');
  assert.equal(edgePath({ x: 1, y: 1 }, { x: 1, y: 1 }, centre), 'M 1 1 L 1 1');
});

test('labels sit outside the ring, anchored away from the centre', () => {
  const result = layout([node('a', 0), node('b', 1), node('c', 1)], [], 600, 600);
  const centreLabel = labelPlacement(result.byId.get('a'));
  assert.equal(centreLabel.anchor, 'middle');
  assert.ok(centreLabel.y > result.byId.get('a').y);

  const right = { id: 'x', depth: 1, sortKey: 'x', angle: 0, radius: 100, x: 400, y: 300 };
  assert.equal(labelPlacement(right).anchor, 'start');
  const left = { ...right, angle: Math.PI, x: 200 };
  assert.equal(labelPlacement(left).anchor, 'end');
});

test('a ring label lands between two nodes, never on one', () => {
  // Six nodes on ring 1: the first is placed at the top, so the top is exactly where a fixed
  // label would collide. The gap midpoints are what this has to find instead.
  const result = layout(
    [
      node('a', 0),
      node('b', 1),
      node('c', 1),
      node('d', 1),
      node('e', 1),
      node('f', 1),
      node('g', 1),
    ],
    [],
    600,
    600,
  );
  const ring = result.rings[0];
  const at = ringLabelPlacement(ring, result.nodes, result.centre);

  for (const placed of result.nodes) {
    if (placed.depth !== 1) continue;
    const distance = Math.hypot(placed.x - at.x, placed.y - at.y);
    assert.ok(distance > 20, `the ring label is ${distance.toFixed(1)}px from ${placed.id}`);
  }

  // And it stays on the ring it is labelling, so it cannot be read as belonging to another.
  const radius = Math.hypot(at.x - result.centre.x, at.y - result.centre.y - 4);
  assert.ok(Math.abs(radius - ring.radius) < 1, `${radius} should be ${ring.radius}`);
});

test('a ring with nothing on it still gets a label', () => {
  const result = layout([node('a', 0), node('b', 1)], [], 600, 600);
  const at = ringLabelPlacement({ depth: 3, radius: 200 }, result.nodes, result.centre);
  assert.equal(at.x, result.centre.x);
  assert.ok(at.y < result.centre.y);
});
