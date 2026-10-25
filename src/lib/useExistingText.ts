import { useCallback, useEffect, useState } from "react";
import { bridge } from "./bridge";
import { errorMessage } from "./geometry";
import type { DocumentInfo, ExistingTextRun } from "./types";

export function useExistingText(info: DocumentInfo | null, page: number, enabled: boolean) {
  const [runs, setRuns] = useState<ExistingTextRun[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const retry = useCallback(() => setReload((value) => value + 1), []);
  useEffect(() => {
    let active = true;
    setRuns([]); setError(null); setLoading(false);
    if (!info || !enabled) return;
    setLoading(true);
    void bridge.textRuns(info, page).then((runs) => {
      if (active) { setRuns(runs); setLoading(false); }
    }).catch((error: unknown) => {
      if (active) { setError(errorMessage(error)); setLoading(false); }
    });
    return () => { active = false; };
  }, [info?.id, info?.revision, page, enabled, reload]);
  return { runs, loading, error, retry };
}
