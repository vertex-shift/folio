import catalog from "../../src-tauri/resources/ocr/catalog.json" with { type: "json" };

export const OCR_LANGUAGES = catalog.languages;
const LANGUAGE_IDS = ["eng", "fra", "spa", "deu", "ita", "por", "nld", "ell", "rus",
  "ara", "hin", "chi_sim", "chi_tra", "jpn", "kor"] as const;
export type OcrLanguage = typeof LANGUAGE_IDS[number];
export const MAX_OCR_LANGUAGES = catalog.maxLanguages;

export function selectedOcrLanguages(value: readonly string[]): OcrLanguage[] {
  if (!value.length || value.length > MAX_OCR_LANGUAGES || new Set(value).size !== value.length) {
    throw new Error("Choose one to three different bundled OCR languages.");
  }
  const selected = value.map((id) => {
    const language = LANGUAGE_IDS.find((item) => item === id);
    if (!language || !OCR_LANGUAGES.some((item) => item.id === language)) throw new Error(`Unsupported OCR language: ${id}. Choose a language in the bundled catalog; no English substitution is made.`);
    return language;
  });
  const bytes = selected.reduce((total, id) => total + OCR_LANGUAGES.find((item) => item.id === id)!.bytes, 0);
  if (bytes > catalog.maxModelBytes) throw new Error("The selected OCR models exceed the 20 MiB job limit.");
  return selected;
}

export function ocrLanguageNames(languages: readonly OcrLanguage[]): string {
  return languages.map((id) => OCR_LANGUAGES.find((item) => item.id === id)?.name ?? id).join(", ");
}

export function ocrLanguageResources(languages: readonly OcrLanguage[]) {
  const selected = selectedOcrLanguages(languages);
  return selected.map((id) => {
    const language = OCR_LANGUAGES.find((item) => item.id === id)!;
    return { id, name: `${id}.traineddata`, bytes: language.bytes, sha256: language.sha256 };
  });
}

export { catalog as OCR_CATALOG };
