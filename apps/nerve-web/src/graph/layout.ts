/**
 * Radial layout for a bounded neighbourhood.
 *
 * There is no physics here and there will not be. A force simulation answers "what shape does
 * this data settle into", which is a question nobody has; this view answers "what is one hop away
 * from the thing I am looking at, and what is two hops away", so **distance from the centre is
 * depth** and nothing else. That makes the picture deterministic — the same neighbourhood always
 * draws identically, so a screenshot means something — and it makes the bound legible: the outer
 * ring is the edge of what was fetched, not the edge of the repository.
 *
 * Every function in this file is pure, which is why it is the one part of the UI with unit tests.
 */

export interface LayoutNode {
  id: string;
  depth: number;
  /** Stable tiebreaker. Ordering by it keeps sibling nodes grouped and the layout reproducible. */
  sortKey: string;
}

export interface LayoutEdge {
  source: string;
  target: string;
}

export interface PlacedNode extends LayoutNode {
  x: number;
  y: number;
  angle: number;
  radius: number;
}

export interface Ring {
  depth: number;
  radius: number;
}

export interface Layout {
  centre: { x: number; y: number };
  extent: number;
  nodes: PlacedNode[];
  rings: Ring[];
  byId: Map<string, PlacedNode>;
}

/**
 * Ring radii as a fraction of the available half-extent, indexed by how many rings there are.
 *
 * Spacing is not uniform: the first ring is pulled in so the focus reads as the centre of
 * attention rather than one node among many, and the outermost ring is pushed close to the edge
 * so the boundary of the query is visibly the boundary of the picture.
 */
const RING_FRACTIONS: Record<number, number[]> = {
  1: [0.7],
  2: [0.42, 0.84],
  3: [0.32, 0.6, 0.9],
  4: [0.26, 0.48, 0.7, 0.92],
};

function ringRadii(maxDepth: number, extent: number): number[] {
  const fractions = RING_FRACTIONS[Math.min(Math.max(maxDepth, 1), 4)] ?? RING_FRACTIONS[4]!;
  return fractions.slice(0, Math.max(maxDepth, 1)).map((fraction) => fraction * extent);
}

/** Mean of angles on the circle. A plain arithmetic mean would put 350° and 10° at 180°. */
export function circularMean(angles: number[]): number {
  if (angles.length === 0) return 0;
  let sin = 0;
  let cos = 0;
  for (const angle of angles) {
    sin += Math.sin(angle);
    cos += Math.cos(angle);
  }
  if (Math.abs(sin) < 1e-12 && Math.abs(cos) < 1e-12) return angles[0]!;
  return Math.atan2(sin / angles.length, cos / angles.length);
}

/** Shortest signed distance between two angles, in radians. */
function angleGap(one: number, two: number): number {
  const delta = Math.abs(one - two) % (Math.PI * 2);
  return Math.min(delta, Math.PI * 2 - delta);
}

/**
 * The rotation of an evenly spaced ring that best matches a set of preferred angles.
 *
 * Candidates are the rotations that land some node exactly on its preference; the best of those
 * by squared angular error wins. With `n` nodes this is `n²` comparisons on at most a few hundred
 * nodes, which is nothing, and it is deterministic.
 */
function bestRotation(anchors: (number | undefined)[], step: number): number {
  const candidates: number[] = [];
  anchors.forEach((angle, index) => {
    if (angle !== undefined) candidates.push(angle - index * step);
  });
  if (candidates.length === 0) return -Math.PI / 2;

  let best = candidates[0]!;
  let bestCost = Number.POSITIVE_INFINITY;
  for (const offset of candidates) {
    let cost = 0;
    anchors.forEach((angle, index) => {
      if (angle === undefined) return;
      const error = angleGap(offset + index * step, angle);
      cost += error * error;
    });
    if (cost < bestCost) {
      bestCost = cost;
      best = offset;
    }
  }
  return best;
}

/**
 * Place a neighbourhood on concentric rings.
 *
 * Ring 1 is ordered by `sortKey`. Each deeper ring is ordered by the mean angle of the nodes it
 * connects to on the ring inside it, so a node's children sit near it instead of scattering —
 * the cheapest crossing reduction available, and a deterministic one.
 */
export function layout(
  nodes: LayoutNode[],
  edges: LayoutEdge[],
  width: number,
  height: number,
  margin = 56,
): Layout {
  const centre = { x: width / 2, y: height / 2 };
  const extent = Math.max(40, Math.min(width, height) / 2 - margin);

  const byDepth = new Map<number, LayoutNode[]>();
  let maxDepth = 0;
  for (const node of nodes) {
    const depth = Math.max(0, Math.trunc(node.depth));
    maxDepth = Math.max(maxDepth, depth);
    const bucket = byDepth.get(depth);
    if (bucket) bucket.push(node);
    else byDepth.set(depth, [node]);
  }

  const neighbours = new Map<string, string[]>();
  const link = (from: string, to: string) => {
    const existing = neighbours.get(from);
    if (existing) existing.push(to);
    else neighbours.set(from, [to]);
  };
  for (const edge of edges) {
    if (edge.source === edge.target) continue;
    link(edge.source, edge.target);
    link(edge.target, edge.source);
  }

  const radii = ringRadii(maxDepth, extent);
  const placed: PlacedNode[] = [];
  const byId = new Map<string, PlacedNode>();

  for (const node of byDepth.get(0) ?? []) {
    const spot: PlacedNode = { ...node, x: centre.x, y: centre.y, angle: 0, radius: 0 };
    placed.push(spot);
    byId.set(node.id, spot);
  }

  for (let depth = 1; depth <= maxDepth; depth += 1) {
    const ring = (byDepth.get(depth) ?? []).slice();
    if (ring.length === 0) continue;
    const radius = radii[depth - 1] ?? extent;

    const anchor = new Map<string, number>();
    if (depth > 1) {
      for (const node of ring) {
        const inner = (neighbours.get(node.id) ?? [])
          .map((id) => byId.get(id))
          .filter((spot): spot is PlacedNode => spot !== undefined && spot.depth === depth - 1)
          .map((spot) => spot.angle);
        if (inner.length > 0) anchor.set(node.id, circularMean(inner));
      }
    }

    ring.sort((a, b) => {
      const left = anchor.get(a.id);
      const right = anchor.get(b.id);
      if (left !== undefined && right !== undefined && left !== right) return left - right;
      if (left !== undefined && right === undefined) return -1;
      if (left === undefined && right !== undefined) return 1;
      return a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0;
    });

    // Spacing stays even — an evenly spaced ring is the readable one — but the whole ring is
    // rotated to whichever alignment puts nodes closest to the inner nodes they hang off. That
    // buys the "children sit beside their parent" property without giving up even spacing.
    const step = (Math.PI * 2) / ring.length;
    const offset =
      depth === 1
        ? -Math.PI / 2
        : bestRotation(
            ring.map((node) => anchor.get(node.id)),
            step,
          );
    ring.forEach((node, index) => {
      const angle = offset + index * step;
      const spot: PlacedNode = {
        ...node,
        angle,
        radius,
        x: centre.x + Math.cos(angle) * radius,
        y: centre.y + Math.sin(angle) * radius,
      };
      placed.push(spot);
      byId.set(node.id, spot);
    });
  }

  return {
    centre,
    extent,
    nodes: placed,
    rings: radii.map((radius, index) => ({ depth: index + 1, radius })),
    byId,
  };
}

/**
 * An edge as a quadratic curve bowed towards the centre.
 *
 * Straight lines through a radial layout all pass near the middle and pile up on the focus node.
 * Bowing every chord inwards by a fixed fraction of its own length separates them without moving
 * a single node, and keeps the focus node's own edges readable as spokes.
 */
export function edgePath(
  from: { x: number; y: number },
  to: { x: number; y: number },
  centre: { x: number; y: number },
  bow = 0.13,
): string {
  const midX = (from.x + to.x) / 2;
  const midY = (from.y + to.y) / 2;
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.hypot(dx, dy);
  if (length < 0.5) return `M ${from.x} ${from.y} L ${to.x} ${to.y}`;

  let perpX = -dy / length;
  let perpY = dx / length;
  const towardsCentre = (centre.x - midX) * perpX + (centre.y - midY) * perpY;
  if (towardsCentre < 0) {
    perpX = -perpX;
    perpY = -perpY;
  }
  const offset = length * bow;
  const controlX = midX + perpX * offset;
  const controlY = midY + perpY * offset;
  return `M ${from.x} ${from.y} Q ${controlX} ${controlY} ${to.x} ${to.y}`;
}

/**
 * Where a ring's own label goes: in the gap between two adjacent nodes on that ring.
 *
 * Putting it at a fixed compass point collides with whichever node happens to be there, which on
 * an evenly spaced ring is often the top one. Nodes on a ring are evenly spaced, so the midpoints
 * between them are known exactly — this picks the midpoint nearest the top of the circle, which is
 * both collision-free by construction and deterministic, so the picture stays reproducible.
 */
export function ringLabelPlacement(
  ring: Ring,
  nodes: PlacedNode[],
  centre: { x: number; y: number },
): { x: number; y: number } {
  const angles = nodes
    .filter((node) => node.depth === ring.depth)
    .map((node) => node.angle)
    .sort((a, b) => a - b);

  // With nothing on the ring there is nothing to avoid, so the top is free.
  if (angles.length === 0) {
    return { x: centre.x, y: centre.y - ring.radius - 7 };
  }

  const top = -Math.PI / 2;
  let best = angles[0]! + Math.PI;
  let bestGap = Number.POSITIVE_INFINITY;
  for (let index = 0; index < angles.length; index += 1) {
    const here = angles[index]!;
    const next = index + 1 < angles.length ? angles[index + 1]! : angles[0]! + Math.PI * 2;
    const middle = (here + next) / 2;
    const distance = angleGap(middle, top);
    if (distance < bestGap) {
      bestGap = distance;
      best = middle;
    }
  }

  return {
    x: centre.x + Math.cos(best) * ring.radius,
    y: centre.y + Math.sin(best) * ring.radius + 4,
  };
}

/** Where a node's label goes: outside the ring, on the side that points away from the centre. */
export function labelPlacement(node: PlacedNode): {
  x: number;
  y: number;
  anchor: 'start' | 'middle' | 'end';
} {
  if (node.radius === 0) return { x: node.x, y: node.y + 30, anchor: 'middle' };
  const cos = Math.cos(node.angle);
  const gap = 13;
  if (Math.abs(cos) < 0.34) {
    return {
      x: node.x,
      y: node.y + (Math.sin(node.angle) < 0 ? -gap - 3 : gap + 9),
      anchor: 'middle',
    };
  }
  return {
    x: node.x + (cos > 0 ? gap : -gap),
    y: node.y + 4,
    anchor: cos > 0 ? 'start' : 'end',
  };
}
