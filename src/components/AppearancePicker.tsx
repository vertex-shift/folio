import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { BookOpen, Contrast, Monitor, Moon, Palette, Sun } from "lucide-react";
import { setTheme as setNativeTheme } from "@tauri-apps/api/app";
import { native } from "../lib/bridge";
import { errorMessage } from "../lib/geometry";
import {
  THEME_OPTIONS, THEME_STORAGE_KEY, nativeTheme, parseThemePreference,
  readThemePreference, resolveTheme, storeThemePreference,
} from "../lib/themes";
import type { ThemePreference } from "../lib/themes";
import IconButton from "./IconButton";

const icons = { system: Monitor, light: Sun, dark: Moon, sepia: BookOpen, contrast: Contrast };

export default function AppearancePicker({ onError }: { onError(message: string): void }) {
  const [initial] = useState(() => readThemePreference(() => localStorage.getItem(THEME_STORAGE_KEY)));
  const [preference, setPreference] = useState<ThemePreference>(initial.preference);
  const [saved, setSaved] = useState(!initial.issue);
  const [systemDark, setSystemDark] = useState(() => matchMedia("(prefers-color-scheme: dark)").matches);
  const [open, setOpen] = useState(false);
  const control = useRef<HTMLDivElement>(null);
  const reportError = useRef(onError);
  const nativeUpdates = useRef<Promise<void>>(Promise.resolve());
  reportError.current = onError;
  const resolved = resolveTheme(preference, systemDark);

  useLayoutEffect(() => {
    const root = document.documentElement;
    // Switch text and surfaces together, then restore normal hover transitions.
    root.dataset.themeChanging = "true";
    root.dataset.theme = resolved;
    const color = getComputedStyle(root).getPropertyValue("--titlebar").trim();
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", color);
    let secondFrame = 0;
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => { delete root.dataset.themeChanging; });
    });
    return () => {
      cancelAnimationFrame(firstFrame);
      cancelAnimationFrame(secondFrame);
      delete root.dataset.themeChanging;
    };
  }, [resolved]);

  useEffect(() => {
    if (initial.issue) reportError.current(initial.issue);
  }, [initial]);

  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const change = () => setSystemDark(media.matches);
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, []);

  useEffect(() => {
    if (!native) return;
    // Serialize native updates so rapid theme changes leave the latest choice applied.
    nativeUpdates.current = nativeUpdates.current
      .then(() => setNativeTheme(nativeTheme(preference)))
      .catch((error: unknown) => {
        reportError.current(`The workspace theme changed, but native appearance could not be updated. ${errorMessage(error)}`);
      });
  }, [preference]);

  useEffect(() => {
    function sync(event: StorageEvent) {
      if (event.key !== THEME_STORAGE_KEY && event.key !== null) return;
      const next = parseThemePreference(event.newValue);
      setPreference(next.preference);
      setSaved(!next.issue);
      if (next.issue) reportError.current(next.issue);
    }
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, []);

  useEffect(() => {
    if (!open) return;
    control.current?.querySelector<HTMLInputElement>("input:checked")?.focus();
    const dismiss = (event: Event) => {
      if (event.target instanceof Node && !control.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("focusin", dismiss);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("focusin", dismiss);
    };
  }, [open]);

  function choose(next: ThemePreference) {
    setPreference(next);
    const issue = storeThemePreference(next, (key, value) => localStorage.setItem(key, value));
    setSaved(issue === null);
    if (issue) reportError.current(issue);
  }

  return <div className="appearance-control" ref={control}>
    <IconButton label="Appearance" className="appearance-toggle" active={open}
      aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? "appearance-panel" : undefined}
      onClick={() => setOpen((value) => !value)}><Palette size={17} /></IconButton>
    {open && <section id="appearance-panel" className="appearance-popover" role="dialog" aria-label="Appearance" aria-modal="false"
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        control.current?.querySelector<HTMLButtonElement>(".appearance-toggle")?.focus();
      }}>
      <header className="appearance-heading"><h2>Appearance</h2><span>Make yourself at home.</span></header>
      <fieldset aria-label="Color theme" className="appearance-options">
        {THEME_OPTIONS.map((option) => {
          const Icon = icons[option.id];
          return <label key={option.id} className={`appearance-choice ${preference === option.id ? "selected" : ""}`}>
            <input type="radio" name="folio-theme" value={option.id} aria-label={option.label}
              aria-describedby={`theme-description-${option.id}`} checked={preference === option.id}
              onChange={() => choose(option.id)} />
            <Icon size={17} />
            <span className="appearance-choice-text"><strong>{option.label}</strong><span id={`theme-description-${option.id}`}>{option.description}</span></span>
            <span className="appearance-swatch" data-theme-preview={option.id === "system" ? systemDark ? "dark" : "light" : option.id} aria-hidden="true" />
          </label>;
        })}
      </fieldset>
      <footer className="appearance-footer">
        <p>{!saved ? "Appearance is applied for this session only." : preference === "system" ? `Following your device's ${resolved} appearance.` : "Your choice is saved on this device."}</p>
        <p>PDF colors and saved documents stay unchanged.</p>
        {!saved && <button className="text-button" onClick={() => {
          choose(preference);
          control.current?.querySelector<HTMLInputElement>("input:checked")?.focus();
        }}>Retry saving preference</button>}
      </footer>
    </section>}
  </div>;
}
