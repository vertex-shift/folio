# Folio

<!-- impeccable:product-schema 1 -->

## Platform

web

The HTML/CSS interface runs inside a macOS-first Tauri desktop application.
This is not a hosted service or a mobile application.

## Stack

User-selected: Tauri, with a Rust backend and an HTML/CSS interface.
Implementation: React and TypeScript, Vite, PDF.js for rendering and text
selection, and lopdf for Rust PDF mutations. The application is not entirely
Rust: its interface and renderer use JavaScript.

## Product Purpose

Independently implement familiar desktop PDF workflows, starting with the
user-confirmed milestone of viewing, search, annotations, and page organization.
Folio is a working name, not an Adobe product or an Acrobat distribution.

## Operating Context

Open local PDFs, read and search their text, add highlights and notes, reorder
and rotate pages, remove pages, undo changes, and save an edited PDF.
The desktop app processes document contents locally.

## Capabilities and Constraints

- MVP: PDF viewing, zoom, navigation, thumbnails, text search, highlights,
  sticky notes, page reordering, rotation, deletion, undo/redo, and saving copies.
- Continuous reading: vertically stacked pages with viewport-bounded rendering.
  Synchronize navigation/search/bookmarks with the visible page; keep writing
  and redaction targets tied to the actual page surface. Preserve reading
  position across zoom, resizing, and same-document updates.
- Multiple PDF tabs: new opens add documents without discarding existing work.
  Support multi-selection and dropped/Finder batches, independent password
  prompts, and explicit partial-failure reporting. Preserve per-tab edits,
  history, navigation/search/zoom/tools, and pending marks. Render only the
  active tab's canvases. Guard closing each tab and quitting with any inactive
  unsaved work; refuse overwriting another tab's open file.
- Authoring milestone: Letter/A4 blank PDFs; editable placed-text and pen
  annotations; duplicate/blank/imported pages; extraction by page range;
  bookmark and internal-link navigation; confirmed external web/email links.
  Placed text uses an embedded Noto Sans font for Latin, Greek, and Cyrillic.
  Placed annotations do not themselves replace existing page text.
- Existing-text editing: replace or delete an exact content-stream text run,
  keeping its original font when encodable or explicitly embedding Noto Sans.
  Index nested Form invocations and isolate shared streams/resources so only
  the selected occurrence changes. Replacements must be searchable, selectable,
  undoable, and persistent after save/reopen, not drawn over old text.
- Page authoring: user-selected PNG/JPEG image stamps with physical placement,
  size, opacity and real alpha; bounded decoding and metadata-free image objects.
  Crop the actual visible CropBox non-destructively, with per-page geometry,
  full-MediaBox and original-crop reset across save/reopen. Center configurable
  single-line text watermarks and position sequential numbers on explicit page
  ranges. Watermarks/numbers are removable annotations, not security or dynamic
  page labels. Preserve document-scoped settings/placement drafts across tab
  switches and virtualization; guard ordinary save, close, quit, and mutations.
  Cropping is not permanent redaction, and existing-image editing remains out
  of scope. Keep cropped rendering, overlays, writing, thumbnails, and redacted
  exports on a shared MediaBox/CropBox/rotation/UserUnit geometry contract.
- Existing AcroForm filling: inspect bounded, inherited field trees and qualified
  names; fill supported text/multiline/comb/password, checkbox, radio, list, and
  combo fields. Preserve export/display distinctions, repeated widgets, read-only
  flags, constraints, encryption, and undo/redo. Apply values and portable
  appearances together, not on every keystroke. Keep drafts at workspace scope
  through tab switches and page virtualization; protect save, close, and quit
  from silently exporting older values. Mask password-type fields without
  implying file encryption. Signature fields stay non-fillable here; scripts,
  actions, calculations, XFA, and signed/certified forms are not executed/rewritten.
- Redaction: mark page rectangles and export a separate, fresh image-only PDF.
  Permanently replace marked pixels with opaque black before lossless image
  encoding; copy no original PDF objects, text layer, attachments, or metadata.
  Explain that all pages are rasterized and search/selection/accessibility
  structure/interactivity are lost. Preserve the original and its unsaved
  work. Pending marks must block accidental ordinary saving and stale edits.
- Appearance: user-requested dark mode and additional themes, implemented as
  System, Light, Dark, Sepia, and High Contrast. Persist the preference locally;
  follow live device changes in System mode and match native light/dark chrome.
  Theme the workspace without changing PDF content or annotation colors.
- Offline OCR: bundled, checksum-pinned English Tesseract/WASM recognition on
  selected page ranges, with explicit skip/force policy for existing text.
  Stage sequential word boxes/baselines against a document ID/revision, then
  add real invisible, Unicode-mapped PDF text in one undoable mutation.
  Preserve original page streams/pixels, CropBox/rotation/UserUnit coordinates,
  encryption, unsaved work, and other tabs. Exclude annotation/form appearances
  and refuse pending drafts/redactions. Cancel/release resources without partial
  commits; report skipped/no-text/error outcomes. No runtime downloads, cloud
  recognition, system Tesseract prerequisite, or accessibility-conformance claim.
- Password-protected PDFs: prompt for supported standard PDF passwords in
  native and preview opening flows. Preserve existing encryption, passwords,
  and permissions in saves, history, extraction, and redacted exports.
  Wrong passwords/cancellation preserve the current document. Restricted
  User access is read-only for general editing unless the owner password is
  supplied; a separate `FILLABLE` gate permits supported form-only mutations
  (`ANNOTABLE` for revision-2 encryption), never arbitrary content changes. Keep
  passwords in memory only and never export the decrypted rendering copy.
  Encrypted redaction retains required encryption parameters and, for legacy
  algorithms, the file identifier; disclose this exception to metadata removal.
- Document management: editable descriptive metadata with consistent classic
  Info/supported existing XMP; real outline create/rename/reorder/nesting/delete;
  bounded document-level embedded-file attachment inspection/add/removal/export.
  Keep edits in Session history, retain per-tab drafts, preserve unrelated
  structures, and never describe metadata changes/removal as secure erasure.
  Attachments are untrusted data, never automatically opened or executed.
- Protection authoring: explicit separate protected/reprotected/unprotected PDF
  copies, using supported AES-256 revision-6 PDF 2.0 encryption and fresh OS
  randomness. Actual owner authorization is required to replace/remove existing
  protection, not inferred from permissive opening-password access. Keep source
  bytes/history/protection unchanged, block pending drafts/marks, guard all open
  file paths, and label permission flags as cooperative rather than absolute DRM.
- Output tools: print full documents or selected ranges from a bounded,
  isolated raster snapshot, independent of virtualized pages. Export real
  PNG/JPEG page pixels or selectable UTF-8 text; multiple images use one ZIP.
  Honor print/high-quality-print/copy permissions independently of editing,
  including owner access. Explicitly disclose unencrypted protected-source
  image/text exports. Preserve print resources until the dialog/job is finished.
- Document comparison: compare two open, copy-permitted working snapshots,
  including unsaved applied edits, without source/history/navigation changes.
  Keep selection/settings/results at workspace level. Match by page index,
  expose real extracted-text spans, rendered RGB pixel counts/masks at explicit
  DPI/tolerance, geometry changes and unmatched trailing pages, with readable
  before/after and overlay views. Guard both inputs' identity/revision and all
  pending drafts; invalidate on edits/undo, drafts and closing. Permit signed
  read-only comparison when copy/extraction is allowed. Bound pages, pixels,
  text/tokens/spans, preview/report bytes and concurrency; support cancellation
  and complete renderer/worker/canvas/URL cleanup. Separate JSON report export
  explicitly warns that protected-source extracted content is unencrypted.
  Never equate matching compared content with identical documents: metadata,
  hidden objects, attachments and signatures/trust are outside this method.
  No semantic/reflow, moved-page matching, OCR or cloud processing is implied.
- Lossless optimization: measure safe unfiltered-stream compression and
  unreachable-object cleanup, then save a separate protected PDF copy.
  Report actual before/after bytes and no-reduction outcomes; never rewrite
  signed sources, silently downsample images, claim PDF/A/linearization or
  guarantee a compression ratio. Output cancellation/failure preserves all
  tabs, source bytes, saved state and history. Unapplied drafts/areas block
  ordinary output, and native output copies never clobber existing paths.
- User-requested modern polish and subtle animation: refine the existing
  document-first shell with coherent control shapes, cleaner surfaces,
  restrained elevation, and purposeful contextual feedback. Honor Reduce
  Motion and never animate PDF geometry or make the user wait for decoration.
- Expanded modern motion: a gliding shared tab-selection surface, coordinated
  mode/tool selection, masked panel reveals, opaque depth-based dialogs,
  lightly sequenced appearance choices, and meaningful press/success feedback.
  Keep routine movement below 300ms, support interruption and live Reduce
  Motion, suppress fast busy flashes, and add no motion dependency.
- Preserve the original file by default; expose unsaved changes and failures.
- Existing digital signatures must not be invalidated silently.
- Cryptographic signatures: inspect the supported detached CMS subset locally,
  with strict ByteRange/Contents/revision checks. Keep signed-byte integrity,
  later changes, certificate dates, and unchecked identity trust distinct.
  Export separately signed copies of unsigned, unencrypted sources using local
  PKCS#12 identities and SHA-256/RSA or supported ECDSA. Include current working edits, preserve
  source/history, guard all open-tab output paths, and block pending drafts.
  Keep keys/passwords in memory only. Reject unsupported structures, encryption,
  certification/co-signing, algorithms, and timestamp/countersignature formats.
  Do not claim trusted chains, revocation, trusted timestamps, qualified
  signatures, or PAdES-LTV; make no external certificate/document requests.
- Certificate-based protection, paragraph reflow, arbitrary font/encoding support, existing-image editing,
  form authoring, unsupported interactive forms, non-English OCR, Office conversion, advanced
  signing/trust validation, cloud
  collaboration, and full Acrobat parity remain incomplete capabilities.
- New annotations, imported pages, and created/extracted documents must remain
  real, interoperable PDFs after save/reopen. Unsupported forms, signed donors,
  and incompatible page structures are rejected explicitly rather than damaged.
- Unsupported source encodings, vertical/Type 3 fonts, invisible/clipping text,
  ActualText spans, and optional-content layer text must be reported rather
  than guessed. Scans and outlined lettering can use the bundled English OCR.
- Do not copy Adobe code, fonts, icons, artwork, branding, or licensing logic.
- A development-only browser bridge exercises the same Rust document engine.
  It must be clearly distinguished from the native desktop app.

## Brand Commitments

The user requested an Acrobat-familiar look and feel. Preserve recognizable
PDF-editor conventions: a document bar, tool navigation, a paper canvas,
page thumbnails, and contextual review controls. Use independent branding
and independently implemented components, not a pixel-identical branded copy.

## Evidence on Hand

Read-only inspection of `/Applications/Adobe Acrobat DC/Adobe Acrobat.app`
identified Acrobat Pro 26.001.21563, a 1.6 GB universal macOS bundle, CEF,
and plug-ins for comments, forms, signatures, search, accessibility, scripting,
and storage. Component names are evidence of presence, not proof of behavior.
Adobe's public workspace documentation identifies the global tool bar,
document tabs, quick actions, comments, bookmarks, thumbnails, and zoom.
No Adobe executable, source, or artwork is included in this project.

## Product Principles

- Familiar workflows without proprietary implementation or branding.
- A visible PDF change must survive saving and reopening.
- Protect source documents and show meaningful errors.
- Distinguish the delivered MVP from the full-parity long-term goal.
