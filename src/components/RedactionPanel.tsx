import { useEffect, useState } from "react";
import { Eraser, FileOutput, ScanLine, Trash2 } from "lucide-react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import type { PdfRect, RedactionMark } from "../lib/types";
import { errorMessage } from "../lib/geometry";
import { redactionFromFields } from "../lib/redaction";
import IconButton from "./IconButton";

interface Props {
  pdf: PDFDocumentProxy | null;
  page: number;
  marks: RedactionMark[];
  disabled: boolean;
  exported: boolean;
  onMark(rect: PdfRect): void;
  onPage(page: number): void;
  onRemove(id: number): void;
  onClear(): void;
  onExport(): void;
  onDraw(): void;
}

export default function RedactionPanel(props: Props) {
  const [fields, setFields] = useState(["", "", "", ""]);
  const [bounds, setBounds] = useState<readonly number[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    setBounds(null); setFailure(null); setFields(["", "", "", ""]);
    if (props.pdf) {
      void props.pdf.getPage(props.page + 1).then((page) => {
        if (current) setBounds(page.view);
      }).catch((error: unknown) => {
        if (current) setFailure(`Page coordinates could not be read: ${errorMessage(error)}`);
      });
    }
    return () => { current = false; };
  }, [props.pdf, props.page]);

  return <div className="redaction-panel">
    <div className="panel-scroll redaction-review">
      <div className="redaction-intro">
        <p>Drag a box around sensitive content. Include the full letters or image and check every occurrence.</p>
        <button className="button secondary redaction-mark-page" disabled={props.disabled} onClick={props.onDraw}><Eraser size={15} />Mark on page</button>
        <p><strong>Red marks are pending.</strong> Only an exported copy is permanently redacted.</p>
        {props.marks.length > 0 && <p className="redaction-lock-hint">Clear all marks before editing the original.</p>}
      </div>
      {props.exported && <p className="redaction-exported" role="status">
        Redacted copy prepared. This window still shows the original. Open the exported PDF and inspect it before sharing.
      </p>}
      <div className="redaction-summary"><span>{props.marks.length} pending {props.marks.length === 1 ? "area" : "areas"}</span>
        <button className="text-button" disabled={props.disabled || !props.marks.length} onClick={props.onClear}>Clear all</button></div>
      <div className="redaction-regions">
      {!props.marks.length && <div className="panel-empty"><ScanLine size={28} /><h3>Cover what stays private.</h3>
        <p>Mark text, images, or any part of a page. No original content is removed until you export.</p></div>}
      {props.marks.map((mark) => <div className="redaction-entry" key={mark.id}>
        <button disabled={props.disabled} onClick={() => props.onPage(mark.page)}>
          <strong>Area {mark.id}</strong><span>Page {mark.page + 1} · {Math.round(mark.rect[2] - mark.rect[0])} × {Math.round(mark.rect[3] - mark.rect[1])} pt</span>
        </button>
        <IconButton label={`Remove redaction area ${mark.id}`} disabled={props.disabled} onClick={() => props.onRemove(mark.id)}><Trash2 size={15} /></IconButton>
      </div>)}
      </div>
      <details className="redaction-coordinates">
        <summary>Add an area by coordinates</summary>
        <p>Unrotated PDF points, with Y increasing upward. Current page: {props.page + 1}.</p>
        <form onSubmit={(event) => {
          event.preventDefault(); setFailure(null);
          try {
            if (!bounds) throw new Error("Wait for page coordinates to finish loading.");
            props.onMark(redactionFromFields(fields, bounds));
          } catch (error) { setFailure(errorMessage(error)); }
        }}>
          <div className="redaction-fields">
            {["Left (pt)", "Bottom (pt)", "Width (pt)", "Height (pt)"].map((label, index) =>
              <label key={label}>{label}<input type="number" step="any" value={fields[index]} disabled={props.disabled}
                onChange={(event) => setFields(fields.map((value, position) => position === index ? event.target.value : value))} /></label>)}
          </div>
          <button type="submit" className="button secondary" disabled={props.disabled || !bounds}>Add redaction area</button>
        </form>
        {failure && <p className="inline-error" role="alert">{failure}</p>}
      </details>
    </div>
    <div className="redaction-export-actions">
      <p>All pages become images; text search is lost. Your original stays unchanged.</p>
      <button className="button primary" disabled={props.disabled || !props.marks.length} onClick={props.onExport}><FileOutput size={15} />Export redacted copy</button>
    </div>
  </div>;
}
