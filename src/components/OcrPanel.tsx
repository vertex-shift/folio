import { ScanText, ShieldCheck, X } from "lucide-react";
import { useId } from "react";
import type { OcrOptions, OcrOutcome, OcrProgress } from "../lib/ocrTypes";
import { OCR_LANGUAGES, ocrLanguageNames, selectedOcrLanguages } from "../lib/ocrLanguages";
import type { OcrLanguage } from "../lib/ocrLanguages";
import "./OcrPanel.css";

interface Props {
  range: string;
  dpi: OcrOptions["dpi"];
  policy: OcrOptions["existingText"];
  languages: OcrLanguage[];
  disabled: boolean;
  blockedReason: string | null;
  pageCount: number;
  outcome: OcrOutcome | null;
  onRange(value: string): void;
  onDpi(value: OcrOptions["dpi"]): void;
  onPolicy(value: OcrOptions["existingText"]): void;
  onLanguages(value: OcrLanguage[]): void;
  onStart(): void;
}

function pageList(pages: number[]): string {
  return pages.map((page) => page + 1).join(", ");
}

export default function OcrPanel(props: Props) {
  const rangeHelp = useId();
  const languageHelp = useId();
  const changeLanguage = (index: number, value: string) => {
    const languages = props.languages.filter((language, position) => position !== index && language !== value);
    if (value) languages.splice(Math.min(index, languages.length), 0, ...selectedOcrLanguages([value]));
    props.onLanguages(selectedOcrLanguages(languages));
  };
  return <div className="panel-scroll ocr-panel">
    <p>Add an invisible, searchable text layer to scanned pages. Original images and page content stay intact.</p>
    <div className="ocr-local"><ShieldCheck size={15} /><span>15 models included. Recognition and fonts stay on this device, even offline.</span></div>
    <form onSubmit={(event) => { event.preventDefault(); props.onStart(); }}>
      <label className="setting-field">Pages to recognize
        <input type="text" value={props.range} placeholder={`1-${props.pageCount}`} maxLength={400}
          aria-describedby={rangeHelp} disabled={props.disabled}
          onChange={(event) => props.onRange(event.target.value)} />
      </label>
      <p id={rangeHelp} className="ocr-help">Use page numbers or ranges, such as 1, 3-5. Up to 50 pages per job, processed in document order.</p>
      {[0, 1, 2].map((index) => <label className="setting-field" key={index}>
        {index === 0 ? "OCR language" : `Additional language ${index}`}
        <select value={props.languages[index] ?? ""} disabled={props.disabled}
          aria-describedby={languageHelp} onChange={(event) => changeLanguage(index, event.target.value)}>
          {index > 0 && <option value="">None</option>}
          {OCR_LANGUAGES.map((language) => <option key={language.id} value={language.id}
            disabled={props.languages.some((selected, position) => selected === language.id && position !== index)}>
            {language.name}
          </option>)}
        </select>
      </label>)}
      <p id={languageHelp} className="ocr-help">Choose the actual scan languages, up to three. The first model has priority. No automatic language detection or English fallback.</p>
      <div className="ocr-script-support">{props.languages.map((id) => {
        const language = OCR_LANGUAGES.find((item) => item.id === id)!;
        return <p key={id}>{language.name}: {language.scripts.join(", ")}; {language.direction === "rtl" ? "right to left" : "left to right"}.</p>;
      })}</div>
      <details className="ocr-catalog"><summary>All bundled languages and scripts</summary>
        <ul>{OCR_LANGUAGES.map((language) => <li key={language.id}>
          {language.name} ({language.id}): {language.scripts.join(", ")}; {language.direction.toUpperCase()}.
        </li>)}</ul>
      </details>
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
      <p>{ocrLanguageNames(props.outcome.languages)}. {props.outcome.wordCount
        ? `${props.outcome.wordCount} words on ${props.outcome.processedPages - props.outcome.noTextPages.length} pages. Review recognition and save a copy to keep it. Undo removes this whole OCR job.`
        : "The working PDF is unchanged."}</p>
      {props.outcome.skippedPages.length > 0 && <p>Skipped existing text on pages {pageList(props.outcome.skippedPages)}.</p>}
      {props.outcome.noTextPages.length > 0 && <p>No text recognized on pages {pageList(props.outcome.noTextPages)}. Check scan quality, language, and orientation.</p>}
    </div>}
    <p className="ocr-limits">Horizontal printed scans only. Review misread words, spacing, and reading order. Handwriting, vertical Japanese/Chinese, and languages outside the catalog are not supported. Annotations and form fields are not recognition input.</p>
    <p className="ocr-limits">The invisible layer preserves logical Unicode, including Arabic and Hindi. It does not enable visible Arabic/Indic paragraph editing. Tagged PDFs are refused; recognize the untagged original before making a new tagged copy.</p>
    <p className="ocr-limits">This adds PDF text, not accessibility tags or PDF/UA conformance. No original content is removed or redacted.</p>
  </div>;
}

export function OcrJobProgress({ progress, canceling, onCancel }: {
  progress: OcrProgress; canceling: boolean; onCancel(): void;
}) {
  const message = progress.stage === "preparing" ? "Preparing offline OCR"
    : progress.stage === "loading" ? "Checking and loading selected local models"
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
