// Both imports must become Unresolved entities with real IMPORTS assertions.
// Neither is silently dropped (ADR-0003).

import './does-not-exist';
import 'some-external-pkg';

export const marker = (): string => 'unresolved fixture';
