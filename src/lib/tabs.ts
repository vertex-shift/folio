export function adjacentTab(ids: readonly number[], activeId: number | null, direction: -1 | 1): number | null {
  if (!ids.length) return null;
  const current = activeId === null ? -1 : ids.indexOf(activeId);
  if (current < 0) return direction === 1 ? ids[0] : ids[ids.length - 1];
  return ids[(current + direction + ids.length) % ids.length];
}

export function unsavedTabs<T extends { id: number; dirty: boolean }>(documents: readonly T[], pending: ReadonlySet<number>): T[] {
  return documents.filter((document) => document.dirty || pending.has(document.id));
}
