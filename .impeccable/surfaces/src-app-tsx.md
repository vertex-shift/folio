---
version: 1
slug: "src-app-tsx"
primary_target: "src/App.tsx"
related_targets: ["src/DocumentWorkspace.tsx", "src/styles.css", "src/themes.css", "src/components/AppearancePicker.tsx", "src/components/DocumentTabs.tsx"]
---

# PDF workspace

Mode: Operate. A macOS-first desktop PDF reader and reviewer, running in Tauri.
Scope: the confirmed MVP, not full Acrobat parity. The user pinned an
Acrobat-familiar interface, so no alternative visual-world tournament is needed.

## Direction contract

THESIS: Put the document first in a familiar, independently implemented PDF
workspace. No dashboard or marketing page between the user and the paper.

OWN-WORLD: Cool white and pale slate surfaces, a slate document well, a
clear blue action color, and system UI typography. Original blue folded-page
Folio mark and consistent Lucide icons; no Adobe fonts, assets, or branding.

STORY: Open a real PDF, find a passage, highlight or comment, organize pages,
and save an interoperable edited copy. A visible change must survive reopening.

FIRST VIEWPORT: A 52px document/title bar above a 54px action toolbar.
A 226px left tools panel, centered white PDF page on a slate canvas, and a
compact right page-thumbnail rail. A floating review toolbar offers select,
highlight, and note. Contextual search or comments replaces the left tool
panel, never the document. The signature move is one-step passage-to-highlight,
with its saved annotation appearing immediately in the review list.

FORM: User-pinned category convention: Acrobat-familiar desktop PDF workspace,
with independently authored identity and components. No seed key: the pinned
direction uses the Operate mode rules directly. Code-first; no image generation
is available or required for document-editor chrome.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

Use scene: a person reviewing local documents at a normally lit desk.
The light, high-contrast controls keep tools readable; the darker canvas
separates application chrome from the white document.

Constraints: keyboard access, explicit load and error states, visible dirty
state, protected originals, real annotations and page mutations, no copied
Adobe UI resources, no unsupported features presented as working actions.

## Appearance extension

The user requested dark mode and additional themes. The existing workspace
now offers System, Light, Dark, Sepia, and High Contrast through a nonmodal
title-bar picker. System follows live device changes; explicit choices
persist locally and synchronize native light/dark application chrome.
Semantic color roles change without changing composition, PDF pixels,
annotation colors, document bytes, or document dirty state.

## Modern polish and prominent motion

Refine, do not replace, the incumbent document-first world. Unify controls
at 8px corners, rows/cards at 10px, floating tools at 12px, and protected
dialogs at 16px. Use cleaner Light/graphite Dark chrome, restrained offset
elevation, compact system typography, and a tonal active mode segment.
Preserve all five themes, factual copy, tool locations, and PDF pixels.

Motion thesis: the selected work surface travels while the document stays
still. The user requested visibly stronger motion: the shared selection now
travels for 380ms with temporary accent-edge emphasis, echoed by modes and
review tools. Interrupt from the current visual position and clean up emphasis
and compositing hints. Supporting panels arrive from 36px away in 380ms;
dialogs stay opaque while settling 32px from 0.94 scale with bounded depth.
The appearance picker opens from 20px/0.92 scale and its icon/label sequence
has at most 128ms of sibling delay; radio targets retain immediate focus.
The empty workspace's original page mark unfolds once in 560ms.
Colors respond in 140ms and press feedback in 100ms. Success checks draw once;
busy feedback waits 160ms so routine tab switching does not flash a spinner.
No layout-driving animation, decorative loops, PDF/thumbnail reveals, or new
dependencies. Live Reduce Motion cancels travel and removes spatial/keyframe
effects while retaining static selection and 80ms color feedback. Theme
switching remains atomic for contrast.

## Read/write authoring extension

The user asked to continue toward Acrobat read/write parity. This milestone
adds new PDFs, placed text and ink, page import/duplication/extraction, document
bookmarks, and links. Extend the established workspace and themes, not its
identity. Write and Organize expose only working operations. Clearly distinguish
editable text annotations from rewriting existing content, and preserve drafts
when validation fails. Actual PDF save/reopen is the acceptance condition;
unsupported forms, signatures, scripts, and complex structures remain explicit.

## Existing-text editing

The user explicitly requested edits to text already in a PDF. Write now
opens Edit existing text: page text is clickable and keyboard-focusable,
and a source-run list handles split or ambiguous content. The dialog changes
the selected content-stream operation, never an annotation overlay. Keep
original font/size/position/color when the font can encode the replacement;
make replacement-font changes explicit. Require save/reopen, search, undo,
and shared-resource isolation as evidence. Surface unsupported formats and
non-reflowing layout limits without losing the user's draft.

## Continuous reading

Stack real pages vertically without changing the document-first world.
Keep 24px page gaps and a stable reading anchor through zoom and panel
resizing. Render only the viewport neighborhood. Scroll updates the page
counter and thumbnail; page entry, bookmarks, and search scroll to targets.
Every editing/annotation/redaction event must carry its originating page,
including when a different page occupies most of the viewport.

## Permanent redaction

Add rectangle redaction without confusing a pending overlay with secure
removal. Pending marks remain visibly red and numbered, with the original
clearly identified. A coordinate form provides keyboard access. Export
rebuilds every page as a lossless image and burns reviewed areas black; the
copy retains no original text layer, attachments, metadata, or hidden PDF
objects. Whole-document rasterization and loss of search/selection/vector
quality must be explicit and acknowledged. Keep the source, unsaved work, and
history unchanged. Lock source mutations while marks exist, route all Save
paths through review, protect marks on exit, and retain them after failure,
cancellation, and export. Verify exported pixels and absence of original
objects, not merely the appearance of the on-page overlay.

## Protected PDFs

Reuse the focused dialog and themed input controls for local PDF passwords.
Mask and clear submitted passwords, show actionable retries, and preserve the
current document on cancellation. Editable protected PDFs get an informational
notice; signed/restricted PDFs retain the read-only notice. Saves and exports
keep existing protection; redaction review discloses required retained
encryption parameters and legacy file identifiers.

## Multi-document tabs

Make the existing title-bar tab affordance real: a scrollable strip with
independent tab selection, close controls, unsaved indicators, and an add
action outside the scroller. Preserve per-document working/view state and
render only the active document's canvases. Support multi-file picker/drop/
Finder opening and independent password prompts. Closing is document-scoped;
quitting protects unsaved inactive documents too. Preserve the incumbent
palette, compact typography, and reduced-motion behavior.

## PDF page-label metadata

Extend the incumbent Operate surface with a focused numbering-section editor
from All tools and Organize. Roman front matter, numbering restarts and Unicode
prefixes are real PDF metadata, not new printed annotations. Keep a fixed
Apply/Cancel footer, native labeled fields, explicit error retention and the
single aggregate document draft/protection authority.

The reader and page rail show logical labels with physical-position context.
Explicit Label/Page navigation avoids guessing duplicate, numeric or `#`
labels. Labels follow existing page identities through organization/extraction;
unsupported trees display a notice rather than being silently replaced.
Preserve pixels, themes, motion and unsaved user work. No new raster assets.
