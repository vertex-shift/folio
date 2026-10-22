export interface Match {
  start: number;
  end: number;
}

export interface TextRun {
  str: string;
  hasEOL?: boolean;
}

export function mapText(items: readonly TextRun[]) {
  let text = "";
  const runs = items.map((item, index) => {
    const start = text.length;
    text += item.str;
    const end = text.length;
    if (item.hasEOL) text += "\n";
    return { index, start, end };
  });
  return { text, runs };
}

export function findMatches(text: string, query: string, matchCase = false, limit = 501): Match[] {
  if (!query.trim()) return [];
  const expression = query.trim().split(/\s+/)
    .map((word) => word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("\\s+");
  const regex = new RegExp(expression, matchCase ? "gu" : "giu");
  const result: Match[] = [];
  for (const match of text.matchAll(regex)) {
    result.push({ start: match.index, end: match.index + match[0].length });
    if (result.length >= limit) break;
  }
  return result;
}

export function excerpt(text: string, match: Match) {
  return {
    before: (match.start > 45 ? "..." : "") + text.slice(Math.max(0, match.start - 45), match.start).replace(/\s+/g, " "),
    match: text.slice(match.start, match.end).replace(/\s+/g, " "),
    after: text.slice(match.end, match.end + 60).replace(/\s+/g, " ") + (match.end + 60 < text.length ? "..." : ""),
  };
}
