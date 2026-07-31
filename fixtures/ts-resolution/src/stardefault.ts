// NEGATIVE: `fallback` cannot reach withdefault.ts's default through a star re-export.
// POSITIVE: `named` can.

import fallback from './starexport';
import { named } from './starexport';

export function useDefault(): number {
  return fallback();
}

export function useNamed(): number {
  return named();
}
