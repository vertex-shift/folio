type DraftWriter = (documentId: number, pending: boolean) => Promise<void>;

export interface DocumentDraftProjection {
  writing: boolean;
  formFilling: boolean;
  formDesign: boolean;
  pageAuthoring: boolean;
  imageEditing: boolean;
  redaction: boolean;
}

export function hasDocumentDrafts(projection: DocumentDraftProjection): boolean {
  return Object.values(projection).some(Boolean);
}

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
