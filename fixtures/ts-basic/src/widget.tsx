// A TSX component with JSX, exercising the TSX grammar.

import type { Shape } from './shapes';

export interface WidgetProps {
  label: string;
  shape: Shape;
}

export function Widget({ label, shape }: WidgetProps) {
  return (
    <div className="widget">
      {label}: {shape.area()}
    </div>
  );
}

export const Badge = ({ count }: { count: number }) => <span>{count}</span>;
