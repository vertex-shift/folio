export const PAGE_SIZES = {
  letter: { label: "US Letter", width: 612, height: 792 },
  a4: { label: "A4", width: 595.28, height: 841.89 },
} as const;

export function parsePageRange(input: string, count: number): number[] {
  if (!input.trim() || !Number.isSafeInteger(count) || count < 1) {
    throw new Error("Enter page numbers, such as 1-3, 5.");
  }
  const result: number[] = [];
  const used = new Set<number>();
  for (const part of input.split(",")) {
    const match = /^\s*(\d+)\s*(?:-\s*(\d+)\s*)?$/.exec(part);
    if (!match) throw new Error("Use page numbers or ascending ranges, such as 1-3, 5.");
    const first = Number(match[1]);
    const last = match[2] ? Number(match[2]) : first;
    if (!Number.isSafeInteger(first) || !Number.isSafeInteger(last) || first < 1 || last > count || first > last) {
      throw new Error(`Choose pages from 1 to ${count} in ascending ranges.`);
    }
    for (let number = first; number <= last; number++) {
      if (used.has(number)) throw new Error(`Page ${number} is listed more than once.`);
      used.add(number);
      result.push(number - 1);
    }
  }
  return result;
}

export function parseWritingNumber(value: string, minimum: number, maximum: number, label: string): number {
  const result = Number(value);
  if (!value.trim() || !Number.isFinite(result) || result < minimum || result > maximum) {
    throw new Error(`${label} must be between ${minimum} and ${maximum} points.`);
  }
  return result;
}

export function rgbFromHex(value: string): [number, number, number] {
  if (!/^#[\da-f]{6}$/i.test(value)) throw new Error("Choose a valid writing color.");
  return [parseInt(value.slice(1, 3), 16) / 255, parseInt(value.slice(3, 5), 16) / 255, parseInt(value.slice(5, 7), 16) / 255];
}
