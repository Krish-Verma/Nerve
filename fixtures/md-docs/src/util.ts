export function describe(subject: string): string {
  return `describing ${subject}`;
}

export class Describer {
  describe(subject: string): string {
    return describe(subject);
  }
}
