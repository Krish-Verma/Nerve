// Generic arguments are stripped to the head identifier: `Base<number>` extends `Base`.

export class Base<T> {
  value: T | null = null;
}

export class Derived extends Base<number> {}
