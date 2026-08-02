import { add } from "./math";

export class Rectangle {
  constructor(
    private width: number,
    private height: number,
  ) {}

  area(): number {
    return this.width * this.height;
  }

  perimeter(): number {
    return add(this.width + this.height, this.width + this.height);
  }
}

export interface Shape {
  area(): number;
}
