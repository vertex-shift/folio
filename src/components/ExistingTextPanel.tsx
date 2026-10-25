import { useState } from "react";
import { LoaderCircle, RefreshCw, Search, SquarePen, TriangleAlert } from "lucide-react";
import type { ExistingTextRun } from "../lib/types";

export default function ExistingTextPanel({ runs, loading, error, page, disabled, retry, onSelect }: {
  runs: ExistingTextRun[]; loading: boolean; error: string | null; page: number; disabled: boolean;
  retry(): void; onSelect(run: ExistingTextRun): void;
}) {
  const [query, setQuery] = useState("");
  const filtered = runs.filter((run) => !query || run.text?.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  return <div className="source-text-panel">
    <div className="source-text-intro"><p>Click existing text on the page or choose its source run below.</p>
      <p>Edits replace actual PDF content. They do not cover it with an annotation.</p>
      <label className="search-field"><Search size={15} /><input aria-label="Filter existing text" value={query}
        onChange={(event) => setQuery(event.target.value)} placeholder="Find a text run" /></label>
    </div>
    <div className="source-text-summary" role="status">Page {page + 1} · {runs.filter((run) => run.editable).length} editable runs</div>
    {loading && <p className="panel-progress" role="status"><LoaderCircle size={14} className="spin" />Reading PDF content...</p>}
    {error && <div className="source-text-error"><p className="inline-error" role="alert">{error}</p>
      <button className="button secondary" onClick={retry}><RefreshCw size={14} />Retry</button></div>}
    {!loading && !error && <div className="panel-scroll source-text-list">
      {!runs.length && <div className="panel-empty"><SquarePen size={28} /><h3>No source text on this page</h3>
        <p>Scans and outlined lettering need OCR. Placed text annotations can be edited in Comments.</p></div>}
      {runs.length > 0 && !filtered.length && <div className="panel-empty"><Search size={26} /><h3>No matching text</h3><p>Try a different word.</p></div>}
      {filtered.map((run) => <div className={`source-text-entry ${run.editable ? "" : "unsupported"}`} key={run.path.join("/")}>
        <button disabled={disabled || !run.editable} onClick={() => onSelect(run)}
          aria-label={`Edit source text: ${run.text?.trim().slice(0, 100) || "Unmapped text"}`}>
          <span className="source-text-font">{run.font.replace(/^[A-Z]{6}\+/, "")}
            {run.fontSize !== null ? ` · ${Math.round(Math.abs(run.fontSize) * 10) / 10} pt` : ""}</span>
          <span className="source-text-content">{run.text?.trim() || (run.text === null ? "Unmapped text" : "Whitespace run")}</span>
        </button>
        {!run.editable && <p className="source-text-reason"><TriangleAlert size={12} />{run.reason}</p>}
      </div>)}
    </div>}
  </div>;
}
