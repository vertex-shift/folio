import { useEffect, useRef, useState } from "react";
import { Check, FileInput, LoaderCircle, LockKeyhole, MapPin, RefreshCw, Search } from "lucide-react";
import type { FormDraft, FormField, FormInfo, FormValue, FormWidget } from "../lib/formsTypes";

function FieldControl({ field, value, inputId, disabled, describedBy, invalid, change }: {
  field: FormField; value: FormValue | null; inputId: string; disabled: boolean; describedBy: string; invalid: boolean;
  change(value: FormValue): void;
}) {
  const label = `${inputId}-label`;
  const shared = { id: inputId, disabled, "aria-labelledby": label, "aria-describedby": describedBy, "aria-invalid": invalid };
  if (value?.type === "text" && field.kind === "text") {
    return field.multiline
      ? <textarea {...shared} rows={3} value={value.text} maxLength={field.maxLength * 2}
        onChange={(event) => change({ type: "text", text: event.target.value })} />
      : <input {...shared} type={field.password ? "password" : "text"} value={value.text}
        autoComplete={field.password ? "new-password" : "off"} spellCheck={!field.password}
        autoCapitalize={field.password ? "none" : undefined} autoCorrect={field.password ? "off" : undefined}
        maxLength={field.maxLength * 2} onChange={(event) => change({ type: "text", text: event.target.value })} />;
  }
  if (value?.type === "button" && field.kind === "checkbox") {
    return <label className="form-checkbox"><input {...shared} type="checkbox" checked={value.state !== null}
      onChange={(event) => change({ type: "button", state: event.target.checked ? field.options[0]?.id ?? null : null })} />
      <span>{value.state === null ? "Not checked" : "Checked"}</span></label>;
  }
  if (value?.type === "button" && field.kind === "radio") {
    return <fieldset className="form-radio-options" disabled={disabled} aria-labelledby={label} aria-describedby={describedBy} aria-invalid={invalid}>
      {field.options.map((option) => <label key={option.id}>
        <input type="radio" name={inputId} value={option.id} checked={value.state === option.id}
          onChange={() => change({ type: "button", state: option.id })} /><span>{option.label}</span>
      </label>)}
      {!field.required && !field.noToggleToOff && <label><input type="radio" name={inputId} checked={value.state === null}
        onChange={() => change({ type: "button", state: null })} /><span>No selection</span></label>}
    </fieldset>;
  }
  if (value?.type === "choice" && field.kind === "choice") {
    if (field.multiple) return <select {...shared} multiple size={Math.min(6, Math.max(3, field.options.length))}
      value={value.indices.map(String)} onChange={(event) => change({
        type: "choice", custom: null,
        indices: Array.from(event.target.selectedOptions, (option) => Number(option.value)).sort((a, b) => a - b),
      })}>{field.options.map((option, index) => <option value={index} key={option.id}>{option.label}</option>)}</select>;
    return <>
      <select {...shared} value={value.custom !== null ? "custom" : value.indices[0]?.toString() ?? "none"}
        onChange={(event) => change({
          type: "choice", indices: ["custom", "none"].includes(event.target.value) ? [] : [Number(event.target.value)],
          custom: event.target.value === "custom" ? "" : null,
        })}>
        <option value="none">No selection</option>
        {field.options.map((option, index) => <option value={index} key={option.id}>{option.label}</option>)}
        {field.editableChoice && <option value="custom">Enter custom text...</option>}
      </select>
      {value.custom !== null && <input type="text" value={value.custom} disabled={disabled}
        aria-label={`Custom value for ${field.label}`} aria-describedby={describedBy} autoComplete="off"
        maxLength={field.maxLength * 2} onChange={(event) => change({ type: "choice", indices: [], custom: event.target.value })} />}
    </>;
  }
  return <p className="form-unfillable"><LockKeyhole size={13} />{field.kind === "signature" ? "Digital-signature field" : "Not a fillable control"}</p>;
}

export default function FormsPanel(props: {
  documentId: number; page: number; form: FormInfo | null; draft: FormDraft | null; count: number;
  loading: boolean; error: string | null; fieldErrors: Record<string, string>; stale: boolean;
  actionError: string | null; notice: string | null;
  disabled: boolean; retry(): void; onChange(id: string, value: FormValue): void;
  onApply(): void; onDiscard(): void; onLocate(widget: FormWidget): void;
}) {
  const [filter, setFilter] = useState("");
  const [currentPage, setCurrentPage] = useState(false);
  const filterRef = useRef<HTMLInputElement>(null);
  useEffect(() => { filterRef.current?.focus(); }, []);
  const fields = props.form?.fields.filter((field) => (!filter || `${field.label} ${field.name}`.toLocaleLowerCase().includes(filter.toLocaleLowerCase()))
    && (!currentPage || field.widgets.some((widget) => widget.page === props.page))) ?? [];
  return <div className="forms-panel">
    <div className="form-intro">
      <p>Fill existing PDF fields, then apply your changes together. Save a copy to keep them.</p>
      <label className="search-field"><Search size={15} /><input ref={filterRef} aria-label="Filter form fields" value={filter}
        onChange={(event) => setFilter(event.target.value)} placeholder="Find a field" /></label>
      <label className="checkbox-setting"><input type="checkbox" checked={currentPage}
        onChange={(event) => setCurrentPage(event.target.checked)} />Only page {props.page + 1}</label>
    </div>
    {props.loading && <p className="panel-progress" role="status"><LoaderCircle size={14} className="spin" />Inspecting form fields...</p>}
    {props.error && <div className="form-error"><p className="inline-error" role="alert">{props.error}</p>
      <button className="button secondary" disabled={props.disabled || props.loading} onClick={props.retry}><RefreshCw size={14} />Retry inspection</button></div>}
    {props.stale && <p className="form-notice" role="alert">The PDF changed after these drafts were started. Your values are kept for review; discard them explicitly before filling the new revision.</p>}
    {props.form?.reason && <p className="form-notice" role="status">{props.form.reason}</p>}
    <div className="panel-scroll form-fields">
      {!props.loading && !props.error && !props.form?.fields.length && <div className="panel-empty"><FileInput size={28} />
        <h3>No AcroForm fields</h3><p>This PDF has no existing interactive fields to fill. Use Add text for a static form.</p></div>}
      {Boolean(props.form?.fields.length) && !fields.length && <div className="panel-empty"><Search size={26} />
        <h3>No matching fields</h3><p>Change the filter or show all pages.</p></div>}
      {fields.map((field) => {
        const value = props.draft?.values[field.id] ?? field.value;
        const changed = Boolean(props.draft && Object.hasOwn(props.draft.values, field.id));
        const inputId = `form-${props.documentId}-${field.id.replace(":", "-")}`;
        const fieldError = props.fieldErrors[field.id];
        const disabled = props.disabled || props.loading || props.stale || !props.form?.fillable || field.readOnly;
        const pages = field.widgets.filter((widget, index, widgets) => widgets.findIndex((other) => other.page === widget.page) === index);
        return <section className={`form-field ${changed ? "form-field-changed" : ""}`} key={field.id}>
          <div className="form-field-heading"><span className="form-field-label" id={`${inputId}-label`}>{field.label}
            {field.required && <span className="form-required">Required</span>}</span>
            {changed && <span className="form-draft-label">Unapplied</span>}</div>
          {field.name && field.name !== field.label && <p className="form-field-name">{field.name}</p>}
          <div className="form-control" aria-invalid={Boolean(fieldError)}>
            <FieldControl field={field} value={value} inputId={inputId} disabled={disabled} describedBy={`${inputId}-help`} invalid={Boolean(fieldError)}
              change={(value) => props.onChange(field.id, value)} />
          </div>
          <div className="form-field-help" id={`${inputId}-help`}>
            {field.reason && <p>{field.reason}</p>}
            {field.kind === "text" && value?.type === "text" && <p>{Array.from(value.text).length} / {field.maxLength.toLocaleString()} characters
              {field.comb ? " · one character per box" : field.multiline ? " · multiline" : ""}</p>}
            {field.password && <p>Masked PDF value, not encryption. Avoid entering secrets in an unprotected PDF.</p>}
            {field.multiple && <p>Use Command/Control to select or clear multiple options.</p>}
            {field.kind === "choice" && value?.type === "choice" && value.indices.some((index) => field.options[index]?.exportValue !== field.options[index]?.label)
              && <p>PDF export: {value.indices.map((index) => field.options[index]?.exportValue).join(", ")}</p>}
            {fieldError && <p className="inline-error" role="alert">{fieldError}</p>}
          </div>
          {pages.length > 0 && <div className="form-page-links" aria-label={`Pages for ${field.label}`}>
            {pages.map((widget) => <button className="text-button" key={widget.page} disabled={props.disabled}
              onClick={() => props.onLocate(widget)} aria-label={`Locate ${field.label} on page ${widget.page + 1}`}>
              <MapPin size={12} />Page {widget.page + 1}</button>)}
          </div>}
        </section>;
      })}
      {Boolean(props.form?.fields.length) && <details className="form-support"><summary>Appearance & support limits</summary>
        {props.form?.warnings.map((warning) => <p key={warning}>{warning}</p>)}</details>}
    </div>
    <div className="form-actions">
      {props.actionError && <p className="inline-error" role="alert">{props.actionError}</p>}
      {props.notice && <p className="form-action-notice" role="status">{props.notice}</p>}
      <p role="status">{props.count ? `${props.count} changed ${props.count === 1 ? "field" : "fields"} not yet in the PDF.` : "No unapplied form changes."}</p>
      <button className="button primary" disabled={props.disabled || props.loading || props.stale || !props.form?.fillable || !props.count} onClick={props.onApply}>
        <Check size={15} />Apply form changes</button>
      <button className="text-button" disabled={props.disabled || !props.count} onClick={props.onDiscard}>Discard unapplied changes</button>
    </div>
  </div>;
}
