# Folio

An independent, macOS-first PDF workspace built with **Tauri, Rust, React,
and PDF.js**. This is an evolving reader and authoring application, not yet
a full Acrobat replacement.
Its familiar PDF-editor layout is independently implemented; no proprietary
code, fonts, icons, artwork, or licensing logic was extracted from Acrobat.

## Start the desktop app

Requires macOS 13+, Xcode Command Line Tools, a current stable Rust toolchain,
and Node.js 22.13+ or 24+. The local build targets the machine's architecture.

```sh
cd folio
npm install
npm run desktop
```

Create a standalone application:

```sh
npm run bundle
open src-tauri/target/release/bundle/macos/Folio.app
```

The generated app needs neither Acrobat nor a separately installed PDF engine.
Local application builds use an ad-hoc signature, not a Developer ID signature
or notarization for public distribution. An Intel or universal build requires the corresponding Rust
target and separate validation; it is not implied by this Apple Silicon build.

Release artifacts intentionally are not stripped. The current macOS 27
dynamic loader rejects stripped procedural-macro libraries with a
`mis-aligned LINKEDIT string pool` error; keeping them intact avoids that
toolchain failure.

## What works

| Workflow | Implementation |
| --- | --- |
| Open PDFs | Native file picker, drag-and-drop, Finder PDF association; one document at a time |
| Read | Continuous vertical scrolling, real PDF rendering, selectable/copyable text, lazy thumbnails, synchronized page navigation, zoom, fit page/width, hand panning |
| Navigate | Document bookmarks and internal links; web/email links require confirmation before opening outside Folio |
| Create | New blank US Letter or A4 PDFs, in portrait or landscape |
| Edit existing text | Replace or delete a specific PDF content-stream text run; keep the original font or explicitly embed Noto Sans |
| Redact | Mark rectangles and export a fresh image-only PDF with permanently blacked-out pixels and no original PDF objects/text layer/attachments/metadata |
| Write | Placed, editable `/FreeText` annotations with embedded Noto Sans; supported Latin, Greek, and Cyrillic text |
| Draw | Standard `/Ink` annotations with selectable color and stroke width |
| Search | Case-insensitive or case-sensitive text and phrases, including across line breaks; result snippets and on-page marks |
| Highlight | Select a passage and choose Highlight, or drag across text with the highlighter |
| Notes | Place a note on the page, enter Unicode text, and revisit it in Comments |
| Annotations | Portable PDF appearance streams; edit note/placed-text contents, remove marks, and undo changes |
| Organize | Move, rotate, delete, duplicate, and insert blank pages; import another PDF after the selected page |
| Extract | Export page numbers/ranges to a separate PDF, in the requested order, without changing the working document |
| Undo/redo | Document changes, including annotations and page operations; saved-state tracking |
| Save | Save an edited copy by default; subsequent Save can update that chosen copy; atomic file replacement |
| Appearance | System, Light, Dark, Sepia, and High Contrast; saved locally with matching native light/dark chrome |
| Safety | Unsaved-change prompts; failed opens preserve the current document; signed PDFs are read-only |

The included three-page quick-start PDF is original example content, generated
by `scripts/prepare-assets.mjs`. It is editable, labeled as an example, and
contains real bookmarks and links for trying the navigation tools.

## Continuous scrolling

Pages are stacked vertically by default. Scroll with the wheel, trackpad,
scrollbar, or Page Up/Down; the page counter and selected thumbnail follow the
visible page. Page buttons, page-number entry, thumbnails, bookmarks, and search
results scroll to their targets without replacing the document with a single
page. Left/Right still navigate by page.

Page dimensions, rotations, and physical scales determine the layout.
Folio prepares geometry with four concurrent metadata requests and renders
only pages around the viewport, releasing off-screen canvases rather than
rendering the entire PDF. Zoom and panel resizing preserve the current page
and reading position. Annotations, source-text edits, and redaction marks are
bound to the page actually clicked, not just the page counter. Highlight a
separate passage on each page; cross-page copy selection is not a multi-page
highlight operation.

## Editing existing PDF text

Choose **Write** or press **E**, then click existing text on the page. The
**Edit existing text** panel also lists the page's actual text-show operations.
Choose a source run from this list when a PDF splits text differently or a
click cannot distinguish repeated/overlapping content safely.

Edit the replacement and choose **Replace text**. The original font, size,
position, color, and surrounding drawing instructions are preserved when the
original character encoding can represent the replacement. If a subset font
does not map a new character, the editor reports it without changing the PDF
or losing your draft. Select **Noto Sans (embedded)** to explicitly change this
run's typeface and embed supported Latin, Greek, or Cyrillic glyphs.

This **rewrites PDF page content**, not an annotation, white rectangle, or
new text drawn over the original. The replacement becomes selectable and
searchable page text after saving/reopening. Clear the field to remove the
run. Undo/redo restores the actual document contents. Shared streams and
nested Form XObjects are copied on write, so one occurrence can be changed
without changing another page or invocation.

This is run-level editing, not a paragraph layout engine. Longer text can
extend beyond the original run; it does not reflow neighboring lines. Replacing
a `TJ` array rebuilds its text and does not retain per-glyph numeric kerning.
Review the result before saving an edited copy. Text replacement is **not
secure redaction** or document sanitization.

Supported sources include standard single-byte fonts/encoding Differences
and horizontal Identity-H fonts with supported Unicode maps. Complex Unicode
maps, vertical/Type 3 fonts, ActualText substitution spans, optional-content
layers, complex-script/combining-mark shaping, invisible OCR text, and clipping
text are reported as unsupported. Noto Sans normalizes supported precomposed
Latin/Greek accents before embedding.
Image-only pages and outlined lettering still require OCR. Signed PDFs remain
read-only.
Editing is bounded per page to 16 MiB of decoded content, 200,000 drawing
operations, 10,000 text runs, 128 distinct fonts, and 200,000 font mappings.
Pages outside these limits remain readable but are refused by the editor.

## Permanent redaction

Choose **Redact a PDF** in All tools, use the floating eraser tool, or press
**R**. Drag rectangles over sensitive content, leaving room around the full
letters or image. A keyboard-accessible **Add an area by coordinates** form
uses unrotated PDF points. Areas can span multiple pages and are listed in
the Redact panel; remove individual areas or explicitly clear all marks.

**Red marks are pending review UI, not applied redactions.** Choose **Export
redacted copy**, review the whole-document rasterization warning, choose
150 or 300 DPI, acknowledge the tradeoff, and export. Folio renders every page,
replaces marked pixels with opaque black, and builds a **new image-only PDF**
in Rust. The Rust engine independently burns the regions after bounded PNG
decoding; it does not trust a frontend covering rectangle or copy original
streams beneath an overlay. RGB data is losslessly compressed, including an
outward-rounded, at-least-one-pixel safety margin. Transparent pixel channels
are composited onto white, not preserved as hidden RGB/alpha data.

No original PDF objects, text layer, fonts, annotations, attachments, forms,
bookmarks, accessibility structure, or source metadata are copied. Visible
page/annotation appearances outside marked regions become pixels. UI-only
note markers and comment bodies are not exported. The result loses text
search, selection, interactivity, and vector quality on **all pages**. This
is a conservative raster redaction path, not content-preserving vector
redaction or OCR. Only marked areas are blacked out; check other occurrences
on every page. Open and inspect the actual exported PDF before sharing it.

The original file, working PDF, unsaved changes, and undo history remain
unchanged. Native export refuses to overwrite the active original or saved
copy, including symlink aliases, and suggests the neutral filename
**Redacted.pdf**. The window still shows the **original** after export:
pending marks remain, and normal Save/download/keyboard/native-menu paths
continue to open redaction review rather than leaking an unredacted copy.
Source edits, page operations, and undo/redo are locked while marks exist.
Leaving or clearing marks requires an explicit decision; cancellation and
failure retain them. Unapplied marks are session-only, not autosaved.

Exports are revision-checked, processed one page at a time, cancellable during
rendering, and limited to **500 areas**, **24 megapixels / 16,384 pixels per
page dimension**, **64 MiB per PNG**, and **128 MiB output**. Oversized pages
are refused; resolution is never lowered silently. Signed PDFs and XFA forms
are not supported by this export path. A failed or incomplete export does
not produce a success-shaped partial PDF.

## Writing and page workflows

Use **Write** to place text or draw on a page. Text size is 6-96 PDF points;
pen width is 0.5-12 points. The writing color applies to new text and strokes,
independently of the workspace theme. Placed text remains upright when added
to a rotated page and uses an embedded, OFL-licensed Noto Sans font, so the
saved appearance does not depend on the next reader's installed fonts.

Placed text is a **FreeText annotation**, not a replacement or reflow of the
original content stream. Edit or remove it in **Comments**. Text boxes are
created at the clicked location, with a bounded default size; text wraps to
the box, and overflow or unsupported scripts are rejected without losing the
draft. Notes continue to support arbitrary Unicode text. Find in document
searches the PDF page text layer, not annotation contents.

Use **Organize** to insert a blank page, duplicate the selected page, or
insert another PDF. Each operation is one undoable change. Duplicated
annotations are independent of the originals. Import preserves page
resources, inherited dimensions/rotations, and supported links; imported
named destinations are resolved before the donor catalog is discarded.
The donor's document-level JavaScript and bookmarks are not imported.

**Extract pages** accepts ranges such as `1-3, 5`, or an explicit order such
as `3, 1`. Invalid ranges and repeated pages are rejected. Export does not
alter the document, its saved state, or its undo history. Native extraction
refuses to overwrite the active PDF. Extraction is not sanitization.

## Appearance

Use the **Appearance** palette button in the title bar. The default **System**
setting follows the device's light/dark preference, including changes while
Folio is open. Explicit Light, Dark, Sepia, and High Contrast choices override
the system setting and persist across launches in local application storage.

Themes cover the workspace, tools, search/comments, page organization, dialogs,
notifications, scrollbars, focus indicators, and native application chrome.
Sepia uses native light chrome; Dark and High Contrast use native dark chrome.
The palette picker supports keyboard navigation and Escape.

PDF pages, thumbnails, annotation colors, and exported document bytes retain
their original colors. This is workspace theming, not document recoloring or
an inversion filter. If preference storage is unavailable, the current session
still changes appearance and an explicit message reports that it was not saved.

Palette values live in `src/themes.css`; `src/styles.css` consumes semantic
roles. Preference validation and system/native mappings live in `src/lib/themes.ts`.

The interface uses shared rounded-control/elevation tokens, tonal mode
selection, and restrained contextual motion. Panels reveal when opened or
changed; controls provide quick hover/press feedback without moving their
hit targets. Dialogs remain opaque during entry, and PDF canvases never
animate. The device's **Reduce Motion** preference removes spatial effects
and smooth search scrolling while retaining gentle color feedback. Theme
changes switch text and surfaces atomically rather than fading through
unreadable intermediate colors.

## Important boundaries

- **Not full Acrobat parity.** Run-level source-text replacement is supported,
  but not paragraph reflow, existing-image editing, OCR, Office conversion, PDF creation from arbitrary inputs,
  cryptographic signing,
  certificate validation, cloud storage, collaboration, AI assistant,
  accessibility remediation, preflight, or print-production workflows.
- Password-protected PDFs are rejected rather than silently decrypted or
  rewritten. Existing signatures are detected conservatively and force
  read-only mode; they are **not cryptographically verified**.
- Interactive forms are rendered but cannot yet be filled or authored.
  Deleting, duplicating, importing, or extracting pages in PDFs with form
  fields or XFA is refused. Duplicate pages with popup/reply annotations
  and imported documents with optional-content layers also require a
  flattened copy.
- Signed PDFs remain read-only, including for extraction and import as a
  donor. Folio does not claim that extracted pages retain a document signature.
- Bookmarks are read-only navigation. Link targets that are unresolved,
  malformed, use embedded credentials, or are not HTTP, HTTPS, or mailto
  are reported rather than executed.
- Edits use a full PDF rewrite, not an incremental update. Work on copies.
  Complex portfolios, tagged-PDF structure, uncommon annotations, and advanced
  page-label/destination semantics need broader compatibility work.
- Maximum input/edited file size: **128 MiB**. Search displays the first
  **500 matches** and explicitly indicates when more exist.
- Undo keeps up to **20 snapshots within a 64 MiB history budget**. Older
  snapshots are evicted; a document larger than the budget may have no undo
  snapshot. History is in memory only.
- No autosave or crash recovery. Only committed annotations and page edits
  belong to the document; save a copy before quitting.
- Page deletion, text replacement, ordinary annotations, and extraction are
  **not secure redaction**. Use the dedicated redacted-copy export for marked
  regions, accept its image-only limitations, and verify the exported file.

## Browser preview

The development preview uses the **same Rust document engine**, not a
JavaScript substitute. Start these in separate terminals:

```sh
npm run preview:backend
```

```sh
npm run dev
```

Open <http://127.0.0.1:1420>. The interface identifies itself as Browser
preview. Open uses a browser file picker; Download PDF exports a copy.
Downloading does not pretend that a local file was overwritten or clear
the working copy's unsaved state.

The development bridge binds only `127.0.0.1:19148`, accepts PDF bytes rather
than filesystem paths, restricts browser origins, and is excluded from the
default native build. Do not expose the Vite server or preview bridge to a
network.

For a second, independent browser preview, set `FOLIO_DEV_PORT=1421` and
`FOLIO_PREVIEW_PORT=19149` for **both** commands above. The interface then uses
`http://127.0.0.1:1421`; the engine still binds only loopback and validates the
matching browser origin. Defaults remain unchanged. This lets a new engine
run without restarting a preview that has unsaved work. These overrides apply
to browser preview; Tauri's default development URL remains port 1420.

Embedded previews can themselves run inside a Tauri-based host. Folio checks
the application identifier before enabling native commands; a host's bridge
is not treated as Folio's bridge. Theme changes in the preview therefore
affect the preview only, not the host application's appearance.

## Architecture

```text
React / TypeScript workspace
  |  Tauri IPC (native) or loopback HTTP (development only)
  v
Rust Workspace: document identity, revision guards, unsaved-change protection
  v
Rust Session / lopdf: annotations, page tree, snapshots, atomic save
  |
  +-- PDF bytes --> PDF.js worker --> canvas + selectable text layer
```

`src-tauri/src/document.rs` owns PDF mutations and serialization.
`workspace.rs` provides the same revision-checked operations to both
`desktop.rs` and the development-only `bin/preview.rs`. `pages.rs` implements
page creation/import/extraction/duplication; `writing.rs` generates standard
annotation appearances and embedded font maps.
`text_edit.rs` indexes and replaces existing text-show operations, including
nested Form invocations, with strict font decoding and isolated resource copies.
`redaction.rs` plans crop/rotation/UserUnit-aware pixel masks, decodes bounded
page PNGs, burns redactions, and constructs fresh image-only PDFs. The renderer
submits one page at a time; original PDF objects never enter the output builder.

Reordering first materializes inherited resources, page boxes, and rotations
before rebuilding the page tree. Deletion prunes unreachable objects and
retargets standard explicit destinations to a surviving neighbor. Highlight
coordinates are converted from actual browser text selections to PDF space,
including on rotated pages; a PDF appearance stream makes the mark portable.

The production interface loads bundled resources only. PDF.js fonts, CMaps,
ICC profiles, and WebAssembly codecs ship locally. No document contents are
sent to Adobe or an external service.

## Installed Acrobat analysis

Read-only inspection of
`/Applications/Adobe Acrobat DC/Adobe Acrobat.app` identified:

- Acrobat Pro **26.001.21563**, approximately **1.6 GB**.
- A universal Mach-O executable containing **arm64 and x86_64**.
- Native PDF/text/color components including `AdobePDFL`, `AdobeCoolType`,
  and `AdobeACE`, plus Chromium Embedded Framework and helper processes.
- Plug-ins named `Comments`, `Search`, `AcroForm`, `EScript`, `DigSig`,
  `PPKLite`, `Accessibility`, `MakeAccessible`, `Reflow`, `ReadOutLoud`,
  `StorageConnectors`, and `DropboxStorage`, among others.

These names establish component presence, not their complete capabilities
or internal APIs. No disassembly, decompilation, resource extraction, Adobe
service impersonation, or license bypass was performed.

Adobe's [public workspace documentation](https://helpx.adobe.com/acrobat/desktop/get-started/learn-the-basics/workspace.html)
describes document tabs, global tools, quick actions, thumbnails, comments,
and zoom. Folio uses those general interaction conventions with its own
identity. Adobe's signing, conversion, cloud, accessibility, and production
systems remain separate engineering efforts, not hidden features of this MVP.

## Development checks

```sh
npm test
npm run build
npm run test:rust
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Tests cover appearance preference validation, persistence errors, system/native
theme mappings, standard and rotated selection coordinates, phrase search,
Unicode annotation persistence, annotation removal, inherited page properties,
page deletion, destination retargeting, save/reopen, source preservation,
signature read-only handling, undo/redo, stale revisions, and failed opens.
Additional authoring coverage checks new-document dirty state, embedded text
appearances, Unicode normalization and supported scripts, overflow rejection,
rotated text matrices, independent annotation copies, imported links and
resources, safe extraction, form/signature guards, and page-range/URL parsing.
Existing-text coverage includes all four text-show operators, repeated text,
shared page streams and nested Forms, original and explicit embedded fonts,
Unicode/Differences encodings, invisible/ActualText guards, deletion,
save/reopen, undo/redo, unambiguous click targeting, and search cache invalidation.
Redaction coverage checks every exported RGB pixel against masks, original
byte/history preservation, removal of source objects and PNG metadata, alpha
channel disposal, rotations/crop offsets/UserUnit, save/reopen, incomplete and
stale exports, cancellation/generation guards, signature/XFA guards, coordinate
validation, source overwrite protection, and exact pixel limits.

The blue app mark is authored in `public/folio.svg`; native icons are generated
with `npm run tauri -- icon public/folio.svg`. PDF.js includes independently
licensed open-source font/CMap/profile data. Third-party dependencies retain
their own licenses; review licensing, signing, and document compatibility
before distributing a production release.

The unmodified Noto Sans font is from the
[Noto fonts project](https://github.com/notofonts/noto-fonts/tree/main/hinted/ttf/NotoSans).
Its copyright and SIL Open Font License are in `src-tauri/resources/fonts/OFL.txt`
and included in the application's third-party notices.
