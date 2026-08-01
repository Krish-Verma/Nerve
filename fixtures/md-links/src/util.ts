export function describe(subject: string): string {
  return `describing ${subject}`;
}

export class Describer {
  private label: string;

  constructor(label: string) {
    this.label = label;
  }

  describe(subject: string): string {
    return `${this.label}: ${subject}`;
  }
}
