import { useEffect, useState } from "react";
import type { DocumentInfo } from "./types";
import { errorMessage } from "./geometry";

export function useDocumentUtility<T>(info: DocumentInfo | null, load: (info: DocumentInfo) => Promise<T>) {
  const [value, setValue] = useState<T | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    setValue(null); setError(null);
    if (!info) { setLoading(false); return; }
    setLoading(true);
    void load(info).then((next) => {
      if (active) { setValue(next); setLoading(false); }
    }).catch((error: unknown) => {
      if (active) { setError(errorMessage(error)); setLoading(false); }
    });
    return () => { active = false; };
  }, [info?.id, info?.revision, attempt, load]);
  return { value, loading, error, retry: () => setAttempt((value) => value + 1) };
}
