import { FileText, LoaderCircle, RotateCcw, Save } from "lucide-react";
import { documentUtilities } from "../lib/documentUtilitiesBridge";
import { metadataChanged } from "../lib/documentUtilitiesModel";
import type { MetadataDraft, MetadataFields } from "../lib/documentUtilitiesModel";
import { useDocumentUtility } from "../lib/useDocumentUtility";
import type { DocumentInfo } from "../lib/types";

export default function DocumentPropertiesPanel({ info, draft, disabled, onDraft, onApply, onDiscard }: {
  info: DocumentInfo | null;
  draft: MetadataDraft | null;
  disabled: boolean;
  onDraft(draft: MetadataDraft): void;
  onApply(): void;
  onDiscard(): void;
}) {
  const resource = useDocumentUtility(info, documentUtilities.metadata);
  const metadata = resource.value;
  const fields = draft?.fields ?? metadata?.fields;
  const changed = metadataChanged(draft);
  const stale = draft !== null && (draft.documentId !== info?.id || draft.revision !== info?.revision);
  const locked = disabled || !metadata?.editable || stale;
  function change(key: keyof MetadataFields, value: string) {
    if (!info || !metadata || !fields) return;
    onDraft(draft ? { ...draft, fields: { ...fields, [key]: value } } : {
      documentId: info.id, revision: info.revision, original: metadata.fields, fields: { ...fields, [key]: value },
    });
  }
  return <div className="document-utility-panel">
    <div className="panel-scroll document-properties">
      <p className="utility-intro">Describe this PDF. Apply changes to the working document, then save a copy.</p>
      {resource.loading && <p className="utility-progress" role="status"><LoaderCircle size={14} className="spin" />Reading metadata...</p>}
      {resource.error && <div role="alert" className="utility-error"><p>{resource.error}</p><button className="text-button" onClick={resource.retry}>Retry</button></div>}
      {metadata?.reason && <p className="utility-error" role="status">{metadata.reason}</p>}
      {stale && <p className="utility-error" role="alert">The PDF changed while this draft was open. Your draft is retained; discard it and review the current properties before editing.</p>}
      {fields && <fieldset className="metadata-fields" disabled={locked}>
        <legend className="visually-hidden">Descriptive PDF metadata</legend>
        {(["title", "author", "subject", "keywords"] as const).map((key) =>
          <label key={key} className="utility-field">{({ title: "Title", author: "Author", subject: "Subject", keywords: "Keywords" })[key]}
            {key === "subject" ? <textarea rows={3} maxLength={4096} value={fields[key]} onChange={(event) => change(key, event.target.value)} />
              : <input type="text" maxLength={4096} value={fields[key]} onChange={(event) => change(key, event.target.value)} />}
          </label>)}
      </fieldset>}
      {metadata && <p className="utility-note">{metadata.xmpPresent
        ? "Updates synchronize these fields in both the classic Info dictionary and supported XMP, including keyword lists. Other properties are retained."
        : "This PDF uses classic descriptive metadata. Unrelated Info properties are retained."}</p>}
      <p className="utility-note">Changing or clearing these fields is not secure sanitization. Old objects or other metadata can remain. Use permanent redaction for marked page content.</p>
      {info && <dl className="utility-facts">
        <dt>Document</dt><dd><FileText size={13} />{info.name}</dd>
        <dt>Protection</dt><dd>{info.passwordProtected ? info.permissions.ownerAuthenticated ? "Password protected · owner access" : "Password protected · opening-password access" : "Not password protected"}</dd>
        <dt>Printing</dt><dd>{!info.permissions.printAllowed ? "Not permitted" : info.permissions.highQualityPrintAllowed ? "Permitted" : "Low-resolution permission only"}</dd>
        <dt>Copying</dt><dd>{info.copyAllowed ? "Permitted" : "Not permitted"}</dd>
      </dl>}
    </div>
    <div className="utility-footer">
      <p>{changed ? "Unapplied metadata draft. It stays with this tab." : "Metadata edits can be undone after applying."}</p>
      <div className="utility-actions">
        <button className="button secondary" disabled={disabled || !draft} onClick={onDiscard}><RotateCcw size={14} />Discard draft</button>
        <button className="button primary" disabled={locked || !changed} onClick={onApply}><Save size={14} />Apply</button>
      </div>
    </div>
  </div>;
}
