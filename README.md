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
PDF signature operations use a statically bundled OpenSSL 3 library, not an
external `openssl` executable. Building that dependency requires a C compiler,
Perl, and make on Unix; Windows MSVC builds need Visual Studio C++ tools,
Windows-native Perl (such as Strawberry Perl), and nmake. CI selects
Strawberry Perl on Windows and installs Perl on Linux.
Local application builds use an ad-hoc signature, not a Developer ID signature
or notarization for public distribution. An Intel or universal build requires the corresponding Rust
target and separate validation; it is not implied by this Apple Silicon build.

Release artifacts intentionally are not stripped. The current macOS 27
dynamic loader rejects stripped procedural-macro libraries with a
`mis-aligned LINKEDIT string pool` error; keeping them intact avoids that
toolchain failure.

## Automated desktop builds and releases

The [Desktop builds and releases workflow](.github/workflows/build.yml) runs
on pull requests, pushes to `main`, and version tags, and can be started manually
from GitHub's **Actions** tab. Each platform runs the frontend and Rust
document-engine tests, then builds the production frontend and native
application with the committed lockfiles.

| Artifact | Platform | Application bundles |
| --- | --- | --- |
| `folio-windows-x64` | Windows x64 | NSIS setup executable (`.exe`) |
| `folio-macos-arm64` | macOS Apple Silicon | Disk image containing `Folio.app` (`.dmg`) |
| `folio-linux-x64` | Linux x64, built on Ubuntu 22.04 | Debian package (`.deb`) and AppImage |

For tagged releases, download the platform-labeled installers from
[GitHub Releases](https://github.com/vertex-shift/folio/releases).
Each release includes `SHA256SUMS` for verifying the downloaded files.
Build-only artifacts are also available from successful workflow runs and are
retained for 14 days. After downloading a Linux AppImage, mark it executable
with `chmod +x Folio_*.AppImage` before launching it. Linux also requires the
distribution's WebKitGTK 4.1 runtime and, for AppImages, FUSE support or
AppImage's extraction mode.

These are not production-signed releases. Windows bundles
are unsigned, macOS bundles are ad-hoc signed and not notarized, and Linux
packages are unsigned. No signing secrets are configured.
Intel macOS and ARM Windows/Linux builds are not included.
The existing `npm run bundle` command remains a macOS `.app`-only local build;
CI explicitly selects each platform's installer formats.

### Publishing a version

Keep the version in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`,
`src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json` synchronized, and commit
those changes to `main`. Push a matching version tag, for example:

```sh
git tag -a v0.1.1 -m "Folio 0.1.1"
git push origin v0.1.1
```

The tag must match the application version exactly. All three platform builds
must succeed before publishing. The workflow creates a draft, uploads all four
installers and their checksums, then publishes the complete release; tags with
a prerelease suffix, such as `v0.2.0-beta.1`, produce GitHub prereleases.
Pull requests, branch pushes, and manual runs do not publish releases.
Published releases are not overwritten on reruns. If publishing fails after
creating a draft, inspect and remove that unpublished draft without deleting
the tag before rerunning the workflow.

## What works

| Workflow | Implementation |
| --- | --- |
| Open PDFs | Multiple PDFs in independent tabs through the native picker, drag-and-drop, Finder association, or browser picker |
| Protected PDFs | Password prompt for supported standard RC4/AES PDFs; preserve passwords and permissions on PDF copies and PDF exports |
| Compare PDFs | Two open working snapshots; page-index embedded-text, rendered-pixel and geometry differences; before/after or overlay review and a separate JSON report |
| Digital signatures | Detached-CMS integrity inspection, explicit custom-CA chain checks, and separate incremental signed/co-signed copies with local PKCS#12 identities, including selected empty signature fields |
| Read | Continuous vertical scrolling, real PDF rendering, selectable/copyable text, lazy thumbnails, synchronized page navigation, zoom, fit page/width, hand panning |
| Navigate | Create, rename, nest, reorder, and delete real PDF bookmarks; internal links; confirmed external web/email links |
| Document properties | Inspect/edit title, author, subject, and keywords; synchronize supported existing XMP with classic Info metadata |
| Attachments | Inspect, embed, remove, and explicitly save bounded document-level embedded files without opening them |
| Protection copies | Separate AES-256 revision-6 protected/reprotected or unprotected copies, with owner authorization and explicit permissions |
| Create | New blank PDFs, or ordered local PNG/JPEG and UTF-8 text files as independent PDFs; Letter/A4, orientation, margins and text-size controls |
| Edit existing text | Replace or delete a specific PDF content-stream text run; keep the original font or explicitly embed Noto Sans |
| Redact | Mark rectangles and export a fresh image-only PDF with permanently blacked-out pixels and no original document content, attachments, or descriptive metadata; retain existing encryption when protected |
| Write | Placed, editable `/FreeText` annotations with embedded Noto Sans; supported Latin, Greek, and Cyrillic text |
| Draw | Standard `/Ink` annotations with selectable color and stroke width |
| Insert images | User-selected PNG/JPEG image stamps with real RGB/alpha appearance streams, placement, size, and opacity controls |
| Edit existing images | Inspect/select actual page-content Image-XObject occurrences; replace, remove, or move/resize the supported occurrence without changing shared images elsewhere |
| Crop | Non-destructive visible-page CropBox cropping by physical margins or dragging; show full MediaBox or restore the original crop |
| Watermark | Configurable, embedded-font text watermark on current, all, or explicitly chosen pages |
| Number pages | Persistent positioned numbers, with starting number, prefix/suffix, and explicit page-range order |
| Search | Case-insensitive or case-sensitive text and phrases, including across line breaks; result snippets and on-page marks |
| Offline OCR | Bundled English recognition on selected pages/ranges; add persistent invisible PDF text without replacing original images or page streams |
| Highlight | Select a passage and choose Highlight, or drag across text with the highlighter |
| Notes | Place a note on the page, enter Unicode text, and revisit it in Comments |
| Annotations | Portable PDF appearance streams; edit note/placed-text contents, remove marks, and undo changes |
| Fill forms | Inspect and fill supported existing AcroForm text, multiline/comb/password, checkbox, radio, list, and combo fields; apply values and embedded appearances together |
| Design forms | Create supported AcroForm controls and unsigned signature slots; edit field properties/defaults and place, resize or remove page widgets |
| Static form copies | Separate PDF copies with real page appearances and interactive widgets removed; source/history and encryption preserved |
| Organize | Move, rotate, delete, duplicate, and insert blank pages; import another PDF after the selected page |
| Extract | Export page numbers/ranges to a separate PDF, in the requested order, without changing the working document |
| Print | Prepare all selected pages independently of virtualization, then open the platform/browser print dialog; explicit raster quality and permission checks |
| Export images/text | PNG or JPEG at 96/150/300 DPI; multiple images in an ordered ZIP; selectable page text as UTF-8 |
| Export Word content | Real editable-text DOCX from all/selected PDF pages in requested order; document content, not high-fidelity Word layout |
| Optimize a copy | Lossless compression of eligible unfiltered streams and unreachable-object cleanup; measured byte counts, protected PDF copies, honest no-reduction outcomes |
| Undo/redo | Document changes, including annotations and page operations; saved-state tracking |
| Save | Save an edited copy by default; subsequent Save can update that chosen copy; atomic file replacement |
| Appearance | System, Light, Dark, Sepia, and High Contrast; saved locally with matching native light/dark chrome |
| Safety | Unsaved-change prompts; failed opens preserve the current document; signed PDFs are read-only |

The included three-page quick-start PDF is original example content, generated
by `scripts/prepare-assets.mjs`. It is editable, labeled as an example, and
contains real bookmarks and links for trying the navigation tools.

## Multiple PDF tabs

Use **+**, **Open PDF**, or **Command+O** (**Control+O** on Windows/Linux) to
add documents without replacing the PDFs already open. The file picker and
drag-and-drop accept multiple PDFs; Finder and command-line opening also
accept multiple local PDF paths. A failed file is reported separately without
discarding successfully opened tabs. Protected files prompt for passwords
one at a time; canceling one does not cancel the others.

Each tab retains its own document bytes, edits, undo/redo, page and reading
position, zoom, search, tools, pending redaction marks, unapplied form values and form-design drafts. Only the active
tab renders page canvases and thumbnails. Use the tab strip's arrow keys,
Home/End, or **Control+Tab / Control+Shift+Tab** to move between documents.

Close a tab with its **X** or **Command/Control+W**. Unsaved edits and pending
marks require an explicit decision before that tab closes; unapplied page-authoring
and form-design drafts must first be applied or discarded. Other PDFs remain
open. Closing the last tab leaves an empty workspace. Quitting checks every
tab, including inactive ones, and lets you review unsaved documents or
explicitly discard all of them. Saving/exporting cannot overwrite a file
that is open in another tab.

Tabs and view state are in-memory, not an autosave or restart-recovery system.
Save copies of important work before quitting.

Ordinary saves, PDF downloads, and page extraction also check pending edits in
the document engine, not only in the interface. Apply or discard those drafts
first. Pending redactions require the dedicated **Export redacted copy**
workflow; viewing the current document remains available.

## Creating PDFs from local files

Choose **Create** in the title bar, **Create PDF from files** in All tools,
or **Command/Control+Shift+N**. Select still PNG/JPEG images or strict UTF-8
text files, reorder them with the named Up/Down buttons, choose Letter/A4,
portrait/landscape and a margin, and prepare the complete batch. **Open new
PDF** then adds an independent, unsaved tab; it never replaces a source tab,
clears its drafts, changes its history, or writes an input file.

Images become real page content, one centered aspect-preserving image per
page, with decoded EXIF orientation and a real PNG transparency soft mask.
Only sanitized RGB/alpha image resources enter the PDF, not source metadata.
Text uses embedded Noto Sans and real Unicode mappings. It wraps and paginates
without truncation; every file starts a new page. Latin, Greek and Cyrillic
within the bundled font's existing supported range are accepted; other glyphs
are explicitly refused. A leading UTF-8 BOM is removed, CRLF/CR become LF,
and tabs become four spaces. Invalid UTF-8, unsupported controls/scripts and
empty text files fail the entire batch rather than inserting replacements.
No OCR, rich-text or Office-file import is performed.

| Creation boundary | Limit |
| --- | --- |
| Files / output | 1-50 files, 500 pages, 128 MiB PDF |
| Image input | 16 MiB/file, 64 MiB/batch, 24 megapixels and 8,192 px/side per image |
| Decoded image budget | 128 MiB RGB/alpha across the batch |
| UTF-8 input | 1 MiB/file, 4 MiB/batch |
| Physical settings | Letter/A4 UI; backend dimensions 144-1,440 pt, margin 0-144 pt leaving at least 72 pt content in each direction, text 8-36 pt |

Animated PNG, CMYK/lossless/arithmetic/higher-bit-depth JPEG and unsupported
images are refused by the shared bounded still-image decoder. Preparation is
cancelable, with no partial document registered. Creation jobs participate
in quit/unload guards. Settings are creation-local, not document drafts.
Native input comes only from explicit local picker selections; preview uploads
bytes and filenames, never arbitrary filesystem paths.

## Exporting editable Word document content

Choose **Export Word text (.docx)** in All tools or **File > Export Word
Document Content**. Select all pages or explicit ranges in the desired
order, for example **3, 1-2**. Folio extracts the current working snapshot's
actual Unicode text with PDF.js, reviews the ready text summary, and saves
a separate interoperable Office Open XML package with real editable
paragraphs and explicit selected-page breaks. It is not a renamed PDF,
page screenshots inside Word, or a cloud/system-Office conversion.

**This is text / document-content conversion, not high-fidelity Word layout.**
Extraction order can include invisible/off-crop text, and columns, paragraph
structure, line wrapping and Word page counts can differ. Tables/charts,
images, annotations, form-field values, OCR, source-PDF paragraph editing,
full Office import/export, and PDF signature preservation are not provided.
Empty scans report **No editable text**, not invented content. XML characters
are correctly escaped; invalid XML controls and malformed text are refused
without silently dropping or replacing them.

Limits are **200 selected pages**, **64 MiB source/renderer snapshot**,
**256 KiB UTF-8 text/page**, **8 MiB text total**, and **64 MiB DOCX package**.
The minimal OPC package has correct content types, package relationships
and a WordprocessingML document part; it needs no installed Word or LibreOffice.
Existing ZIP/CRC helpers are shared with image export; no runtime dependency
was added.

Copy/extraction permission is required independently of editing; true owner
authentication is honored. Authorized signed/read-only sources can be copied
without rewriting them. Protected inputs require explicit acknowledgment
that DOCX is **unencrypted and retains no PDF passwords, permissions or
cryptographic signatures**. Apply/discard every pending document draft,
staged image and redaction first. Drafts, edits, staging, redaction jobs and
closing permanently invalidate prepared output, even when Undo restores an
older revision. Canceling or failing releases partial output and renderer
resources without changing source bytes, saved state or history.

Native saving rechecks the scoped job, revision, drafts and every open tab's
source/saved paths after the dialog, then atomically writes a **new,
no-clobber file**. Existing files/folders/symlinks are not overwritten.
Browser preview downloads the copy without claiming a local file was saved.

## Comparing two open PDFs

Choose **Compare PDFs** in All tools or the title-bar comparison button. Select
the **Before PDF** and **After PDF** tabs, including applied but unsaved edits,
then choose **Compare working PDFs**. With only one PDF open, **Open another
PDF** adds the second without replacing the first. Comparison never changes
either document's bytes, dirty state, undo/redo, selected page or reading position.
Selections, settings and results remain at workspace level when returning to
document tabs or other tools.

Matching is strictly **page 1 to page 1**, not semantic/reflow or moved-page
analysis. A middle insertion shifts subsequent pairings; only unmatched trailing
indexes are labeled added/removed. Each indexed page reports exact embedded-text
token changes in PDF.js extraction order, visible RGB pixel changes, and
MediaBox/effective CropBox/rotation/UserUnit differences. Text is not OCR or
inferred paragraph order; invisible/off-crop text returned by extraction can
also appear in the text comparison. Annotation and form appearances contribute
only where rendered, including small appearance-only changes.

Review side by side or with an adjustable overlay and changed-pixel mask.
Rendering is local, on white, at an explicit **72, 96 or 150 DPI**; pixels
are top-left aligned without scaling differently sized pages to fit. An RGB
channel must differ by **more than** the selected 0-64/255 tolerance to count.
Geometry is checked separately without a pixel tolerance. Previews fit within
1,200 px; actual pixel comparison uses the full selected resolution.

**No observed differences does not mean identical PDFs.** Metadata, bookmarks,
attachments, hidden objects, signature changes/validity/trust and other
unexamined document structure are not compared. This is not a whole-file
equivalence check. Unsupported renderer/core geometry disagreements (including
different raster rounding at a pixel boundary) and
over-budget work are rejected rather than reported as matches.

Both PDFs must allow copy/extraction; owner-password access is honored.
Signed/read-only documents remain eligible for permitted comparison, without
enabling edits. Apply or discard pending writing, forms, metadata, authoring,
redaction and live-gesture drafts on **both** inputs first. A source mutation,
new draft or closed tab invalidates the comparison, even if Undo later restores
the old revision. Source IDs/revisions and draft/copy guards are rechecked
around asynchronous snapshots, completion, and export. Canceling discards partial
results and releases isolated PDF.js renderers, workers and canvases.

**Export JSON report** writes a separate, bounded, unencrypted report containing
per-page text spans, pixel counts/bounds, geometry and method limitations, not
page images or a complete object diff. Protected inputs require an explicit
unencrypted-content acknowledgment: reports retain no PDF passwords, permissions
or signatures. Native export never overwrites existing files or any open PDF.

One comparison is retained at a time. Limits are 100 pages and 64 MiB per
source snapshot; 8 megapixels/8,192 px per dimension per aligned page pair,
100 megapixels total; 256 KiB/20,000 tokens per page side and 40,000 extraction
items per page; 2 MiB extracted text, 100,000 tokens and 4,000 spans overall;
32 MiB previews and a 4 MiB JSON report. Processing is sequential. Complex text
diffs have bounded edit/time budgets; no content, changes, pages or quality are
silently truncated or downgraded.

## Password-protected PDFs

Opening a supported standard-password PDF prompts for its password in the
desktop app or browser preview. The native picker, drag-and-drop, and Finder
opening use the same flow. Leave the password blank for an encrypted PDF with
no opening password; use its owner password to edit restricted content.
Wrong passwords and cancellation leave the current document and unsaved work
unchanged. Passwords are kept only in memory for the open document, not in
preferences, logs, or external services. The browser preview sends them only
in request bodies to its loopback-only Rust engine.

Folio preserves the existing encryption algorithm, passwords, and permission
settings when saving, undoing/redoing, extracting pages, or exporting redacted
copies. The original file remains unchanged by default. Decrypted bytes are
prepared in memory for the renderer only; downloads and native saves use
protected bytes, not that rendering copy. Signed PDFs remain read-only and
an unchanged saved copy retains the original bytes.

User-password access with restricted editing/copying permissions is
conservatively read-only for general editing; reopen with the owner password
to edit content or export pages. **Forms** separately permits filling when the
PDF grants `FILLABLE` access (`ANNOTABLE` for legacy revision-2 protection),
without enabling page, text, annotation, or extraction edits.
Permitted read-only printing and image/text export remain
available without enabling editing. Printing honors `PRINTABLE` and the
high-quality-print permission; text/images honor `COPYABLE`. Owner access
overrides these permission restrictions. Text copying is disabled when the PDF disallows it. Encrypted
donor pages cannot be inserted into an unencrypted destination; when the
destination is encrypted, inserted pages inherit its protection.

Redacted exports still discard the original document objects and descriptive
metadata, but retain the encryption parameters needed to keep the passwords
working. Older RC4/AES-128 protection also requires the original file
identifier, so an encrypted redacted export is not an unlinkability guarantee.
Ordinary saving does not create, remove, or change passwords. Use the separate
protection-copy workflow below for an explicit change. Certificate-based security
and unsupported encryption filters are rejected explicitly rather than silently
removing protection. Errors while reading a malformed file leave the current
document unchanged.

## Cryptographic PDF signatures

Choose **Digital signatures** in All tools or the right utility rail. Inspection
checks supported detached CMS signatures against the PDF's original byte spans,
without rewriting it. Each result separates **signed-byte integrity**,
**whole-file versus earlier-revision coverage**, **current certificate dates**,
and **custom certificate-chain evaluation**, **revocation not checked**, and
**trusted time not checked**. Certificate subject, issuer, serial number,
SHA-256 fingerprint, date bounds, and signature/digest algorithms are available
in the details disclosure.

**Intact bytes do not mean a trusted signer.** Without explicitly added custom
CA anchors, identity trust remains **not checked**, including for embedded
roots and self-signed signers. Certificate dates use this computer's current time,
not an authenticated historical signing time. Folio makes no revocation,
trusted-timestamp, qualified-signature, certification-policy, or PAdES-LTV
assurance and makes no TSA, OCSP, or other external certificate requests.
Later appended bytes are explicitly **not covered**, even if the older signed
revision's CMS signature is intact.

For **Export signed copy**, choose a local `.p12`/`.pfx` identity (maximum
4 MiB), enter its password, review the current PDF, and sign a separate output.
The native Save dialog cannot overwrite this document's source/saved file or
another open tab's file. Browser preview sends the selected identity only to
its loopback Rust engine and downloads a new PDF; it accepts no server file
paths. Keys and passwords are used only in memory, never saved in preferences
or logged. The password field clears after submission; native/preview
identity buffers and Rust password buffers are zeroized on drop.

New signatures use SHA-256 and a currently valid RSA 2048-8192-bit or NIST
P-256/P-384/P-521 ECDSA certificate, with a matching private key and appropriate
document-signing usage. The default remains a new invisible approval field
with detached `adbe.pkcs7.detached` CMS, not a drawn signature image.
**Current unsaved PDF edits are included.** Export does not mark the working
document saved, clear its history, or replace the source shown in the window.
Open the exported copy to inspect it; signed
PDFs remain read-only for ordinary editing, extraction, and page import.

In **Signature location**, optionally select an existing empty signature field
by its name, page, and logical object ID (for example `50:0`, not its widget ID).
Both merged field/widgets and separate indirect `/Kids` widgets are supported.
Signing sets that field's `/V`, retains its identity and geometry, and replaces
its visible widgets' unsigned placeholders with persistent embedded-font
appearances. These show a certificate fingerprint and a clearly **unverified
local time**, not a trusted timestamp or a validation badge. Invisible selected
widgets remain invisible. Other fields, attachments, and appearances are retained.
Blank signature slots alone do not make an unsigned document read-only.

For a supported already-signed source, use **Export co-signed copy** to append
another approval signature, either in a selected blank slot or a new invisible
field. All signature exports use lopdf's `IncrementalDocument`: every current
source byte is an exact prefix of the output. Prior signature objects and
revision coverage are retained; all previous supported CMS signatures and the
new whole-file signature are reverified after reopening before any file is
written. Earlier signatures cover their own revisions, not the newly added
approval. The latest existing signature must cover the entire source file:
unsigned appended bytes/revisions, invalid or unsupported signatures, and
ambiguous bindings are refused, not legitimized by co-signing.

Pending writing, forms, metadata, page-authoring/image/gesture drafts, and
redaction marks block signing in both the UI and document engine. Redaction overlays are
not removed content: first export and inspect the redacted PDF, then open
that separate copy and sign it.

### Explicit local custom CA anchors

Expand **Custom CA store** and choose **Add public root CA** to explicitly load
one public `.pem`, `.der`, `.cer`, or `.crt` certificate, at most **64 KiB**.
The store accepts at most **32 self-signed root CA certificates**, requiring
critical CA basic constraints, certificate-signing key usage, a supported strong
key and SHA-2 certificate signature. Private keys, PKCS#12 identities, leaf
certificates, non-root intermediates, PEM bundles, and trailing DER data are
rejected. Intermediates must come from the CMS certificate set, not a download.

Anchors are **public certificates in memory for this app run only**, shared
across tabs. Nothing is persisted to preferences or disk. Add/remove actions
use a separate store revision; removal immediately invalidates cached conclusions
in active and inactive panels, including late inspection responses. Removing
all anchors returns reports to `not_checked` without changing any PDF revision.

OpenSSL verifies chain signatures, paths, current dates, critical extensions,
CA constraints, and key usages with strict verification, authentication level 2,
and verification depth 16. PDF signing has no built-in OpenSSL purpose: Folio
does not impose TLS/email semantics, but explicitly requires leaf signing usage,
a non-CA leaf, and absent or document-signing/any-purpose EKU on the chosen
path. Document-signing EKU is `1.3.6.1.5.5.7.3.36`; other restricted purposes
(including TLS-, email-, code-, or timestamp-only certificates) are refused.
Chain certificates use supported RSA/ECDSA keys and SHA-256/384/512 RSA PKCS#1
or ECDSA certificate signatures; RSA-PSS/EdDSA certificate chains are outside
this subset. Expired/future CA certificates can be loaded for inspection, but
cannot produce a valid current-date chain.

Success is labeled **Chain anchored in your custom CA store**, **not Adobe
AATL/EUTL or OS trust**. No system/keychain roots, embedded signer auto-trust,
certificate discovery, AIA downloads, OCSP/CRL checks, TSA, qualified-signature,
or LTV claims are made. A valid chain never overrides failed integrity,
unsigned later bytes, current-date failure, or absent trusted time.

The explicit supported subset is:

- Verification: ordinary indirect signature dictionaries with a single
  hexadecimal Contents gap, exactly four direct ByteRange integers, a complete
  signed PDF revision, an embedded unambiguous signer certificate, and one
  detached CMS signer. Supports `adbe.pkcs7.detached` and
  `ETSI.CAdES.detached` content-integrity checks with SHA-256/384/512 and
  RSA PKCS#1 v1.5 or supported ECDSA keys. This is not a claim of CAdES/PAdES
  profile conformance.
- Export: unencrypted, unrepaired PDFs with ordinary table or stream
  cross-references, including supported incremental sources and AcroForms.
  Supported approvals may be appended to unsigned or fully covered signed
  sources. Hybrid references, XFA, certification/DocMDP/FieldMDP, signature
  field locks and seed-value policies, direct/orphan/shared field structures,
  actions on a selected slot, and ambiguous signatures are refused.
  There is no encrypted signing, certification, policy-approved editing,
  or general mutation bypass for signed files.
- Unsupported verification includes encryption, compressed/direct signature
  dictionaries, repaired cross-references, DocMDP/FieldMDP transforms,
  multiple CMS signers, attached CMS content, unsigned CMS attributes
  (including timestamps/countersignatures), embedded CMS revocation lists,
  non-X.509 certificate choices, RSA-PSS, EdDSA, weak/unknown
  digests or keys, and missing external certificates. Malformed bounds,
  duplicate dictionary keys, mismatched Contents gaps, trailing nonzero CMS
  padding, and failed cryptographic checks cannot produce an intact result.
- Limits: 64 signatures per inspection, 256 KiB per CMS Contents value,
  64 embedded certificates, and a 32 KiB CMS reservation for new signatures.
  Signing supports at most 64 signature fields, 16 widgets per logical field,
  and form trees within 4,096 nodes / 32 levels. A visible selected widget must
  fit the visible page and measure at least 72 x 32 physical points when
  displayed, at most 14,400 points per side; zero-width/zero-height invisible
  widgets are supported. Independent widget `/MK` rotation, hidden/read-only/
  locked widgets, and malformed geometry are refused. An oversized chain or
  output above 128 MiB is rejected before any export is written. The engine
  also requires 128 available object numbers below its `u32` ceiling to reserve
  safe incremental allocation headroom.
  Raw cross-reference links are checked through at most 4,096 revisions;
  hybrid references in any earlier revision and cyclic/out-of-bounds links
  are refused even when the PDF library normalizes away those trailer keys.

The implementation uses RustCrypto `cms`/`der`/`x509-cert` for structural
checks and the Rust `openssl` bindings with vendored OpenSSL 3 for PKCS#12
and CMS/path cryptography, with lopdf's existing incremental writer rather
than a parallel PDF or cryptographic engine. No new dependency was added;
existing OpenSSL and RustCrypto license/attribution sections are retained.
No external executable or test private key is shipped.
This feature is unrelated to macOS application signing or notarization.

## Filling existing PDF forms

Choose **All tools > Fill form fields**, or the **Show form fields** button
on the utility rail. The panel lists qualified field names and accessible
labels, with page links for each occurrence. Filter by name or the current
page. A field repeated on several pages is one value, not independent drafts.

Supported AcroForms include plain text, multiline text, comb boxes with a
maximum length, checkboxes with a shared export/on-state, radio groups with
unambiguous states (including `RadiosInUnison`), single/multiple-selection
lists, and fixed/editable combo boxes. Choices show their display labels while
writing the PDF's original export values and selected indices. Required,
read-only, locked, maximum-length, radio no-toggle, and choice constraints
are enforced. Signature fields are identified but are not text inputs.

Changing a control creates a **document-scoped draft**, not an immediate
full-file rewrite. Drafts survive panel changes, page virtualization, and tab
switching. Choose **Apply form changes** to update values and every repeated
widget appearance in one undoable mutation, then save a copy. Applying errors
keep all drafts. Discarding requires confirmation. Ordinary Save/download,
document-history actions, source mutations, closing, and quitting cannot
silently omit unapplied values, including in inactive tabs.

Changed text/list/combo appearances embed the existing bundled Noto Sans font
for precomposed Latin, Greek, and Cyrillic text; the original field typeface
may change. Font size, alignment, simple colors/borders, widget rotations,
and page geometry are respected. Auto-size fields fit down to 4 pt; values
that cannot fit fixed-size fields are rejected rather than silently clipped.
Existing checkbox/radio Off/on appearance streams are retained and their
`AS` states synchronized with `V`. Filling does not flatten the form.
If the PDF already requests viewer-generated appearances (`NeedAppearances`),
that global request is retained so untouched fields are not suppressed; changed
widgets still receive their own embedded appearances and font resources.

Password-type inputs and their saved appearances are masked. A PDF password
**field** is not PDF file encryption: its value remains ordinary form data
unless the file itself is protected. Do not enter secrets in an unprotected
form. Folio does not log form values or execute PDF scripts.

XFA, calculation dependencies, document/page actions, signed/certified/
append-only forms, field actions/scripts, rich-text/file-selection inputs,
multiline password fields, ambiguous names or radio states, missing button
appearances, custom/beveled/inset/rounded borders, and unsupported geometry
are explicitly read-only or rejected. A PDF with document JavaScript is not
silently filled with stale calculations. Filling does not author controls,
submit forms, run JavaScript, or verify signatures.

Inspection is bounded to 1,024 fields, 4,096 tree nodes/options, 32 tree
levels, and 1 MiB of field text. Values are limited to 4,000 characters or
the PDF's lower maximum. One apply handles at most 1,024 widgets and 32 MiB
of generated appearances. Original synthetic fixtures are generated by
`src-tauri/tests/forms.rs`; no user forms are bundled.

## Designing forms and exporting a static copy

Choose **All tools > Design form fields**, or **Design form fields** in the
filling panel. Create a supported control or select an outlined existing field.
Edit its partial name, accessible label, defaults, text/choice constraints and
individual widget placements. **Place selected widget on page** lets you click
to move it or drag a new box to move and resize it. Page/left/top/width/height
inputs provide the same operation from the keyboard. Properties apply to the
whole logical field; layout and individual-widget removal do not split shared
values into unrelated fields.

Design settings are per-document workspace drafts, not virtualized-page state.
They survive panel/tab changes, including invalid numeric input, until explicitly
applied or discarded. The Unapplied banner and fixed footer distinguish previews
from PDF changes. Save, history, close/quit, other mutations, signing, OCR and
output cannot silently omit pending design drafts. Defaults on an existing
field do not replace its current value; use **Fill form fields** first when a
current value would violate a new constraint. Save a copy after applying.

**Export static form copy**, available in the designer and filling panel,
reviews the exact field/widget counts and limitations before producing a
separate PDF. This tab keeps showing the interactive original, with its existing
unsaved edits, history and protection unchanged. Password values are painted
as masks, and empty signature slots remain unsigned static outlines.

The form-design core creates text (including multiline, password and comb),
checkbox, radio, list, combo and **empty digital-signature** fields. It edits
partial names, accessible labels, defaults and core constraints; changes choice
options without silently discarding current exports; moves/resizes one widget;
and removes supported fields or individual repeated widgets. Existing field
types and button export-state identities cannot be converted. New fields can
have repeated widgets; adding widgets to an existing logical field is not
implemented. Radio creation uses exactly one widget per option. Empty signature
slots contain no `/V`, do not enable append-only signature policy, and do not
make the document or its other fields read-only. They are not signatures.
Signature widgets keep `/MK/R` at zero and carry page orientation in the
appearance matrix, including after cross-page moves, for selected-slot signing.

Logical IDs remain PDF object keys such as `"12:0"`; `FormWidget.id` supplies
the independent widget object key. New fields use an indirect logical parent
and indirect `/Kids` widgets with matching `/Parent`, `/P` and page `/Annots`.
Existing merged fields and inherited parent/child trees retain their identities.
Clearing an inherited default isolates sibling defaults first. Removal refuses
external references, including tagged-structure references, rather than leaving
dangling objects. Property/layout changes preserve current values and regenerate
text/choice appearances using the filling engine's actual-glyph fitting.
Definitions can leave required fields initially blank; this does not weaken the
separate filling constraints. Button captions use widget `/Contents`; existing
`/Opt` submission exports and unrelated per-widget captions are preserved.
Choice exports are unique and may include one empty placeholder. Button states
use portable ASCII names other than `Off`. Widget-level locking flags are not
implicitly cleared by changing the logical field's read-only flag.

All design mutations use the transactional, encrypted, undoable `Session`
writer and require **authorized general editing** plus an available 64 MiB
undo snapshot. A user password granting only form filling cannot author or
flatten. Requests carry `documentId` and `revision`. Native commands and
loopback-only preview routes are equivalent:

| Native command | Preview route | Result |
| --- | --- | --- |
| `form_designer` | `POST /forms/designer` | Supported field definitions, per-field capability reasons and document permission |
| `design_form_fields` | `POST /forms/design` | Updated `DocumentInfo`; `changes` is an atomic batch of `create`, `update`, `place_widget`, `remove_widget`, `remove_field` |
| `inspect_flattened_form_copy` | `POST /forms/flatten/inspect` | Field/widget/unsigned-slot counts and explicit copy warnings |
| `export_flattened_form_copy` | `POST /forms/flatten` | Native separate-file dialog / preview PDF bytes, never preview filesystem paths |

`Definition`, `WidgetPlacement` and the exact tagged change payloads are in
`src-tauri/src/forms/designer.rs`. Placements are `[left, top, width, height]`
in physical points from the **visible, rotated page's top-left**; saved widget
rectangles remain unrotated PDF coordinates. Crop offsets, all right-angle
rotations and page-local `UserUnit` are respected. One batch supports 1-64
changes and a new field 1-64 widgets (one for a signature), with widget dimensions
4-14,400 default PDF units. The existing 1,024-field, 4,096-node/option and
4,000-character limits still apply.

A static form copy paints normal appearance Form XObjects into persistent page
content and removes the AcroForm and widget/action objects. Text and choices
are rebuilt from current values, so `NeedAppearances`, stale/missing text AP and
password masking are handled explicitly. Visible read-only fields are included;
hidden, optional-content and no-view/no-zoom/no-rotate widgets stop export.
Buttons select the authoritative Off/on stream using `/V`, not stale `/AS`.
Inherited resources and appearance `BBox`/`Matrix` transforms are captured.
Each static appearance clips once in page coordinates; PDF-lib half-unit
borders and fractional `UserUnit` values must not acquire a second antialias clip.
Every selected list value must fit the visible rows in every widget.
Malformed or unbalanced content, unsupported resources, unknown types,
XFA/scripts/calculations, signed/certified sources and ambiguous structures
fail without writing a partial copy. Maximum: 1,024 widgets, 32 MiB decoded
page/appearance content and 128 MiB serialized output.

Static export preserves the source session, unsaved edits, saved state, history
and existing encryption. Draft/revision checks run again after the native
dialog, and source/saved/other-tab paths are protected. Other page content,
annotations, metadata and attachments remain. **This is not secure
sanitization, redaction, PDF/A conversion or digital-signature preservation.**
Legacy page-operation guards for interactive forms/XFA remain unchanged.
Original synthetic API/history/protection fixtures are in
`src-tauri/tests/form_designer.rs`.

Run `npm run test:forms` for original synthetic field creation, independent
PDF.js/pdf-lib reopening, per-widget pixel comparisons across all rotations,
exact RGBA comparisons of filled PDF-lib fractional-edge forms, protected-copy
reopening, and isolated UI draft/history/export checks. Set `FOLIO_FORMS_ARTIFACTS` to
retain evidence, `FOLIO_FORMS_BROWSER` for an installed compatible Chromium
binary, and `CARGO_TARGET_DIR` for a separate build cache. The runner prefers
loopback ports 1429/19157, selects free alternatives if occupied, and stops
only its own helpers. It does not use the running native app or user documents.

## Exporting a protection copy

Choose **All tools → Export protection copy**. Set separate opening and owner
passwords and printing/copying/content/annotation/form-filling/page-assembly
permissions, or explicitly choose **No password protection**. The exported PDF
is a separate file; the current tab's bytes, passwords, history, saved path,
and unsaved work are unchanged. Native exports cannot overwrite the active
PDF, its saved copy, or another open tab's source/saved files. The revision
and pending-draft state are checked again after the save dialog.

Creating protection uses lopdf's standard **AES-256, V5/R6, PDF 2.0** handler,
with fresh operating-system randomness and encrypted metadata. It does not
create deprecated revision-5 encryption. New passwords are limited to **127
UTF-8 bytes**, without silent truncation. The owner password must be nonempty
and distinct from the opening password, including after PDF password
normalization. An empty opening password means anyone can open the copy.
Credentials stay in memory, are masked, are cleared from the export dialog on
submission/closure, and are never saved in preferences.

**Removing or replacing existing protection requires actual owner
authentication**, even if the opening password allows every normal edit.
Enter the current owner password unless this tab was already opened as owner.
Incorrect passwords and canceled dialogs leave the tab/source unchanged.
Signed/certified PDFs cannot use this export path. Pending writing, form,
metadata, and redaction drafts must be finished or explicitly discarded first;
the export does not quietly omit drafts or marked redactions.

Permission flags are **cooperative PDF restrictions, not absolute DRM**.
Owner access overrides them; other readers may ignore them. Annotation
permission also allows filling existing forms. Accessibility extraction
remains enabled for PDF 2.0 compatibility. Folio retains its conservative
general editing guard for restricted user access. This workflow does not
perform redaction or sanitization, and older readers may not support PDF 2.0
AES-256.

## Document management

Use **All tools → Document properties** (or the properties rail button) to
inspect/edit the title, author, subject, and keywords. **Apply** makes one
undoable document change; save a copy to persist it. Drafts stay at the
document-workspace level, survive side-panel/tab switching, and block stale
saving, closing, and protection/attachment exports until applied or discarded.
Clearing a field is a real pending edit.

Classic Info values and supported existing XMP are updated together while
preserving unrelated properties. Supported XMP is UTF-8 RDF with document
descriptions (`rdf:about=""`), standard Dublin Core title/creator/description/
subject and PDF Keywords, in attribute or element form. Updated title/subject
alternatives use one `x-default` value; creator and keyword bags use the
entered values. Named/non-document RDF subjects, DTDs/custom entities, malformed
XML, unsupported encodings, and oversized/deep packets are rejected rather
than leaving contradictory old descriptive fields. Limits: **4,096 UTF-8
bytes per field**, **1 MiB encoded/decoded XMP**, **64 XML levels**, **50,000
events**, **256 attributes per element**, and **8 stream filters**.

**Bookmarks** adds actual `/Dest [page-reference /Fit]` outline entries.
Choose a page and optional parent; select an existing bookmark to rename it,
move it up/down, nest it under its previous sibling, move it out one level,
add a child, or delete a group. Deleting a group removes its descendants, not
PDF pages. Existing actions, destination details, style, unknown properties,
and open/closed group state are preserved. Trees are bounded to **2,000
bookmarks / 32 nested levels**; malformed, shared/cyclic, or inconsistent
outline links are refused. All changes support undo/redo and save/reopen.
Document-management mutations require the current serialized PDF to fit within
the **64 MiB undo-snapshot budget**; larger inputs remain inspectable/exportable
but are rejected for these edits rather than silently losing the only undo
snapshot. A successful attachment addition can produce a larger output; undo
and redo still work, but further management edits then require a smaller PDF.

**File attachments** manages the catalog's `Names/EmbeddedFiles` tree.
Add through a user-driven native/browser picker; inspect names and actual
decoded sizes; save a selected payload only after confirmation; remove an
entry with an explicit decision. Files are untrusted: Folio never opens,
executes, or follows an attachment path. Suggested output names discard path
segments, control characters, invalid filename characters, and Windows device
names. Native output locations are selected explicitly and cannot overwrite
any open PDF source/saved path. The preview accepts uploaded bytes, never an
arbitrary local path.

Attachment limits are **128 entries**, **16 MiB per encoded/decoded
payload**, **64 MiB aggregate encoded/decoded payloads**, **32 tree levels /
1,024 nodes**, and **8 decoding filters**. Decoding is bounded before
allocation; malformed filters/parameters and differing Unicode/legacy
payloads are rejected. PDF portfolios, related-file chains, page attachment
annotations, and associated-file references outside this list are not managed.
Other name trees remain unchanged.

**Metadata changes and attachment removal are not secure sanitization or
secure erasure.** Unreferenced PDF objects, other references, and in-memory
undo snapshots may retain old values/payloads. Ordinary encrypted edits keep
the original protection. The dedicated permanent raster-redaction workflow
remains the route for its supported secure content-removal guarantee.

PNG, JPEG, ZIP and TXT are deliberately **unencrypted** exports, not protected
PDF saves. Exporting them from a protected PDF requires an explicit
acknowledgment before content is prepared. A printer or Print-to-PDF driver
likewise receives raster page appearances, not the encrypted source PDF or
its signatures. PDF optimization retains existing passwords and permissions.

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

## Offline OCR

Choose **All tools > Recognize scanned text**, enter pages such as `1, 3-5`,
choose **300 DPI** (or **150 DPI** for lower memory use), then **Recognize
text**. The included language is **English**. Recognition uses a local
Tesseract WebAssembly worker, not a cloud service, system-installed executable,
or a CDN. Annotations and form fields are excluded from recognition input;
finish/cancel pending writing/form drafts and clear redaction marks first.

The default **Skip existing text** policy conservatively skips pages containing
text-show operations, including hidden text and nested Form XObjects. **Force
an additional text layer** is explicit: it can duplicate searchable text and
does not remove any previous text layer. The summary distinguishes skipped
pages, pages with no recognized text, and actual words added. Review recognition
accuracy and reading order before relying on search or copying.

OCR adds real invisible PDF content using embedded Noto Sans and Unicode
mapping. Original content/image streams remain intact; pages are rendered only
as recognition input, never raster-replaced. Word boxes and line baselines are
mapped back into unrotated PDF coordinates, respecting the visible CropBox,
nonzero box origins, right-angle rotation, and UserUnit. Private font resources
and a prefixed, balanced graphics/text-state scope prevent changes to the
original visible content.

Recognition is sequential and cancellable, including while loading the
engine/model. **Cancel OCR** releases the worker/canvas and discards all staged
results. Every page is validated before one all-or-nothing document mutation.
Jobs are tied to document ID/revision and invalidated by edits, undo/redo,
pending drafts, or tab closure; an undo back to an old revision cannot resurrect
a stale job. Other tabs are not modified. The final atomic commit cannot be
canceled once started. No-text/all-skipped jobs create no change or undo step.

The result stays **unsaved** until ordinary Save/Save a copy/Download. One Undo
removes the whole job within the normal history budget; Redo restores it. Saved/reopened text remains selectable
and searchable. Supported password protection, passwords, and permissions are
retained. Signed PDFs are refused; restricted PDFs require editable owner
access. OCR is **not redaction, accessibility tagging, or PDF/UA
conformance**; handwriting, other recognition languages, vertical/right-to-left
text, severely skewed baselines, and unsupported output glyphs are not supported.

Limits are **50 selected pages**, **10,000 words/page**, **100,000 words/job**,
**8 MiB recognized UTF-8**, **24 megapixels / 16,384 pixels per page dimension**,
**64 MiB per rendered PNG**, **two minutes per worker step**, and **15 minutes
per recognition job**. The engine permits at most four staged jobs across a
workspace and one per document; the current UI serializes document actions.
Oversize/unsupported results fail explicitly rather than lowering resolution,
dropping words, or committing partial text.

### Reproducible offline assets and licenses

`tesseract.js` **6.0.1** and compatible `tesseract.js-core` **6.1.2** are
exact-pinned in `package.json` and integrity-pinned in `package-lock.json`.
`npm ci` downloads those packages at **build time**. `scripts/prepare-ocr.mjs`
checks installed versions, verifies committed SHA-256 pins for every worker/core
variant and the model, and copies the worker,
all matching SIMD/non-SIMD and LSTM/full WASM core variants, and the English
model to `public/vendor/ocr`. Vite includes these files in `dist`; Tauri embeds
that frontend. Runtime worker, core, and model paths are all explicitly local,
language caching is disabled, and missing assets fail without CDN fallback.
`public/vendor/ocr/manifest.json` records the copied assets' SHA-256 hashes.
Preparing/building needs no model download.

The original, unmodified `eng.traineddata` is committed at
`src-tauri/resources/ocr/eng.traineddata`. It was obtained from:

```text
https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/87416418657359cb625c412a48b6e1d6d41c29bd/eng.traineddata
SHA-256: 7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2
```

To restore this source asset, fetch only that immutable URL and verify that
exact checksum before building. Do not substitute a model from a moving branch.
The engine, WASM wrapper, and English model use **Apache-2.0** licenses.
Bundled engine/model license texts are included in generated third-party
notices and local OCR assets. Original notices for the WASM engine's Leptonica,
codec, compression, and math dependencies are committed under
`src-tauri/resources/ocr/notices`; provenance is recorded in `ocr/NOTICE`.
The PDF text font remains **SIL OFL 1.1**.

Run `node --test tests/ocr.test.ts` and
`npm run test:rust -- --test ocr` for focused checks. For real synthetic scan
recognition, save/download/reopen search and selection, all four rotations,
offset CropBoxes/UserUnit, pixel equality, cancellation/cleanup, and production
worker recognition with networking disabled under the native CSP:

```sh
npx playwright install chromium
npm run test:ocr
# Optional: retain original synthetic fixtures, PDFs, screenshots, and JSON evidence.
FOLIO_OCR_ARTIFACTS=/absolute/output/folder npm run test:ocr
```

The runner starts/stops only its own loopback servers on dynamically selected
ports and its own browser profile; it does not stop an existing Folio process.
`FOLIO_OCR_BROWSER=/absolute/path/to/chrome` can select an existing complete
Chromium/Chrome binary for verification. Tests use only newly generated,
original synthetic content, never private documents or copyrighted fixtures.
Rust OCR tests separately verify RC4/AES save/reopen with both passwords,
permission/signature guards, original resource isolation, and atomic stale-job
rejection. Native Windows/Linux execution and real-world OCR quality still
require the normal platform review.

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

## Page authoring

Open **Page authoring** from All tools or the writing panel. It groups **Image**,
**Crop**, **Watermark**, and **Page numbers** without changing the document
workspace. Each applied operation is one PDF history change, retains existing
password protection, and survives saving/reopening. Signed and restricted PDFs
remain guarded. Drafts live with their document tab, not virtualized page
components; switching tabs or scrolling away does not lose them. Apply or
explicitly discard a draft before ordinary Save, close, history, or another
document mutation. Failed input validation keeps the draft and working PDF.

**Image:** choose a still PNG or an 8-bit grayscale/RGB JPEG through the local
file picker in desktop or browser preview. Set left/top, width/height, original
proportions, and opacity; click a visible page to position the image or use
the numeric controls. This creates a standard `/Stamp` annotation containing
a real image XObject, not an HTML image or a link to the source file. PNG
transparency uses a PDF soft mask and combines with the selected opacity.
EXIF orientation is applied to pixels; original EXIF/location, filename, and
other image metadata are not copied into the PDF. Source color samples are
converted to 8-bit RGB; 16-bit PNG samples are reduced to 8 bits. CMYK,
higher-bit-depth/lossless/arithmetic JPEGs, animated PNGs, and other formats
are explicitly unsupported. Limits are **16 MiB encoded**, **24 megapixels**,
**8,192 pixels per side**, and **128 MiB of staged decoded images across tabs**.
Inserted image stamps can be removed in Comments or undone. They are separate
from the existing page-content image editor below.

**Crop:** drag the visible area to keep, or enter top/right/bottom/left margins.
Current-page, all-page, and explicit page ranges are supported. Each selected
page uses its own current visible edges, rotation, dimensions, and physical
scale; margins must leave at least one physical point in each dimension.
**Show full MediaBox** removes visible cropping, including an inherited crop.
**Reset original crop** restores the effective crop from before Folio's first
crop change, including after save/reopen and page reordering. The MediaBox and
underlying drawing instructions are never changed.

**Cropping is not redaction.** Hidden content, annotations, and metadata remain
recoverable. Use the separate **Redact a PDF** export for permanent removal.
PDF rendering, continuous layout, thumbnails, writing/selection overlays, and
redaction export all use the actual cropped geometry. Cropping documents with
interactive form fields/widgets or XFA remains unsupported.

**Watermark:** place a centered, single-line text stamp on chosen pages, with
font size (6-144 pt), color, opacity, and angle (-90 to 90 degrees). Positive
angles rise left to right. Text uses the existing embedded Noto Sans font and
its supported Latin/Greek/Cyrillic subset. Text that does not fit any chosen
page fails without changing any page. Applying again replaces only Folio
watermarks on the chosen pages; the panel can also remove those watermarks.

**Page numbers:** choose a starting integer (0-999,999), optional prefix/suffix,
font size/color, one of six top/bottom positions, and an edge-to-text-box
margin. The first page in the entered range receives the starting number;
`3, 1-2` with start `7` produces `7` on page 3, `8` on page 1, and `9` on page 2.
These are visible static stamps, not PDF page labels or an automatic
renumbering system. Reapply after reordering to renumber chosen pages.
Applying again replaces only Folio numbers there; removal is also available.

Watermarks and numbers are ordinary editable/removable PDF annotations, **not
tamper-proof security**. Their text appears in Comments, not Find's page-text
search. Use the authoring panel to replace/remove them, or remove an individual
stamp in Comments. No passwords, rights management, or certification are added.

Authoring measurements are physical points (72 pt = 1 inch) from the rotated
visible top-left. Stored annotation rectangles and source-text/OCR coordinates
remain absolute, unrotated PDF default-user-space units. The visible rectangle
is normalized inherited `MediaBox` intersect inherited `CropBox`; page-local
`UserUnit` supplies physical scale and does not change those coordinates.
Cropping never rebases the original content or coordinate origin.

## Editing existing images

Open **Edit existing images** in All tools or the writing panel. Select a
numbered on-page image outline or its exact occurrence in the current-page
list. The list inspects actual raster `/Image` XObject `Do` operations, including
nested/repeated Form invocations; it never treats annotation, stamp or widget
appearance images as ordinary page content. Unsupported occurrences expose
their reason instead of a misleading editing control.

Choose a local **PNG or JPEG** replacement, edit X/Y/width/height in physical
points, drag the selected outline to move, or drag its corner to resize.
Arrow keys move by 1 pt (Shift by 10 pt); the resize handle supports the same
keys. Physical placement is measured from the rotated visible CropBox's
top-left, with page-local UserUnit respected. A replacement preserves the
source footprint and right-angle orientation/reflection by default; different
aspect ratios stretch to that footprint. The sanitized thumbnail shows the
replacement; the on-page outline previews bounds only, not a simulated alpha
composite over the old pixels. Original PDF pixels remain until you choose
**Apply image changes** to mutate source page content.
**Mark selected occurrence for removal** followed by **Remove image occurrence**
removes only that invocation.

Each successful edit is one transactional, undoable source mutation. Shared
image objects, content streams and repeated nested Forms are isolated by
copy-on-write; other invocations, other pages, existing text/vector content
and appearance streams stay unchanged. Saving/reopening retains the actual
content change and existing password protection. Replacement decoding,
EXIF orientation, RGB conversion, metadata stripping and true alpha/soft-mask
handling reuse image insertion's bounded pipeline. Its **16 MiB**, **24 MP**,
**8,192 px/side** and combined **128 MiB staged decoded pixels across tabs**
limits also apply to replacement.

Supported placements are finite, nonzero and axis-aligned in page space,
including 90-degree turns and reflections. The entire image must fit inside
the visible page and every enclosing Form BBox, both before and after edits.
Inline images, stencil masks, optional-content layers, skew/oblique transforms,
explicit path/text clipping, page/Form transparency groups, reference/OPI
Forms, external/alternate images and unsupported alpha/blend/transfer/overprint
graphics states remain read-only or produce an explicit inspection error.
Dynamic XFA and signed/certified/general-edit-restricted PDFs are not editable.
Per-page inspection is bounded to **2,000 occurrences**, **200,000 operations**,
**16 nested Forms**, **16 MiB decoded content**, and 128 graphics/marked-content
stack levels. Cyclic, malformed and unstable structures fail explicitly.
Placement changes that cannot round-trip through PDF drawing numbers within
0.01 physical point, or would cross a Form clip after serialization, are
refused atomically instead of silently moving to a different position.

Replacement bytes, removal and placement input stay in the document workspace
across tabs, panels, scrolling and virtualization. Apply or explicitly discard
them before source edits/history, Save, close/quit, output, OCR, signing or
comparison. Live image gestures participate in the existing activity contract,
with independent feature/page ownership so one overlay cannot clear another
feature's gesture. Backend assets also guard output before the frontend draft
update; document/revision and a monotonic mutation epoch reject stale assets,
even when undo resurrects an old revision. Failures retain the draft and source.

**Image removal/replacement is ordinary editing, not secure redaction.** Old
image objects, original files, metadata and undo history may retain previous
pixels or private data. No image-extraction filesystem API is exposed. Use
the separate rasterized Redact export when permanent pixel removal is needed.

## Printing, content exports and optimization

Use **Print PDF**, **Export images or text**, or **Optimize PDF copy** in
All tools. The native File menu offers the same workflows.
**Command/Control+P** opens Folio's print controls, not a printout of the
virtualized workspace.

### Print selected pages or the whole PDF

Choose All pages or a range such as `24, 2-3, 1`, then 96, 150 or 300 DPI and
**Prepare print**. Preparation loads an isolated, revision-checked PDF snapshot
and renders pages sequentially, including pages that have never been mounted
on screen. Page crops, rotation and `UserUnit` physical scaling come from the
real PDF.js viewport. The print-only surface contains page images, not app
chrome, other tabs, note popups, password controls or editing overlays.

This is **raster printing**, not vector/native-PDF printing. High-quality
printing restrictions cap the choice at 150 DPI. Open the platform/browser
print dialog only after preparation completes. Check paper size, scaling,
orientation and browser headers/footers in that dialog; printer/driver
capabilities still determine the final paper output. Browser/webview versions
that cannot express the PDF's physical page sizes are refused explicitly.
Unsupported native platforms and native print-command failures are errors,
not reported as completed printing.
Embedded previews without Folio's native printing operation must be opened
in an ordinary desktop browser to print. Browser print calls that never
acknowledge a print lifecycle are reported as blocked/unsupported, not success.

Folio cannot know whether a driver actually printed, canceled or saved a file.
The UI says **Print dialog requested**, not "Printed." Prepared images and
their blob URLs remain alive until **Finished with print dialog** is explicitly
chosen after the dialog/job finishes. Closing a PDF or quitting is guarded
while a job is pending. Do not refresh or navigate away during a print job.
Direct browser-menu printing without a prepared surface cannot print the app
workspace.

### Export page images or selectable text

PNG uses lossless pixel encoding; JPEG uses the selected 60/80/90/100% quality
and remains lossy even at 100%. Both support 96/150/300 DPI, white page
backgrounds, and the same visible crops/rotations/physical dimensions as
printing. One selected page produces an image file. Multiple pages produce
one ZIP, in the selected order, with names such as `page-0024.jpg`.
ZIP entries use stored, already-compressed image bytes, not an additional
image recompression pass.

TXT exports actual selectable page text as UTF-8, in renderer reading order,
with `LF + form feed + LF` between selected pages. It adds no page headings,
source filename, annotation bodies, images or form-field values. It does not
perform OCR, preserve layout or infer missing text. Image-only/empty selections
produce an explicit no-text message instead of a success-shaped empty file.

Images and text lose PDF structure, encryption, permissions and signatures.
Protected-source exports require explicit acknowledgment and `COPYABLE`
permission (or owner access). Signed PDFs may still be printed or exported
read-only when their permissions allow; exported pixels/text are not signed.

### Optimize a separate PDF copy

Choose **Measure optimization** before saving. Folio clones the current
working PDF, compresses eligible streams that have no existing filters or
decode parameters, and removes objects unreachable from the trailer/catalog.
Existing compressed streams and image codecs are not decoded/re-encoded;
there is no image downsampling, font substitution, flattening, metadata
sanitization, PDF/A conversion or linearization. Reachable metadata,
attachments, bookmarks, annotations and page content are retained.

The result reports actual input, candidate and copy byte counts. If rewriting
is not smaller, Folio offers only an **unchanged copy of the original working
bytes**, not a larger "optimized" file. No compression ratio is promised.
Existing encryption/passwords/permissions are preserved using protected
serialization, never the decrypted rendering copy. Signed PDFs are rejected
before any rewrite, even with owner access. The existing conservative editing
permission policy also applies to optimization.

All three workflows preserve source bytes, saved state, undo history and every
tab on failure/cancel. Pending redactions, writing/form drafts and unfinished
pen/area gestures block ordinary output. Native writes recheck job identity,
document revision and draft state **after** the Save dialog, protect every
tab's source/saved paths, and atomically create a new file without replacing
existing files, folders or symlinks. Filename suggestions are sanitized.

### Output resource limits

Preparation is sequential, with one output job at a time. Exports allow up
to **500 selected pages**; printing allows **200 pages and 100 megapixels**
in the retained print snapshot. Each image is limited to **24 megapixels,
16,384 pixels per dimension and 64 MiB encoded**. Prepared output is bounded
to **128 MiB**, and text to **16 MiB including separators**. Oversized jobs
fail explicitly; resolution is never silently reduced. Optimization is also
limited to 200,000 PDF objects, 128 MiB encoded stream data, two million
traversal items and 128 levels of inline nesting. Cancellation stops
preparation without saving a partial file; native Save dialogs have their
own Cancel control. A running bounded compression call finishes before its
cancellation can be acknowledged, but cancellation prevents saving it.

## Technical preflight and PDF/A-2b archival copies

These are **two separate tools**, available in All tools and the native File
menu. Neither replaces the working PDF or certifies an arbitrary input.

**Technical preflight** inspects local, bounded dictionary/resource facts:
font-program stream presence (not font validity, coverage or actual use),
MediaBox/effective CropBox and other declared page boxes, right-angle rotation,
page-local UserUnit and physical dimensions, image dictionary properties,
form/active/attachment keys, catalog language/tag presence, and namespace-resolved
UTF-8 PDF/A/PDF/UA XMP identification properties. Each finding is an observed
**fact**, scoped **absence**, **unsupported** inspection, or **unchecked** property.
Flags and declarations are never promoted to a validation success. Inline images,
content-stream semantics, tag correctness/reading order, accessibility, font
program validity, hidden history and signature trust are outside this inspection.
The optional JSON report requires copy/extraction rights and an explicit
unencrypted-property disclosure for protected sources.

**PDF/A-2b archival copy** creates a fresh PDF 1.7 document from isolated PDF.js
page appearances, independent of mounted/virtualized canvases or HTML form
controls. Choose all source pages or an explicit unique ascending range and
exactly **150 or 300 DPI**. No pages are skipped and quality is never silently
lowered. Effective CropBox, rotation and page-local UserUnit determine actual
physical dimensions. Core and renderer pixel ceilings must agree exactly;
near-boundary source precision disagreements fail instead of clipping a pixel.
The new image matrix retains the chosen pixel pitch and clips only ceil-rounded
padding at the physical page boundary, not a rescaled whole raster.

Every page uses lossless opaque sRGB image pixels and the unchanged, checksum-pinned
ICC **sRGB2014** profile, an embedded output intent and UTF-8 PDF/A-2b identification
XMP. The source's objects, text/fonts/vectors, search/selection, accessibility
tags, interactive fields, attachments, active code, original metadata/history
and cryptographic signatures do **not** carry over. Explicitly accept these
losses and review the copy's appearances. This is not OCR, accessibility
remediation, automatic redaction, or validation of the original. Pending
redactions and every writing/form/metadata/image/authoring/live-gesture draft
block preparation. XFA, forms requiring regenerated appearances, visible widgets
without persistent appearances and unsupported multimedia/3D stop archival export.
Zero-area invisible signature slots contribute no pixels and do not require an
appearance; their cryptographic signatures still do not carry over.

PDF/A forbids encryption. Protected inputs require both actual **owner
authentication** (reopen with the owner password) and explicit acknowledgment of
an **unencrypted** archive, in addition to copy rights. Permissive opening-password
access is not ownership. Signed/certified inputs can be raster-copied only with
explicit **fresh unsigned copy** acknowledgment; the signed source remains
read-only. Renderer-only decrypted PDF snapshots are never exported as PDFs.

Folio's generated archives carry a private catalog marker and open **read-only**.
Ordinary edits, form filling, protection changes, signing, optimization, extraction
and import-as-donor rewrites are blocked, including narrow signed-source
exceptions. Exact-byte saving and honest non-PDF outputs remain available. Edit
the original and export a new archive; no ordinary Folio rewrite silently retains
the generated archival declaration. The marker itself is not validation evidence.

The archive has its own job identity, source ID/revision and monotonic source
epoch, separate from print/output, comparison, OCR and redaction jobs. Edits
(including undo returning to an older revision), drafts, image staging and tab
closure permanently invalidate retained technical results/jobs. Cancellation
releases staging, renderers, canvases and temporary URLs without partial exports.
Native writes recheck after the Save dialog, guard all open source/saved paths,
and atomically create a new copy without clobbering existing files or symlinks.
Close/quit/unload also guard archival work.

Inspection limits: **500 pages, 200,000 parsed objects, 50,000 reachable graph
nodes, 64 levels, 4,000 findings and 2 MiB report**; XMP is limited to 1 MiB,
50,000 XML events, 64 levels, 256 attributes per element, 64 identification
values and 4,096 UTF-8 bytes per value. DTD/entity-bearing or unsupported XMP is
reported, not guessed. Archive limits: **100 selected pages, 24 megapixels per
page, 100 megapixels total, 16,384 pixels per dimension, 64 MiB PNG per page,
128 MiB source/render/output and 15 minutes**. Unsupported/oversized jobs fail
explicitly instead of producing an apparently successful partial copy.

The profile's exact provenance, checksum and full redistributable license are in
`src-tauri/resources/color/NOTICE.txt` and the bundled third-party notices. It is
not an Adobe or operating-system profile.

### Development acceptance

`npm run test:archive` generates original synthetic fixtures, downloads actual
UI archival outputs, checks PDF.js physical geometry/pixels after save/reopen,
and runs **local veraPDF with `--flavour 2b`**. It also rejects negative controls
with false source declarations, missing/invalid ICC profiles and missing/false
XMP, checks protected/signed copies, source invariance, cancellation and resource
cleanup. Set `FOLIO_VERAPDF` to an already installed local CLI and optionally
`FOLIO_ARCHIVE_BROWSER` and `FOLIO_ARCHIVE_ARTIFACTS`. The test does not download a
validator, repair browser caches, start a native app or upload any document.
The verifier is a development/acceptance tool, **not an end-user Java service**
or a hidden runtime prerequisite. See the official [veraPDF validation
CLI](https://docs.verapdf.org/cli/validation/) and [verified local installation
instructions](https://docs.verapdf.org/install/).

Core APIs are `Workspace::inspect_technical`, `check_inspection`,
`inspection_report`, `write_inspection_report`, `cancel_inspection` and
`begin_archive`, `check_archive`, `archive_render_bytes`, `stage_archive_page`,
`archive_bytes`, `write_archive_copy`, `cancel_archive`. Native IPC uses
`inspectionId`/`archiveId`; the preview endpoints are `/preflight/*` and
`/archive/*`. Additional source-draft domains must reuse the aggregate draft
projection and `ensure_export_ready`, and call `invalidate_technical` when
staging or mutating a source.

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

The interface uses shared rounded-control/elevation tokens and a coordinated
motion system. A selection surface glides between document tabs, workspace
modes, and review tools. Panels crop into place, appearance choices settle
in a short sequence, dialogs gain depth without fading their text, and success
notifications draw their check mark. Hover/press feedback moves icon paint,
not control targets. Fast tab switches do not flash a busy indicator.

Motion never delays actions or animates PDF pages/thumbnails. The device's
**Reduce Motion** preference cancels in-flight selection movement immediately
and removes spatial/keyframe effects and smooth search scrolling, while
retaining clear state cues and gentle color feedback. Theme colors switch
atomically rather than fading through unreadable intermediate colors.
The implementation uses CSS and the Web Animations API, with no animation
package dependency.

## Important boundaries

- **Not full Acrobat parity.** Run-level source-text replacement is supported,
  but not paragraph reflow, unrestricted existing-image editing, non-English OCR,
  general Office conversion or PDF creation beyond the explicit PNG/JPEG/UTF-8 subset,
  automatic AATL/EUTL/OS trust, cloud storage, collaboration, AI assistant,
  accessibility remediation, full conformance validation/preflight, or
  print-production workflows. The bounded technical inspection and fresh
  image-only archival-copy subset above do not imply PDF/UA authoring or full
  Acrobat parity.
- Ordinary saves of supported password-protected PDFs retain their protection;
  explicit protection changes are separate, owner-authorized copy exports.
  Certificate-based security is not supported. Existing signatures remain read-only; the Digital signatures
  panel verifies the supported CMS subset and optional local custom-CA chains
  above, not all signature formats or authoritative signer identity.
- Supported AcroForms can be filled and authored within the explicit subset
  above. XFA, calculations, scripts/actions, field-type conversion and
  signature-field filling through the Forms panel are not supported.
  Cropping, deleting, duplicating, importing, or extracting pages in PDFs with form
  fields or XFA is refused. Duplicate pages with popup/reply annotations
  and imported documents with optional-content layers also require a
  flattened copy.
- Signed PDFs remain read-only, including for extraction and import as a
  donor. Folio does not claim that extracted pages retain a document signature.
- Unresolved or unsupported bookmark actions are preserved, not executed.
  Link targets that are unresolved,
  malformed, use embedded credentials, or are not HTTP, HTTPS, or mailto
  are reported rather than executed.
- Ordinary edits use a full PDF rewrite; only separate signature-copy exports
  use the narrow append-only incremental workflow. Work on copies.
  Complex portfolios, tagged-PDF structure, uncommon annotations, and advanced
  page-label/destination semantics need broader compatibility work.
- Maximum input/edited file size: **128 MiB**. Search displays the first
  **500 matches** and explicitly indicates when more exist.
- Undo keeps up to **20 snapshots within a 64 MiB history budget**. Older
  snapshots are evicted; a document larger than the budget may have no undo
  snapshot. History is in memory only.
- No autosave or crash recovery. Only applied form values, annotations, and page edits
  belong to the document; save a copy before quitting.
- Cropping, page deletion, text replacement, ordinary annotations, and extraction are
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

The development bridge binds only `127.0.0.1:19148`, accepts user-selected PDF,
PNG/JPEG and UTF-8 text bytes rather than filesystem paths, restricts browser origins, and is excluded from the
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
`workspace.rs` keeps independent document sessions and provides the same
document-ID/revision-checked operations to both
`desktop.rs` and the development-only `bin/preview.rs`. `pages.rs` implements
page creation/import/extraction/duplication; `writing.rs` generates standard
annotation appearances and embedded font maps.
`text_edit.rs` indexes and replaces existing text-show operations, including
nested Form invocations, with strict font decoding and isolated resource copies.
`image_edit.rs` indexes supported source-image occurrences and mutates exact
invocation paths. Both editors use `pdf_objects.rs` for copy-on-write content/
Form/resource isolation; image insertion/replacement share `authoring_image.rs`
for bounded, oriented and sanitized RGB/soft-mask embedding. The image UI,
draft model and transport live in `src/features/imageEditing`; native commands
live in `desktop_image_edit.rs`. Preview `/images`, `/images/edit`,
`/images/replacement` and `/images/replacement/cancel` expose the same checked
engine, accepting uploaded bytes rather than arbitrary image filesystem paths.
`redaction.rs` plans crop/rotation/UserUnit-aware pixel masks, decodes bounded
page PNGs, burns redactions, and constructs fresh image-only PDFs. The renderer
submits one page at a time; original PDF objects never enter the output builder.
`signatures/` performs strict signature-object/byte-range and CMS checks,
local certificate inspection, and signed-copy construction. Both bridges use
the same Session/Workspace revision, pending-draft, and separate-output guards.
`metadata.rs`, `outlines.rs`, and `attachments.rs` implement bounded document
management through Session history and encrypted serialization.
`protection.rs` owns standard-password authentication, permission reporting,
default protection preservation, and explicit AES-256 protection-copy exports.
`document_utilities_desktop.rs` contains picker/save commands; the development
bridge exposes only document-ID/revision requests and uploaded attachment bytes.
`output.rs` owns permission-checked output jobs, sequential image/text staging,
bounded ZIP construction, no-clobber copy writes and lossless optimization;
`desktop_output.rs` adds the native save/print commands. Frontend output
preparation, transport and the isolated print surface live in
`src/lib/output.ts`, `outputBridge.ts` and `printSurface.ts`, with
`components/OutputDialog.tsx` and print-only `src/output.css`.
`conversion.rs` owns transactional new-PDF creation, cancelable bounded
background preparation and the DOCX WordprocessingML package writer.
`desktop_conversion.rs` handles native creation pickers; `features/conversion/`
contains creation-local settings, ordering, transport and streamed DOCX text
extraction. DOCX reuses `output.rs` identity/permission/draft/lifetime and
atomic separate-copy guards, not a parallel save framework.

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

Focused comparison checks:

```sh
node --test tests/comparison.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test comparison
npm run test:comparison
```

The browser check requires Playwright Chromium (`npx playwright install chromium`);
`FOLIO_COMPARISON_BROWSER` can select an existing compatible browser executable.
The comparison browser check uses original generated PDFs and independent actual
PDF.js rendering for text, rectangle-color-only, small AcroForm-appearance,
crop/rotation/UserUnit and added/removed-page differences. It also checks
file-only metadata/attachment changes without an identity claim, cancellation,
stale/undo-resurrected sources, report shape, source/history/reading-position
preservation, themes, and worker/canvas/blob-URL cleanup. Set
`FOLIO_COMPARISON_ARTIFACTS` to an output directory to retain synthetic fixtures,
screenshots and JSON evidence. Helpers bind loopback-only ports (prefer 1428/
19156, otherwise free alternatives) and stop only their own processes.

Tests cover appearance preference validation, persistence errors, system/native
theme mappings, standard and rotated selection coordinates, phrase search,
Unicode annotation persistence, annotation removal, inherited page properties,
page deletion, destination retargeting, save/reopen, source preservation,
signature read-only handling, undo/redo, stale revisions, and failed opens.
Conversion coverage checks fresh-tab/source/history/draft isolation, real
Unicode page content and pagination, RGB/alpha image resources, strict
UTF-8/BOM/newline rules, unsupported input and exact settings/text limits,
cancel/order/all-or-nothing behavior, real OPC ZIP/CRC/XML, selected DOCX page
order, copy/owner/protected/signed authority, XML controls, no-text results,
draft/edit/undo invalidation and no-clobber output. Run the targeted checks:

```sh
node --test tests/conversion.test.ts tests/output.test.ts tests/draftProtection.test.ts
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --test conversion --test output
```
Additional authoring coverage checks new-document dirty state, embedded text
appearances, Unicode normalization and supported scripts, overflow rejection,
rotated text matrices, independent annotation copies, imported links and
resources, safe extraction, form/signature guards, and page-range/URL parsing.
Page-authoring regressions cover real image pixels and soft masks, palette/
16-bit PNG alpha, JPEG orientation, exact decode limits, metadata removal,
document/revision-scoped image staging, all crop rotations/nonzero origins/
inherited boxes/UserUnit, crop reset after save/reorder, selected-page numbering,
embedded watermark fonts, transactional batch failures, RC4/AES history and
save/reopen, and cropped writing/source-text/redaction geometry.

Existing-image regressions exercise occurrence-local replacement/removal/
movement, shared page streams/resources, nested repeated Forms, appearance
exclusion, crop/rotation/page-local UserUnit, reflections, alpha/orientation,
atomic failures, stale/foreign assets, undo-resurrected revisions, restricted/
signed sources and legacy RC4 save/history/reopen. Pure frontend tests cover
draft protection, exact footprint defaults, numeric limits, the bounded
eight-page inspection cache, and live placement.
To run the real-engine/PDF.js/UI check with original synthetic fixtures:

```sh
node tests/imageEditing.e2e.mjs
```

The runner uses isolated loopback ports (prefer UI 1433/engine 19161), an
isolated debug target and a caller-selected `FOLIO_IMAGE_EVIDENCE_DIR`.
It never launches or stops a native Folio application or creates release
bundles. Use `FOLIO_IMAGE_BROWSER` to select a compatible local Chromium.

To generate original page-authoring fixtures, set `FOLIO_AUTHORING_FIXTURES` to
an empty output directory and run:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test page_authoring write_original_synthetic_authoring_fixtures_when_requested
```

The fixtures include a four-page original with 0/90/180/270-degree rotations,
nonzero origins, inherited crops, differing page sizes and UserUnit 2; an alpha
PNG; an oriented JPEG; and stamped/cropped/reset saved PDFs. Compare the reset
with the stamped view, then try page-2 cropping, numbering `3, 1-2`, save/reopen,
history, draft tab switches, and a redacted export. No user PDFs are used.

Existing-text coverage includes all four text-show operators, repeated text,
shared page streams and nested Forms, original and explicit embedded fonts,
Unicode/Differences encodings, invisible/ActualText guards, deletion,
save/reopen, undo/redo, unambiguous click targeting, and search cache invalidation.
Redaction coverage checks every exported RGB pixel against masks, original
byte/history preservation, removal of source objects and PNG metadata, alpha
channel disposal, rotations/crop offsets/UserUnit, save/reopen, incomplete and
stale exports, cancellation/generation guards, signature/XFA guards, coordinate
validation, source overwrite protection, and exact pixel limits.
Password-protection coverage exercises RC4, AES-128, and AES-256 with user and
owner passwords, incorrect-password retries, cancellation, stale opening
requests, restricted permissions, encrypted history/save/reopen, encrypted
page imports/extraction/redaction, and unchanged signed documents.
Multi-document coverage checks independent histories and revisions, tab
activation and guarded closing, empty-workspace behavior, partial batch-open
failures, per-document redaction jobs, encrypted import target identity, and
cross-tab source/saved-file overwrite protection.

Signature regressions generate synthetic identities at runtime, never commit
private keys, and check successive approval exports, exact original-byte
prefixes, selected field/widget identity, persistent appearances, independent
CMS verification, weak/expired/future/missing/wrong-purpose chains, explicit
anchor removal across tabs, exact limits, draft/stale/cancel guards, and source
history preservation. Run the independent OpenSSL CLI check explicitly (the
application itself needs no external executable):

```sh
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test signature_parity successive_pdfs_verify_every_cms_with_independent_openssl_cli \
  -- --ignored --exact
```

An explicit designer/signing contract check consumes original synthetic
`unsigned-slots-created.pdf/.json` and `unsigned-slots-moved.pdf/.json` fixtures.
It actually signs every slot successively across all four page rotations,
crop/UserUnit and page moves, reopens and checks field/widget identity and all
CMS, and independently verifies every CMS with the OpenSSL CLI:

```sh
FOLIO_DESIGNER_SIGNATURE_FIXTURES=/absolute/fixture-directory \
  FOLIO_SIGNATURE_CONTRACT_OUTPUT=/absolute/new-evidence-directory \
  cargo test --manifest-path src-tauri/Cargo.toml --no-default-features \
  --test signature_parity designer_created_and_moved_slots_sign_successively_when_requested \
  -- --ignored --exact
```

The real browser signing workflow uses local-only synthetic files, loopback
ports 1430/19158 when free (free alternatives otherwise), and its own short-lived
helpers. It never opens or restarts a running native Folio app:

```sh
npx playwright install chromium --only-shell
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=target/signatures npm run test:signatures
```

The browser test requires an `openssl` executable for runtime-only test identity
generation. `FOLIO_SIGNATURE_BROWSER` can select a compatible headless Chromium
binary. Set `FOLIO_SIGNATURE_ARTIFACTS` to an output directory to retain synthetic
PDFs, screenshots, and JSON evidence; test identities/private keys are always
kept in a temporary directory and removed at completion, not copied there.

Document-utility coverage checks Unicode metadata/XMP consistency, preserved
unrelated RDF/Info/name-tree properties, real bookmark destinations and group
history, unrecognized-action preservation, attachment decompression limits,
safe suggested basenames, encrypted document management, actual owner-only
reprotection/removal, AES-256 revision/permission bits, and blocked draft exports.

Generate original runnable document-utility fixtures (no private or third-party
PDFs) with:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example document_utilities_fixture -- /tmp/folio-document-utilities
```

This creates plain, unrestricted-user protected, and restricted-user protected
PDFs with XMP, Unicode bookmarks, and a harmless text attachment. Synthetic
opening/owner passwords are `reader` / `owner`. Open these in native or
preview, change properties/bookmarks/attachments, save/reopen, and try
protection-copy export as both reader and owner. Wrong owner passwords must
not create a copy or change the current document.

Output coverage includes real compression/no-reduction byte counts,
RC4/AES-128/AES-256 preservation, separate print/copy/owner permissions,
signed-source rules, exact UTF-8/ZIP contents, pixel/byte/page limits,
incomplete/stale/canceled jobs, draft gates and no-clobber cross-tab writes.

Generate original synthetic output fixtures (never use private PDFs):

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example output_fixtures -- /absolute/path/to/a/new/fixture-directory
```

The generator refuses to overwrite fixture files. Protected samples use
opening password `user` and owner password `owner`. `output-signed-marker.pdf`
contains an explicitly labeled synthetic signature marker, **not a valid
cryptographic signature**.

For an actual-renderer regression check, run the browser preview, finish any
output dialog, then run in its development console:

```js
await import("/tests/output.browser.ts").then((tests) => tests.checkOutputRenderer())
```

Vite may reload once when first optimizing the test-only `pdf-lib` dependency;
rerun the command after that reload. The check uses an in-memory 24-page PDF
and verifies off-screen text/pixels, rotated crop/UserUnit dimensions, real
PNG/JPEG encoding, print-surface cleanup, cancellation and stale revisions.
It does not open a print dialog or save a user's document.

Manual native/browser output checks:

1. Open `output-24-pages.pdf`, stay on page 1 and prepare all pages at 150 DPI.
   Only a viewport neighborhood is mounted, but the print snapshot has 24
   images. Open the print dialog and inspect pages 1, 2, 3 and 24. The first
   three physical sizes are 612x792, 400x680 and 600x450 points (minor driver
   rounding may apply). Finish/cancel the dialog before clearing the snapshot.
2. Export text for `24, 2, 1`; check that those three actual page strings occur
   in order, separated only by form feeds. Export page 2 as PNG at 150 DPI:
   it is 834x1417 pixels. Export `24, 1` as JPEG: the ZIP contains
   `page-0024.jpg` then `page-0001.jpg`.
3. Measure/save optimization, compare reported bytes with the file's actual
   length, and reopen it. Optimize the already optimized copy: the UI reports
   no reduction and any offered copy is byte-for-byte unchanged.
4. Try `output-print-only.pdf` with `user`: 150-DPI print is available, 300-DPI
   print/image/text export/editing are not. `owner` allows these outputs.
   `output-copy-only.pdf` permits acknowledged image/text export but not
   printing or optimization. `output-protected.pdf` optimized copies reopen
   with both passwords. The signed marker permits read-only content output
   but not optimization.
5. Start a large preparation and cancel; create pending redactions or a
   writing draft and attempt print/save; attempt an existing filename, an
   active source/saved path or another tab's path in the native Save dialog.
   Verify explicit refusal/cancellation, no partial file, unchanged tabs and
   unchanged source/saved/undo state.

The blue app mark is authored in `public/folio.svg`; native icons are generated
with `npm run tauri -- icon public/folio.svg`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, validation,
safe PDF fixtures, and the pull request process.

## License

Except where otherwise noted, Folio's original source code and project assets
are licensed under the [Apache License, Version 2.0](LICENSE).
The full license is included in application bundles as `LICENSE`.

Third-party dependencies and bundled assets retain their own licenses; Folio's
license does not replace those terms. PDF.js includes independently licensed
open-source font/CMap/profile data. Dependency notices are collected in
[`src-tauri/resources/THIRD_PARTY_NOTICES.txt`](src-tauri/resources/THIRD_PARTY_NOTICES.txt)
and included in application bundles as `THIRD_PARTY_NOTICES.txt`.
Review licensing, signing, and document compatibility before distributing a
production release.

The unmodified Noto Sans font is from the
[Noto fonts project](https://github.com/notofonts/noto-fonts/tree/main/hinted/ttf/NotoSans).
Its copyright and SIL Open Font License are in `src-tauri/resources/fonts/OFL.txt`
and included in the application's third-party notices.
