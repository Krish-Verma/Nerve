// Aliased named imports, a namespace import, and a type-only import.

import { add as plus, subtract } from './math';
import * as shapes from './shapes';
import type { Shape } from './shapes';

export function describe(shape: Shape): string {
  const area = shape.area();
  const known = Object.keys(shapes).length;
  return `${plus(1, 2)} ${subtract(3, 1)} ${area} ${known}`;
}
