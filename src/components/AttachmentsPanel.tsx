import { Download, LoaderCircle, Paperclip, Plus, Trash2 } from "lucide-react";
import { documentUtilities } from "../lib/documentUtilitiesBridge";
import type { AttachmentInfo } from "../lib/documentUtilitiesModel";
import { useDocumentUtility } from "../lib/useDocumentUtility";
import { formatBytes } from "../lib/geometry";
import type { DocumentInfo } from "../lib/types";
import IconButton from "./IconButton";

export default function AttachmentsPanel({ info, editDisabled, exportDisabled, onAdd, onExport, onRemove }: {
  info: DocumentInfo | null;
  editDisabled: boolean;
  exportDisabled: boolean;
  onAdd(): void;
  onExport(attachment: AttachmentInfo): void;
  onRemove(attachment: AttachmentInfo): void;
}) {
  const resource = useDocumentUtility(info, documentUtilities.attachments);
  return <div className="document-utility-panel">
    <div className="utility-panel-intro">
      <button className="button secondary" disabled={editDisabled || resource.loading || Boolean(resource.error)} onClick={onAdd}><Plus size={15} />Add attachment</button>
      <p>Attached files are untrusted. Folio never opens or executes them.</p>
    </div>
    <div className="panel-scroll attachment-list">
      {resource.loading && <p className="utility-progress" role="status"><LoaderCircle size={14} className="spin" />Inspecting embedded files...</p>}
      {resource.error && <div role="alert" className="utility-error"><p>{resource.error}</p><button className="text-button" onClick={resource.retry}>Retry</button></div>}
      {!resource.loading && !resource.error && !resource.value?.length && <div className="panel-empty"><Paperclip size={28} /><h3>No document attachments</h3><p>Add a local file to embed it in this PDF.</p></div>}
      {resource.value?.map((attachment) => <article className="attachment-entry" key={attachment.id}>
        <Paperclip size={16} />
        <div><h3>{attachment.name}</h3><p>{formatBytes(attachment.byteLength)}</p>
          {attachment.description && <p>{attachment.description}</p>}
          <div className="attachment-actions">
            <button className="text-button" disabled={exportDisabled || info?.copyAllowed === false} onClick={() => onExport(attachment)}><Download size={13} />Save attachment</button>
            <IconButton label={`Remove attachment ${attachment.name}`} disabled={editDisabled} onClick={() => onRemove(attachment)}><Trash2 size={14} /></IconButton>
          </div>
        </div>
      </article>)}
    </div>
    <div className="utility-footer">
      <p>Up to 128 files, 16 MiB each and 64 MiB total, before and after decoding. Saves keep the current PDF protection.</p>
      <p>This list manages the document EmbeddedFiles name tree, not portfolios, page attachment annotations, or other associated-file references. Removal is not secure erasure.</p>
    </div>
  </div>;
}
