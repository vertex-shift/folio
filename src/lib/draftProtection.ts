type DraftWriter = (documentId: number, pending: boolean) => Promise<void>;

interface DraftUpdate {
  documentId: number;
  pending: boolean;
  request: Promise<void>;
}

export function createDraftSynchronizer(write: DraftWriter) {
  let latest: DraftUpdate | null = null;
  return (documentId: number, pending: boolean): Promise<void> => {
    if (latest?.documentId === documentId && latest.pending === pending) return latest.request;
    const previous = latest?.request;
    const send = () => write(documentId, pending);
    // Serialize updates while allowing a newer projection to retry a failed write.
    const request: Promise<void> = (previous ? previous.then(send, send) : Promise.resolve().then(send))
      .catch((error: unknown) => {
        if (latest?.request === request) latest = null;
        throw error;
      });
    latest = { documentId, pending, request };
    return request;
  };
}
