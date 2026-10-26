export function createSignatureTrustState() {
  let revision = 0;
  const listeners = new Set<() => void>();
  return {
    accept(next: number): boolean {
      if (!Number.isSafeInteger(next) || next < 0 || next < revision) return false;
      if (next > revision) {
        revision = next;
        for (const listener of listeners) listener();
      }
      return true;
    },
    subscribe(listener: () => void): () => void {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

// Shared across all document tabs, including inactive panel instances.
export const signatureTrustState = createSignatureTrustState();
