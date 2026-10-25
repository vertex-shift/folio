---
version: 1
slug: "src-app-tsx"
primary_target: "src/App.tsx"
related_targets: ["src/styles.css", "src/themes.css", "src/components/AppearancePicker.tsx"]
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

## Modern polish and subtle motion

Refine, do not replace, the incumbent document-first world. Unify controls
at 8px corners, rows/cards at 10px, floating tools at 12px, and protected
dialogs at 16px. Use cleaner Light/graphite Dark chrome, restrained offset
elevation, compact system typography, and a tonal active mode segment.
Preserve all five themes, factual copy, tool locations, and PDF pixels.

Motion thesis: contextual tools reveal their relationship to the document,
while the document itself stays still. The focal moment is a 200ms panel
mask/lateral settle; supporting colors respond in 140ms and press feedback
in 100ms. Animate icon/label paint, not control hit boxes or PDF geometry.
No layout-driving animation, decorative loops, page reveals, or dependencies.
Reduce Motion removes spatial movement and smooth scrolling but retains
80ms color feedback. Theme switching remains atomic for contrast.

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
