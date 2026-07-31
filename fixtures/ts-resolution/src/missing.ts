// AMBIGUOUS: the import specifier names no indexed module, so the call cannot resolve.

import { helper } from './does-not-exist';

export function callThroughMissingImport(): number {
  return helper();
}
