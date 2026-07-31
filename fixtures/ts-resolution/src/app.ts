// Every positive import form, plus the one receiver that needs a type Nerve does not have.

import { plus } from './index';
import { add as renamed } from './math';
import defaultAdd from './math';
import * as math from './math';
import { Rectangle, Shape } from './shapes';

export function viaBarrel(): number {
  return plus(1, 2);
}

export function viaAlias(): number {
  return renamed(1, 2);
}

export function viaDefault(): number {
  return defaultAdd(1, 2);
}

export function viaNamespace(): number {
  return math.scale(2, 3);
}

export function viaNew(): Rectangle {
  return new Rectangle(1, 2);
}

// AMBIGUOUS: resolving `shape.area` needs the type of `shape`. Nerve records the gap.
export function typedParameter(shape: Shape): number {
  return shape.area();
}

export const alias = renamed;
