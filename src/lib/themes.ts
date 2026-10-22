import { errorMessage } from "./geometry.ts";

export const THEME_STORAGE_KEY = "folio.appearance";

export const THEME_OPTIONS = [
  { id: "system", label: "System", description: "Follow your device's appearance" },
  { id: "light", label: "Light", description: "Clear paper and cool slate" },
  { id: "dark", label: "Dark", description: "Quiet surfaces for low light" },
  { id: "sepia", label: "Sepia", description: "Warm surfaces and softer contrast" },
  { id: "contrast", label: "High Contrast", description: "Deep blacks and distinct controls" },
] as const;

export type ThemePreference = typeof THEME_OPTIONS[number]["id"];
export type ResolvedTheme = Exclude<ThemePreference, "system">;

export interface ThemePreferenceResult {
  preference: ThemePreference;
  issue?: string;
}

export function isThemePreference(value: unknown): value is ThemePreference {
  return typeof value === "string" && THEME_OPTIONS.some((option) => option.id === value);
}

export function parseThemePreference(value: unknown): ThemePreferenceResult {
  if (value === null) return { preference: "system" };
  if (isThemePreference(value)) return { preference: value };
  return {
    preference: "system",
    issue: "The saved appearance preference was not recognized. Folio is following the system theme.",
  };
}

export function readThemePreference(read: () => string | null): ThemePreferenceResult {
  try {
    return parseThemePreference(read());
  } catch (error) {
    return {
      preference: "system",
      issue: `The saved appearance preference could not be read. Using the system theme. ${errorMessage(error)}`,
    };
  }
}

export function storeThemePreference(
  preference: ThemePreference,
  write: (key: string, value: string) => void,
): string | null {
  try {
    write(THEME_STORAGE_KEY, preference);
    return null;
  } catch (error) {
    return `Appearance changed for this session, but the preference could not be saved. ${errorMessage(error)}`;
  }
}

export function resolveTheme(preference: ThemePreference, systemDark: boolean): ResolvedTheme {
  return preference === "system" ? systemDark ? "dark" : "light" : preference;
}

export function nativeTheme(preference: ThemePreference): "light" | "dark" | null {
  if (preference === "system") return null;
  return preference === "dark" || preference === "contrast" ? "dark" : "light";
}
