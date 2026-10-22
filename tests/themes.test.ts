import test from "node:test";
import assert from "node:assert/strict";
import {
  THEME_OPTIONS, THEME_STORAGE_KEY, isThemePreference, nativeTheme,
  parseThemePreference, readThemePreference, resolveTheme, storeThemePreference,
} from "../src/lib/themes.ts";

test("the appearance picker has unique system, light, dark, sepia, and contrast choices", () => {
  assert.deepEqual(THEME_OPTIONS.map((option) => option.id), ["system", "light", "dark", "sepia", "contrast"]);
  for (const option of THEME_OPTIONS) {
    assert.equal(isThemePreference(option.id), true);
    assert.deepEqual(parseThemePreference(option.id), { preference: option.id });
    assert.ok(option.label && option.description);
  }
});

test("an absent preference follows the system; invalid saved values are reported", () => {
  assert.deepEqual(parseThemePreference(null), { preference: "system" });
  for (const invalid of ["", "midnight", "Dark", undefined, 0, {}, "dark; color:red"]) {
    assert.equal(isThemePreference(invalid), false);
    const result = parseThemePreference(invalid);
    assert.equal(result.preference, "system");
    assert.match(result.issue ?? "", /not recognized/);
  }
});

test("system appearance responds to the device without overriding an explicit choice", () => {
  assert.equal(resolveTheme("system", false), "light");
  assert.equal(resolveTheme("system", true), "dark");
  for (const theme of ["light", "dark", "sepia", "contrast"] as const) {
    assert.equal(resolveTheme(theme, false), theme);
    assert.equal(resolveTheme(theme, true), theme);
  }
});

test("native chrome follows the system or the selected palette's lightness", () => {
  assert.equal(nativeTheme("system"), null);
  assert.equal(nativeTheme("light"), "light");
  assert.equal(nativeTheme("sepia"), "light");
  assert.equal(nativeTheme("dark"), "dark");
  assert.equal(nativeTheme("contrast"), "dark");
});

test("appearance persistence uses only the namespaced preference key", () => {
  const writes: [string, string][] = [];
  assert.equal(storeThemePreference("dark", (key, value) => writes.push([key, value])), null);
  assert.deepEqual(writes, [[THEME_STORAGE_KEY, "dark"]]);
  assert.deepEqual(readThemePreference(() => "sepia"), { preference: "sepia" });
});

test("unavailable storage produces an explicit, actionable error", () => {
  const loaded = readThemePreference(() => { throw new Error("Storage denied"); });
  assert.equal(loaded.preference, "system");
  assert.match(loaded.issue ?? "", /could not be read.*Storage denied/);
  const issue = storeThemePreference("contrast", () => { throw new Error("Quota exceeded"); });
  assert.match(issue ?? "", /this session.*could not be saved.*Quota exceeded/);
});
