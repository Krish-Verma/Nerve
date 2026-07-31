// Heritage inside one module, and both `this.m()` failure modes from plan P3.

export interface Shape {
  area(): number;
}

export interface Solid extends Shape {
  volume(): number;
}

export class Rectangle implements Shape {
  constructor(
    private width: number,
    private height: number,
  ) {}

  area(): number {
    return this.width * this.height;
  }

  // POSITIVE: `this` is the instance of a class that itself declares `area`.
  describe(): number {
    return this.area();
  }

  // NEGATIVE: inside a non-arrow `function`, `this` is not the instance.
  viaNestedFunction(): number {
    function helper(): number {
      return this.area();
    }
    return helper();
  }
}

export class Circle implements Shape {
  constructor(readonly radius: number) {}

  area(): number {
    return 3 * this.radius;
  }
}

// NEGATIVE: `area` is declared on the base class, not on Ellipse.
export class Ellipse extends Circle {
  stretch(): number {
    return this.area();
  }
}
