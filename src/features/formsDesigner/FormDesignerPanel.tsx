import { Check, FileOutput, LoaderCircle, MapPin, Plus, RefreshCw, Trash2 } from "lucide-react";
import type { DocumentInfo } from "../../lib/types";
import type { FormValue } from "../../lib/formsTypes";
import { buttonState, controlLabels, newWidget } from "./model";
import type { ControlKind, DesignerDraft, DesignerInfo, FieldDefinition } from "./types";

function emptyDefault(kind: ControlKind): FormValue | null {
  return kind === "text" ? { type: "text", text: "" } : ["checkbox", "radio"].includes(kind)
    ? { type: "button", state: null } : ["list", "combo"].includes(kind) ? { type: "choice", indices: [], custom: null } : null;
}

function DefaultControl({ definition, disabled, onChange }: {
  definition: DesignerDraft["definition"]; disabled: boolean; onChange(value: FormValue): void;
}) {
  const value = definition.defaultValue;
  if (value?.type === "text") return definition.multiline
    ? <textarea aria-label="Default text" rows={3} value={value.text} disabled={disabled}
      onChange={(event) => onChange({ type: "text", text: event.target.value })} />
    : <input aria-label="Default text" type={definition.password ? "password" : "text"} value={value.text}
      autoComplete="off" disabled={disabled} onChange={(event) => onChange({ type: "text", text: event.target.value })} />;
  if (value?.type === "button") return <select aria-label="Default button state" value={value.state ?? ""}
    disabled={disabled} onChange={(event) => onChange({ type: "button", state: event.target.value || null })}>
    <option value="">No selection / Off</option>
    {definition.options.map((option, index) => <option key={index} value={buttonState(option.exportValue)}>{option.label}</option>)}
  </select>;
  if (value?.type === "choice") return <>
    <select aria-label="Default choice" multiple={definition.multiple} disabled={disabled}
      value={definition.multiple ? value.indices.map(String) : value.custom !== null ? "custom" : String(value.indices[0] ?? "")}
      onChange={(event) => {
        const indices = Array.from(event.target.selectedOptions, (option) => option.value)
          .filter((value) => value !== "" && value !== "custom").map(Number).sort((a, b) => a - b);
        onChange({ type: "choice", indices, custom: event.target.value === "custom" ? "" : null });
      }}>
      {!definition.multiple && <option value="">No selection</option>}
      {definition.options.map((option, index) => <option key={index} value={index}>{option.label}</option>)}
      {definition.editableChoice && <option value="custom">Custom text</option>}
    </select>
    {value.custom !== null && <input aria-label="Default custom choice" value={value.custom} disabled={disabled}
      onChange={(event) => onChange({ type: "choice", indices: [], custom: event.target.value })} />}
  </>;
  return null;
}

export default function FormDesignerPanel(props: {
  info: DocumentInfo; page: number; designer: DesignerInfo | null; draft: DesignerDraft | null;
  widgetIndex: number; loading: boolean; stale: boolean; error: string | null; notice: string | null;
  disabled: boolean; retry(): void; onNew(kind: ControlKind): void; onSelect(id: string): void;
  onChange(draft: DesignerDraft): void; onWidget(index: number): void; onTarget(): void;
  onLocate(page: number): void; onApply(): void; onDiscard(): void; onFlatten(): void; onFill(): void;
}) {
  const { draft, designer } = props;
  const locked = props.disabled || props.loading || props.stale || !designer?.authorable;
  const definition = draft?.definition;
  function change(values: Partial<DesignerDraft["definition"]>) {
    if (draft) props.onChange({ ...draft, definition: { ...draft.definition, ...values } });
  }
  function flag(name: keyof Pick<FieldDefinition, "required" | "readOnly" | "multiline" | "password" | "comb" | "multiple" | "editableChoice" | "noToggleToOff">, label: string) {
    return <label className="checkbox-setting"><input type="checkbox" disabled={locked}
      checked={Boolean(definition?.[name])} onChange={(event) => change({ [name]: event.target.checked })} />{label}</label>;
  }
  return <div className="form-designer-panel">
    <div className="designer-intro">
      <p>Create interactive PDF fields, or select a logical field to edit its properties and page widgets.</p>
      <select aria-label="Create field type" value="" disabled={locked || Boolean(draft?.dirty)}
        onChange={(event) => props.onNew(event.target.value as ControlKind)}>
        <option value="" disabled>Create a field...</option>
        {Object.entries(controlLabels).map(([kind, label]) => <option value={kind} key={kind}>{label}</option>)}
      </select>
      <label className="designer-field">Existing logical field<select aria-label="Existing logical field" value={draft?.fieldId ?? ""} disabled={props.disabled || props.loading || Boolean(draft?.dirty)}
        onChange={(event) => props.onSelect(event.target.value)}>
        <option value="" disabled>{designer?.fields.length ? "Choose a field..." : "No existing fields"}</option>
        {designer?.fields.map(({ field, editable }) => <option key={field.id} value={field.id} disabled={!editable}>
          {field.name || field.id} ({field.kind === "signature" ? "unsigned signature" : field.kind})
        </option>)}
      </select></label>
      {props.loading && <p role="status"><LoaderCircle className="spin" size={14} />Inspecting field structure...</p>}
      {designer?.reason && <p className="designer-notice" role="status">{designer.reason}</p>}
      {props.stale && <p className="designer-notice" role="alert">The PDF changed. This draft is kept; discard it explicitly before editing the new revision.</p>}
      {!designer && !props.loading && <button className="text-button" disabled={props.disabled} onClick={props.retry}><RefreshCw size={14} />Retry inspection</button>}
    </div>
    <div className="panel-scroll designer-settings">
      {!draft && !props.loading && <p>Select a field on the page, choose an existing field above, or create a supported control. Layout and property changes remain drafts until applied.</p>}
      {draft && definition && <>
        <div className="designer-context"><h3>{draft.fieldId ? "Edit logical field" : `New ${controlLabels[definition.kind].toLowerCase()}`}</h3>
          <p>{draft.fieldId ? `Field ${draft.fieldId} · ${draft.widgets.length} page widget${draft.widgets.length === 1 ? "" : "s"}` : "Creates a real AcroForm field, not static text."}</p></div>
        {draft.deleteField ? <div className="designer-notice" role="status"><p>Applying will remove this logical field and all its widgets. Other fields and their values stay unchanged.</p>
          <button className="text-button" disabled={locked} onClick={() => props.onChange({ ...draft, deleteField: false })}>Keep this field</button></div>
          : <>
            <fieldset disabled={locked} className="designer-group">
              <legend>Field properties</legend>
              <label className="designer-field">Partial name<input value={definition.name} maxLength={256}
                onChange={(event) => change({ name: event.target.value })} /></label>
              <label className="designer-field">Accessible label<input value={definition.label} maxLength={256}
                onChange={(event) => change({ label: event.target.value })} placeholder="Describe this control" /></label>
              {flag("required", "Required when filling")}
              {flag("readOnly", "Read-only when filling")}
              {definition.kind === "text" && <>
                {flag("multiline", "Multiline")}{flag("password", "Mask password text")}{flag("comb", "Comb (one character per cell)")}
                <div className="designer-grid">
                  <label className="designer-field">Maximum characters<input type="number" min="1" max={definition.comb ? 256 : 4000}
                    value={definition.maxLength} onChange={(event) => change({ maxLength: event.target.value })} /></label>
                  <label className="designer-field">Font size (PDF units)<input type="number" min="0" max="256" step="0.5"
                    value={definition.fontSize} onChange={(event) => change({ fontSize: event.target.value })} /></label>
                </div>
                <p>Font size 0 fits automatically. Fixed sizes never shrink silently. Masking is not file encryption.</p>
              </>}
              {["list", "combo"].includes(definition.kind) && <>
                {definition.kind === "list" ? flag("multiple", "Allow multiple selections") : flag("editableChoice", "Allow custom choice text")}
                <label className="designer-field">Font size (PDF units, 0 = fit)<input type="number" min="0" max="256" step="0.5"
                  value={definition.fontSize} onChange={(event) => change({ fontSize: event.target.value })} /></label>
              </>}
              {["text", "list", "combo"].includes(definition.kind) && <label className="designer-field">Text alignment<select aria-label="Text alignment" value={definition.alignment}
                onChange={(event) => change({ alignment: Number(event.target.value) })}>
                <option value="0">Left</option><option value="1">Center</option><option value="2">Right</option>
              </select></label>}
              {definition.kind === "radio" && flag("noToggleToOff", "Require a selection once filled")}
              {definition.kind === "signature" && <p className="designer-notice">This creates an empty digital-signature slot. It is unsigned, does not sign or certify the PDF, and does not make other fields read-only.</p>}
            </fieldset>
            {definition.options.length > 0 && <fieldset className="designer-group" disabled={locked}>
              <legend>{["radio", "checkbox"].includes(definition.kind) ? "Button states" : "Choice options"}</legend>
              {definition.options.map((option, index) => <div className="designer-option" key={index}>
                <label className="designer-field">Option {index + 1} label<input value={option.label}
                  onChange={(event) => change({ options: definition.options.map((item, at) => at === index ? { ...item, label: event.target.value } : item) })} /></label>
                <label className="designer-field">Export value<input value={option.exportValue}
                  disabled={locked || Boolean(draft.fieldId && ["radio", "checkbox"].includes(definition.kind))}
                  onChange={(event) => change({ options: definition.options.map((item, at) => at === index ? { ...item, exportValue: event.target.value } : item) })} /></label>
                {definition.kind !== "checkbox" && (!draft.fieldId || definition.kind !== "radio") && definition.options.length > 1 &&
                  <button className="text-button" onClick={() => props.onChange({ ...draft,
                    definition: { ...definition, options: definition.options.filter((_, at) => at !== index) },
                    widgets: definition.kind === "radio" ? draft.widgets.filter((_, at) => at !== index).map((item, option) => ({ ...item, option })) : draft.widgets,
                  })}>Remove option {index + 1}</button>}
              </div>)}
              {definition.kind !== "checkbox" && (!draft.fieldId || definition.kind !== "radio") && <button className="text-button" onClick={() => {
                const index = definition.options.length;
                props.onChange({ ...draft, definition: { ...definition, options: [...definition.options, { label: `Option ${index + 1}`, exportValue: `Option${index + 1}` }] },
                  widgets: definition.kind === "radio" ? [...draft.widgets, newWidget(props.info, props.page, "radio", index)] : draft.widgets });
              }}><Plus size={14} />Add option</button>}
              <p>Export values identify saved choices. Existing button states keep their identities; labels can change.</p>
            </fieldset>}
            {definition.kind !== "signature" && <fieldset className="designer-group" disabled={locked}>
              <legend>Default value</legend>
              <label className="checkbox-setting"><input type="checkbox" checked={definition.defaultValue !== null}
                onChange={(event) => change({ defaultValue: event.target.checked ? emptyDefault(definition.kind) : null })} />Set a default value</label>
              <DefaultControl definition={definition} disabled={locked} onChange={(defaultValue) => change({ defaultValue })} />
              <p>{draft.fieldId ? "Defaults do not replace current values. Use Fill form fields to change current values before tightening constraints."
                : "The default is also the new field's initial value. Required fields may start blank."}</p>
            </fieldset>}
            <fieldset className="designer-group" disabled={locked}>
              <legend>Page widgets</legend>
              {draft.widgets.map((widget, index) => <div className={`designer-widget ${props.widgetIndex === index ? "selected" : ""}`} key={widget.id ?? index}>
                <div className="designer-widget-heading"><button className="text-button" type="button" aria-pressed={props.widgetIndex === index}
                  onClick={() => props.onWidget(index)}>Widget {index + 1}{definition.kind === "radio" && !draft.fieldId ? ` · ${definition.options[widget.option ?? index]?.label ?? ""}` : ""}</button>
                  <button className="text-button" type="button" onClick={() => {
                    const page = Number(widget.page) - 1;
                    if (Number.isSafeInteger(page) && page >= 0 && page < props.info.pageCount) props.onLocate(page);
                  }}><MapPin size={12} />Locate</button></div>
                <label className="designer-field">Page<input type="number" min="1" max={props.info.pageCount} value={widget.page}
                  onChange={(event) => props.onChange({ ...draft, widgets: draft.widgets.map((item, at) => at === index ? { ...item, page: event.target.value } : item) })} /></label>
                <div className="designer-grid">{(["x", "y", "width", "height"] as const).map((key) =>
                  <label className="designer-field" key={key}>{{ x: "Left", y: "Top", width: "Width", height: "Height" }[key]} (pt)
                    <input type="number" step="0.1" min={key === "width" || key === "height" ? "0.1" : "0"} value={widget[key]}
                      onChange={(event) => props.onChange({ ...draft, widgets: draft.widgets.map((item, at) => at === index ? { ...item, [key]: event.target.value } : item) })} />
                  </label>)}</div>
                {draft.widgets.length > 1 && (draft.fieldId || definition.kind !== "radio") && <button className="text-button danger-text" onClick={() => {
                  props.onChange({ ...draft, widgets: draft.widgets.filter((_, at) => at !== index),
                    removedWidgets: widget.id ? [...draft.removedWidgets, widget.id] : draft.removedWidgets });
                  props.onWidget(0);
                }}><Trash2 size={12} />Remove widget {index + 1}</button>}
              </div>)}
              <button className="button secondary" onClick={props.onTarget}><MapPin size={14} />Place selected widget on page</button>
              <p>Click to move, or drag a new box to move and resize. Numeric coordinates are physical points from the visible rotated page's top-left.</p>
              {!draft.fieldId && !["signature", "radio"].includes(definition.kind) && <button className="text-button" disabled={draft.widgets.length >= 64}
                onClick={() => props.onChange({ ...draft, widgets: [...draft.widgets, newWidget(props.info, props.page, definition.kind)] })}>
                <Plus size={14} />Add repeated widget</button>}
              {draft.removedWidgets.length > 0 && <p>{draft.removedWidgets.length} widget removal(s) staged. Discard restores the draft.</p>}
            </fieldset>
            {draft.fieldId && <button className="text-button danger-text" disabled={locked} onClick={() => props.onChange({ ...draft, deleteField: true })}>
              <Trash2 size={14} />Remove logical field and all widgets</button>}
          </>}
      </>}
      <details className="designer-support"><summary>Capabilities and exact limits</summary>
        {(designer?.warnings ?? []).map((warning) => <p key={warning}>{warning}</p>)}
        {designer?.fields.filter((field) => !field.editable).map(({ field, reason }) => <p key={field.id}><strong>{field.label}:</strong> {reason}</p>)}
      </details>
      <div className="designer-copy-actions"><button className="button secondary" disabled={props.disabled || Boolean(draft?.dirty)} onClick={props.onFill}>Fill form fields</button>
        <button className="button secondary" disabled={props.disabled || Boolean(draft?.dirty) || !designer?.fields.length} onClick={props.onFlatten}><FileOutput size={14} />Export static form copy</button>
        <p>Separate PDF with values painted into pages and widgets removed. Not secure sanitization.</p></div>
    </div>
    <div className="designer-commit">
      {props.error && <p className="inline-error" role="alert">{props.error}</p>}
      {props.notice && <p className="designer-success" role="status">{props.notice}</p>}
      <p role="status">{draft?.dirty ? "Unapplied form-design draft. Save requires applying or discarding it." : "The PDF changes only when you apply."}</p>
      <button className={`button ${draft?.deleteField ? "danger" : "primary"}`} disabled={locked || !draft || Boolean(draft.fieldId && !draft.dirty)} onClick={props.onApply}>
        <Check size={14} />{draft?.deleteField ? "Remove field" : draft?.fieldId ? "Apply design changes" : "Create field"}</button>
      <button className="text-button" disabled={props.disabled || !draft} onClick={props.onDiscard}>Discard design draft</button>
    </div>
  </div>;
}
