import { errorMessage } from "./geometry.ts";

export const FOLIO_IDENTIFIER = "app.folio.desktop";

export interface RuntimeMode {
  native: boolean;
  issue: string | null;
}

export async function resolveRuntime({
  hasNativeBridge, identify, developmentPreview, reportPreview,
}: {
  hasNativeBridge: boolean;
  identify(): Promise<string>;
  developmentPreview: boolean;
  reportPreview(message: string): void;
}): Promise<RuntimeMode> {
  const unsupported = "Open the Folio desktop application. This build is not running in Folio's desktop runtime.";
  if (!hasNativeBridge) return { native: false, issue: developmentPreview ? null : unsupported };
  try {
    const identifier = await identify();
    if (typeof identifier !== "string" || !identifier) {
      throw new Error("The native bridge returned an invalid application identifier.");
    }
    if (identifier === FOLIO_IDENTIFIER) return { native: true, issue: null };
    return { native: false, issue: developmentPreview ? null : unsupported };
  } catch (error) {
    if (developmentPreview) {
      reportPreview(`Folio is using its browser preview, without host native APIs. ${errorMessage(error)}`);
      return { native: false, issue: null };
    }
    return {
      native: false,
      issue: `Folio could not identify its desktop runtime. Restart the desktop application. ${errorMessage(error)}`,
    };
  }
}
