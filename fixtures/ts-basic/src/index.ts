// A barrel file: named re-exports and a star re-export. Re-exported entities keep the
// identity of the module that defines them (ADR-0002).

export { add as plus, scale } from './math';
export * from './shapes';
