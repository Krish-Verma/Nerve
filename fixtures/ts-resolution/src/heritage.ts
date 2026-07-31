// Heritage across a module boundary: both clauses resolve through the import.

import { Circle, Shape } from './shapes';

export class Bubble extends Circle implements Shape {
  area(): number {
    return 1;
  }
}
