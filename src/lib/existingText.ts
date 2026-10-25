import type { ExistingTextRun, Point } from "./types.ts";

function normalized(value: string): string {
  return value.normalize("NFKC").replace(/\s+/gu, "");
}

export function matchExistingText(runs: readonly ExistingTextRun[], text: string, origin: Point): ExistingTextRun | null {
  const needle = normalized(text);
  if (!needle) return null;
  const exact = runs.filter((run) => run.text !== null && normalized(run.text) === needle);
  const candidates = exact.length ? exact : runs.filter((run) => run.text !== null && normalized(run.text).includes(needle));
  if (!candidates.length || candidates.some((run) => !run.origin)) return null;
  const distances = candidates.map((run) => ({
    run,
    distance: run.origin ? Math.hypot(run.origin[0] - origin[0], run.origin[1] - origin[1]) : Infinity,
  })).sort((a, b) => a.distance - b.distance);
  const [first, second] = distances;
  const overlapping = runs.some((run) => run !== first.run && run.origin && first.run.origin
    && Math.hypot(run.origin[0] - first.run.origin[0], run.origin[1] - first.run.origin[1]) < 0.5);
  return !overlapping && first.distance <= 12 && (!second || second.distance - first.distance > 0.5) ? first.run : null;
}

export function textRevisionKey(info: { id: number; revision: number }): string {
  return `${info.id}:${info.revision}`;
}
