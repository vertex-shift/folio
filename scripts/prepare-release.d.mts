export function verifyUpdate(
  bytes: Uint8Array, encodedSignature: string, encodedKey: string, version: string,
): void;

export function prepareRelease(options: {
  artifacts: string;
  output: string;
  version: string;
  tag: string;
  publicKey: string;
  date?: string;
}): Promise<{
  version: string;
  notes: string;
  pub_date: string;
  platforms: Record<string, { signature: string; url: string }>;
}>;
