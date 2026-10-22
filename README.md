# Folio

An independent, macOS-first PDF workspace built with **Tauri, Rust, React,
and PDF.js**. This is the agreed working MVP, not a full Acrobat clone.
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
| Read | Real PDF rendering, selectable/copyable text, lazy thumbnails, page navigation, zoom, fit page/width, hand panning |
| Search | Case-insensitive or case-sensitive text and phrases, including across line breaks; result snippets and on-page marks |
| Highlight | Select a passage and choose Highlight, or drag across text with the highlighter |
| Notes | Place a note on the page, enter Unicode text, and revisit it in Comments |
| Annotations | Saved inside the PDF as standard `/Highlight` and `/Text` annotations, not an external sidecar |
| Organize | Drag pages or use accessible move buttons; rotate and remove pages |
| Undo/redo | Document changes, including annotations and page operations; saved-state tracking |
| Save | Save an edited copy by default; subsequent Save can update that chosen copy; atomic file replacement |
| Appearance | System, Light, Dark, Sepia, and High Contrast; saved locally with matching native light/dark chrome |
| Safety | Unsaved-change prompts; failed opens preserve the current document; signed PDFs are read-only |

The included three-page quick-start PDF is original example content, generated
by `scripts/prepare-assets.mjs`. It is editable and labeled as an example.

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

## Important boundaries

- **Not full Acrobat parity.** No PDF text/content editing, OCR, Office
  conversion, PDF creation from arbitrary inputs, cryptographic signing,
  certificate validation, cloud storage, collaboration, AI assistant,
  accessibility remediation, preflight, or print-production workflows.
- Password-protected PDFs are rejected rather than silently decrypted or
  rewritten. Existing signatures are detected conservatively and force
  read-only mode; they are **not cryptographically verified**.
- Interactive forms and document hyperlinks are rendered but are not
  interactive. Removing pages from PDFs with form fields or XFA is refused.
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
- Page deletion is **not secure redaction**. Do not use this MVP to sanitize
  confidential PDFs or remove all recoverable information.

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
`desktop.rs` and the development-only `bin/preview.rs`.

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

The blue app mark is authored in `public/folio.svg`; native icons are generated
with `npm run tauri -- icon public/folio.svg`. PDF.js includes independently
licensed open-source font/CMap/profile data. Third-party dependencies retain
their own licenses; review licensing, signing, and document compatibility
before distributing a production release.
