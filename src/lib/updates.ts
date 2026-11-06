export interface UpdateStatus {
  phase: "unavailable" | "idle" | "checking" | "downloading" | "ready" | "installing" | "error";
  version: string | null;
  received: number;
  total: number | null;
  message: string | null;
}

export function updateSummary(status: UpdateStatus): string {
  switch (status.phase) {
    case "unavailable": return status.message ?? "Automatic updates are unavailable for this installation.";
    case "idle": return status.message ?? "Folio will check for updates automatically.";
    case "checking": return "Checking for updates...";
    case "downloading": return `Downloading Folio ${status.version ?? "update"} in the background...`;
    case "ready": return `Folio ${status.version ?? "update"} is ready. It will install when you quit.`;
    case "installing": return "Installing the update. Folio will close when it is finished.";
    case "error": return status.message ?? "The update could not be prepared. Your current app is unchanged.";
  }
}

export function canCheckForUpdates(status: UpdateStatus): boolean {
  return status.phase === "idle" || status.phase === "error";
}

export function downloadProgress(status: UpdateStatus): number | undefined {
  if (!status.total || !Number.isFinite(status.total) || status.total < 0
    || !Number.isFinite(status.received) || status.received < 0) return undefined;
  return Math.min(1, status.received / status.total);
}
