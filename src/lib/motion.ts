export interface MotionRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export const SELECTION_MOTION = {
  duration: 260,
  easing: "cubic-bezier(0.16, 1, 0.3, 1)",
};

export function selectionTransform(from: MotionRect, to: MotionRect): string | null {
  const values = [from.left, from.top, from.width, from.height, to.left, to.top, to.width, to.height];
  if (!values.every(Number.isFinite) || from.width <= 0 || from.height <= 0 || to.width <= 0 || to.height <= 0) return null;
  if (Math.abs(from.left - to.left) < 0.5 && Math.abs(from.top - to.top) < 0.5
    && Math.abs(from.width - to.width) < 0.5 && Math.abs(from.height - to.height) < 0.5) return null;
  return `translate(${from.left - to.left}px, ${from.top - to.top}px) scale(${from.width / to.width}, ${from.height / to.height})`;
}
