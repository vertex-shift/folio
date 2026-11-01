export function createSignatureTrustState() {
  let revision = 0;
  const listeners = new Set<(documentId?: number) => void>();
  const lifetimes = new Map<number, number>();
  const epochs = new Map<number, number>();
  return {
    accept(next: number): boolean {
      if (!Number.isSafeInteger(next) || next < 0 || next < revision) return false;
      if (next > revision) {
        revision = next;
        for (const listener of listeners) listener();
      }
      return true;
    },
    captureDocument(documentId: number): number {
      return lifetimes.get(documentId) ?? 0;
    },
    invalidateDocument(documentId: number): void {
      lifetimes.set(documentId, (lifetimes.get(documentId) ?? 0) + 1);
      for (const listener of listeners) listener(documentId);
    },
    documentCurrent(documentId: number, lifetime: number): boolean {
      return lifetime === (lifetimes.get(documentId) ?? 0);
    },
    acceptDocumentEpoch(documentId: number, epoch: number, lifetime: number): boolean {
      if (!Number.isSafeInteger(epoch) || epoch < 0 || lifetime !== (lifetimes.get(documentId) ?? 0)
          || epoch < (epochs.get(documentId) ?? 0)) return false;
      epochs.set(documentId, epoch);
      return true;
    },
    currentRevision(): number { return revision; },
    subscribe(listener: (documentId?: number) => void): () => void {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

// Public evidence/anchors are shared; source/draft lifetimes remain document-scoped.
export const signatureTrustState = createSignatureTrustState();
