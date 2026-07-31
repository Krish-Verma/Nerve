// An interface, and two classes implementing it. Structure only: Slice 1 emits no
// IMPLEMENTS edge, so this fixture pins that absence.

export interface Shape {
  area(): number;
}

export class Rectangle implements Shape {
  constructor(
    private width: number,
    private height: number,
  ) {}

  area(): number {
    return this.width * this.height;
  }

  static unit(): Rectangle {
    return new Rectangle(1, 1);
  }
}

export class Circle implements Shape {
  constructor(readonly radius: number) {}

  area(): number {
    return Math.PI * this.radius * this.radius;
  }

  get diameter(): number {
    return this.radius * 2;
  }
}
