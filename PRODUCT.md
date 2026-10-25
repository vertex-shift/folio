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
- User-requested modern polish and subtle animation: refine the existing
  document-first shell with coherent control shapes, cleaner surfaces,
  restrained elevation, and purposeful contextual feedback. Honor Reduce
  Motion and never animate PDF geometry or make the user wait for decoration.
- Preserve the original file by default; expose unsaved changes and failures.
- Existing digital signatures must not be invalidated silently.
- Password-protected PDFs, paragraph reflow, arbitrary font/encoding support, existing-image editing,
  interactive forms, OCR, Office conversion, cryptographic signing, cloud
  collaboration, and full Acrobat parity remain incomplete capabilities.
- New annotations, imported pages, and created/extracted documents must remain
  real, interoperable PDFs after save/reopen. Unsupported forms, signed donors,
  and incompatible page structures are rejected explicitly rather than damaged.
- Unsupported source encodings, vertical/Type 3 fonts, invisible/clipping text,
  ActualText spans, and optional-content layer text must be reported rather
  than guessed. Scans and outlined lettering still require OCR.
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
