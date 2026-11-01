export interface InstallerDownload {
  href: string;
  label: string;
  detail: string;
}

export const pageUrl: string;
export const downloads: Readonly<{
  macos: InstallerDownload;
  windows: InstallerDownload;
  linux: InstallerDownload;
}>;
export function platformDownload(platform: string): InstallerDownload | null;
