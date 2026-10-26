import { ScanText, ShieldCheck, X } from "lucide-react";
import { useId } from "react";
import type { OcrOptions, OcrOutcome, OcrProgress } from "../lib/ocrTypes";
import "./OcrPanel.css";

interface Props {
  range: string;
  dpi: OcrOptions["dpi"];
  policy: OcrOptions["existingText"];
  disabled: boolean;
  blockedReason: string | null;
  pageCount: number;
  outcome: OcrOutcome | null;
  onRange(value: string): void;
  onDpi(value: OcrOptions["dpi"]): void;
  onPolicy(value: OcrOptions["existingText"]): void;
  onStart(): void;
}

function pageList(pages: number[]): string {
  return pages.map((page) => page + 1).join(", ");
}

export default function OcrPanel(props: Props) {
  const rangeHelp = useId();
  return <div className="panel-scroll ocr-panel">
    <p>Add an invisible, searchable text layer to scanned pages. Original images and page content stay intact.</p>
    <div className="ocr-local"><ShieldCheck size={15} /><span>English model included. Recognition stays on this device.</span></div>
    <form onSubmit={(event) => { event.preventDefault(); props.onStart(); }}>
      <label className="setting-field">Pages to recognize
        <input type="text" value={props.range} placeholder={`1-${props.pageCount}`} maxLength={400}
          aria-describedby={rangeHelp} disabled={props.disabled}
          onChange={(event) => props.onRange(event.target.value)} />
      </label>
      <p id={rangeHelp} className="ocr-help">Use page numbers or ranges, such as 1, 3-5. Up to 50 pages per job, processed in document order.</p>
      <label className="setting-field">Language<select value="eng" disabled aria-label="OCR language">
        <option value="eng">English (bundled)</option>
      </select></label>
      <label className="setting-field">Scan resolution<select value={props.dpi} disabled={props.disabled}
        onChange={(event) => props.onDpi(event.target.value === "150" ? 150 : 300)}>
        <option value={300}>300 DPI — finer text</option>
        <option value={150}>150 DPI — smaller pages in memory</option>
      </select></label>
      <label className="setting-field">Pages with existing text<select value={props.policy} disabled={props.disabled}
        onChange={(event) => props.onPolicy(event.target.value === "force" ? "force" : "skip")}>
        <option value="skip">Skip existing text (recommended)</option>
        <option value="force">Force an additional text layer</option>
      </select></label>
      {props.policy === "force" && <p className="ocr-warning">Force can duplicate searchable text, including previous OCR. It does not replace or remove the old text layer.</p>}
      {props.blockedReason && <p className="ocr-warning">{props.blockedReason}</p>}
      <button type="submit" className="button primary ocr-start" disabled={props.disabled || Boolean(props.blockedReason)}>
        <ScanText size={16} />Recognize text
      </button>
    </form>
    {props.outcome && <div className="ocr-outcome" role="status">
      <h3>{props.outcome.wordCount ? "Searchable text added" : "No text added"}</h3>
      <p>{props.outcome.wordCount
        ? `${props.outcome.wordCount} words on ${props.outcome.processedPages - props.outcome.noTextPages.length} pages. Review recognition and save a copy to keep it. Undo removes this whole OCR job.`
        : "The working PDF is unchanged."}</p>
      {props.outcome.skippedPages.length > 0 && <p>Skipped existing text on pages {pageList(props.outcome.skippedPages)}.</p>}
      {props.outcome.noTextPages.length > 0 && <p>No text recognized on pages {pageList(props.outcome.noTextPages)}. Check scan quality, language, and orientation.</p>}
    </div>}
    <p className="ocr-limits">Review the result: OCR can misread words and reading order. Upright English scans work best; handwriting and other languages are not supported. Annotations and form fields are not recognition input.</p>
    <p className="ocr-limits">This adds PDF text, not accessibility tags or PDF/UA conformance. No original content is removed or redacted.</p>
  </div>;
}

export function OcrJobProgress({ progress, canceling, onCancel }: {
  progress: OcrProgress; canceling: boolean; onCancel(): void;
}) {
  const message = progress.stage === "preparing" ? "Preparing offline OCR"
    : progress.stage === "loading" ? "Loading bundled English model"
    : progress.stage === "committing" ? "Adding searchable text"
    : `${progress.stage === "rendering" ? "Rendering" : progress.stage === "recognizing" ? "Recognizing" : "Checking"} page ${progress.page + 1}`;
  const value = progress.stage === "committing" ? progress.total
    : progress.completed + (progress.stage === "staging" ? 1 : progress.stage === "recognizing" ? progress.fraction : 0);
  return <div className="ocr-job-banner">
    <div><span role="status">{canceling ? "Canceling OCR; the PDF is unchanged" : message}</span>
      <span className="ocr-job-count">{progress.completed} of {progress.total} pages checked</span></div>
    <progress aria-label="OCR pages processed" value={value} max={Math.max(1, progress.total)} />
    <button className="button secondary" disabled={canceling || progress.stage === "committing"} onClick={onCancel}>
      <X size={14} />{canceling ? "Canceling" : "Cancel OCR"}
    </button>
  </div>;
}
