---
name: Folio
description: An independent, document-first PDF workspace.
colors:
  primary: "#225ae6"
  primary-ink: "#174cc6"
  primary-soft: "#ecf2ff"
  surface: "#ffffff"
  canvas: "#e9edf3"
  ink: "#293241"
  muted: "#626d7e"
  divider: "#e7ecf3"
  on-primary: "#ffffff"
  input: "#ffffff"
  chrome: "#f5f7fb"
  panel: "#f7f9fc"
  control-border: "#7e8da3"
  document-hover: "#5a8aef22"
  redaction-mark: "#a32134"
  redaction-wash: "#a3213422"
typography:
  body:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "13px"
    fontWeight: 400
  label:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "12px"
    fontWeight: 600
  detail:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "11px"
    fontWeight: 400
  metadata:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "10px"
    fontWeight: 400
  title:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "23px"
    fontWeight: 600
    letterSpacing: "-0.025em"
  dialog:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "20px"
    fontWeight: 600
    letterSpacing: "-0.02em"
  wordmark:
    fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "23px"
    fontWeight: 750
    letterSpacing: "-0.035em"
rounded:
  compact: "6px"
  control: "8px"
  panel: "10px"
  toolbar: "12px"
  dialog: "16px"
spacing:
  tight: "4px"
  related: "8px"
  inset: "16px"
  section: "24px"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "7px 13px"
    height: "34px"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "#4a5c77"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "7px 13px"
    height: "34px"
  search-input:
    backgroundColor: "{colors.input}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "0 10px"
    height: "36px"
  document-tabs:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.primary-ink}"
    height: "55px"
  count-badge:
    backgroundColor: "#eef1f6"
    textColor: "#62718a"
    rounded: "{rounded.compact}"
    padding: "0 5px"
    height: "19px"
  page-card:
    backgroundColor: "#e9eef6"
    textColor: "#576c8b"
    rounded: "{rounded.panel}"
    padding: "16px 8px 8px"
  appearance-picker:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.toolbar}"
    width: "296px"
  writing-setting:
    backgroundColor: "{colors.input}"
    textColor: "{colors.ink}"
    typography: "{typography.detail}"
    rounded: "{rounded.control}"
    padding: "5px 9px"
    height: "33px"
---

# Design System: Folio

## Overview

**Creative North Star: "Document-first workspace"**

Folio uses familiar desktop PDF-editor conventions with its own identity.
The paper is the work surface, not a decorative preview inside a dashboard.
Tools, document navigation, and review controls stay in predictable places.

The default light palette suits reviewing local documents at a normally lit
desk; Dark, Sepia, and High Contrast extend the same workspace to other
conditions. Actions, selection, and keyboard focus keep their semantic roles
as the palette changes. The document well separates paper from the application.

**Key Characteristics:**
- Familiar PDF-editor organization with independently authored branding.
- Compact system typography and consistent outline icons.
- Clear selected, disabled, loading, error, and unsaved states.
- Real document content rather than simulated page artwork.

## Colors

The frontmatter records the Light palette. All appearance palettes live in
`src/themes.css`; `src/styles.css` consumes semantic surface, text, action,
border, focus, and status roles. The legacy `--blue` names are action tokens,
not a requirement that every theme use a blue accent.

The action accent owns save/export, active tools, and selected page outlines.
Dedicated foreground tokens keep text readable on accent-filled buttons.
Surface, canvas, ink, and muted roles provide hierarchy in every palette;
the High Contrast theme uses a separate amber focus indicator.

| Palette | Surface | Document well | Main text | Action accent |
| --- | --- | --- | --- | --- |
| Light | `#ffffff` | `#e9edf3` | `#293241` | `#225ae6` |
| Dark | `#202630` | `#131820` | `#e8edf5` | `#91b9ff` |
| Sepia | `#f7f1e7` | `#dcd3c5` | `#40352a` | `#805028` |
| High Contrast | `#080d14` | `#000000` | `#ffffff` | `#8bdeff` |

System is a persistent preference that resolves to Light or Dark and follows
device changes. Native application chrome uses the corresponding light/dark
appearance. Do not invert the interface mechanically or recolor a PDF.

Yellow identifies highlights and note markers. Amber read-only notices,
red error/destructive states, and green saved-state feedback communicate
meaning, not interchangeable decoration.

**The Document Boundary Rule.** App selection colors and UI tokens must not
recolor the user's PDF. Document fonts, artwork, and existing annotations
belong to the document.

## Typography

The system font stack is used throughout the interface. No remote font
stylesheet or extracted Adobe font is required.

The base UI size is 13px. Action labels use 12px, contextual headings 13px,
the organize heading 23px, and dialogs 20px. Compact metadata and page
numbers use 10px and 11px text with tabular numerals where relevant.
Do not reuse metadata sizing for primary actions.

## Page authoring

Use one contextual Page authoring panel for Image, Crop, Watermark, and Page
numbers. Its compact two-column tool selector inherits the existing outlined
control/selected-action vocabulary; do not introduce a second visual system.
Group dimensions and related settings in paired fields with tabular numerals.
Keep apply/discard actions outside the settings scroll area. Inline errors and
success feedback must not cover those controls, including in narrow previews.
Keyboard numeric entry is an equal alternative to page placement and dragging.

Draft image previews and crop edges are document-blue, temporary review UI,
not saved geometry. Crop-outside shading is neutral slate. Never animate PDF
rectangles, image dimensions, crop edges, thumbnails, or changes in paper size.
Clip all on-page overlays to the actual visible page and omit off-page note
and link targets. Preserve the existing continuous-page reading anchor.

Explicitly distinguish non-destructive cropping from redaction and ordinary
watermarks/numbers from tamper-proof security. Name current-page/range targets,
physical-point measurements, and static numbering behavior. Drafts belong to
the document workspace so tab switching and virtualized pages do not erase
uncommitted input. Keep the established appearance themes, focus treatment,
reduced-motion behavior, and native/preview distinction.

## Existing-image editing

Edit existing images extends the same contextual tools panel at 300px, not
the shell navigation. List actual page-content occurrences with pixel size,
operation identity, selection state and readable read-only reasons. Match
each item to a keyboard-focusable on-page outline; exclude annotation/widget
appearances. Keep temporary selection/draft bounds document-blue on white
paper, with a labeled corner resize handle, numeric physical-point alternatives
and arrow-key movement. Never animate PDF geometry or pixel changes.

Preserve the current footprint and orientation when choosing a replacement;
label stretching when aspect ratios differ. Show a sanitized replacement
thumbnail separately, not a false alpha composite over the original. The
on-page outline previews bounds only; original pixels remain until Apply.
Removal has an explicit marked-for-removal
state followed by an explicit Remove image occurrence action. Keep errors and
Apply/Discard in a fixed footer above the panel edge; scroll settings separately.
Retain the complete draft through panel/tab changes and page virtualization.
A workspace banner returns to unapplied image edits from another panel.

Inherit system type, all five themes, semantic chrome colors, visible focus
and reduced-motion behavior. Unsupported clips, transforms, transparency and
optional content stay honestly read-only. Ordinary image replacement/removal
does not securely erase old bytes or undo history; keep this distinction near
the controls, rather than implying redaction.

## Protected document opening

Use the existing focused dialog for password entry, not a new workspace or
sign-in page. Name the selected PDF, mask the input, allow an empty opening
password, and offer explicit Unlock PDF and Cancel actions. Show incorrect
passwords inline, return focus to the field after each attempt, and preserve
the existing document until unlocking succeeds. Passwords are not preferences.

Use the established informational palette for an editable protected-document
notice; keep amber read-only notices for restricted permissions and signatures.
PDF saves and PDF exports retain protection. Image/text export requires a
separate explicit acknowledgment that these files are unencrypted and lose
PDF permissions/signatures. Redaction review must disclose that
encryption parameters, including a legacy file identifier where required,
are retained even though original document content and metadata are removed.

## Document tabs

The title bar contains a horizontally scrollable tab strip. Give each PDF a
named tab, an unsaved indicator when needed, and a separate close button;
never nest the close button inside the tab's selection button. The active
tab uses the workspace surface and inactive tabs use title-bar tones. Keep
the add-document action outside the scroll area so it stays available.

Use tab/tabpanel semantics, roving keyboard focus, arrow/Home/End navigation,
and visible focus. Preserve each document's working/view state independently.
Opening PDFs is additive, not a destructive navigation requiring discard.
Close decisions name the affected PDF; quitting reviews unsaved inactive tabs
as well. An empty workspace still offers opening and document creation.

## Local file conversion

Keep conversion inside the established local document-first workspace. A
visible title-bar Create action and All tools entry open the focused
Create PDF from files dialog; Command/Control+Shift+N is the keyboard
alternative. Keep the source-kind chooser, ordered filename list with
named Up/Down buttons, physical layout controls, limits and ready/error/
canceled feedback in one scrollable body. Header/footer actions stay visible
at desktop and compact sizes. Reuse semantic theme/focus/status tokens and
the existing reduced-motion dialog behavior. Preparation and opening are
distinct: Open new PDF creates a new tab, not a source mutation.
Place loading/errors and the completed PDF's page/size/layout summary at the
top of the body; do not hide readiness below the file/layout controls.
Keyboard-scrollable dialog bodies use the semantic focus color, including
High Contrast's amber outline, rather than the browser's default blue.

Export Word text uses the existing separate-output dialog and All tools/
native File menu. Name it Word document content, with a prominent
text-conversion/not-layout disclaimer, explicit ordered ranges, protected
source acknowledgment, cancelable progress, ready text summary and Save DOCX
copy footer. Empty scans and unsupported text are actionable errors, never
success-shaped empty exports. Conversion settings must not become document
drafts or silently clear any existing draft.
Keep DOCX progress, errors and ready text summaries near the beginning of the
dialog, while the limitations and protection boundary remain visible and
readable. Move focus to the current primary/cancel action as preparation
changes state, without moving the source document.

## PDF digital signatures

Digital signatures extends the existing tools panel and utility rail without
adding a workspace mode. Its scrollable inspection list separates signed-byte
integrity, earlier-revision coverage, and certificate dates from explicit
custom-chain, revocation, and trusted-time results. Green means intact bytes
only, never a trusted signer. Custom-anchor success is a text conclusion in
the document-blue palette, not an authoritative identity badge. Unsupported and later-change states use the warning palette;
failed integrity checks use the error palette. Certificate details disclose
subject/issuer, current date bounds, serial, algorithms, and a selectable
SHA-256 fingerprint for manual comparison.

Signed-copy export uses the protected dialog for local identity selection,
masked password entry, and explicit review acknowledgment. Clear submitted
passwords and keep errors actionable without losing the chosen identity.
Explain the default invisible approval field, selected empty field/widget
identity and page location, persistent visible appearance versus mathematical
validation, unverified local time, included unsaved unsigned edits, and unchanged
sources. Co-signing names earlier-revision coverage and the exact-prefix append
policy without enabling general signed-file editing. All pending drafts/redactions
block export rather than signing an unapplied original.
The exported file is separate; success must not imply that the
working tab is signed or saved. Reuse all themes, focus, responsive overlays,
existing panel/dialog motion, and reduced-motion behavior.

Custom CA management is a disclosure inside this panel, not a new workspace
mode or global-trust setting. Explain explicit public-root selection, bounded
input, memory-only lifetime, shared tabs, and removal beside the controls.
Never imply AATL/EUTL, OS/keychain, revocation, or trusted timestamp assurance.
Clear all panels' cached custom-chain conclusions on store changes, including
inactive tabs and stale async responses.

PDF page typography is rendered from the file itself. The generated example
PDF uses standard PDF fonts; it is not the interface's typography source.
The existing-text edit hover uses a translucent document-blue overlay, not
the surrounding workspace palette; it is temporary UI and is never saved
into the PDF.

Pending redaction areas use a fixed document-red dashed outline (`#a32134`)
and translucent wash (`#a3213422`), with a white numbered label. These are
review UI on white paper, not applied black rectangles. The exported copy
uses opaque black pixels, independent of the workspace theme.

## Layout

The full desktop shell has a 52px title/document bar, a 55px action bar,
and a 29px status bar. The central workspace fills the remaining height.
The tool panel is 226px wide; search/comments use 274px. The thumbnail
rail is 157px, and the utility rail is 43px.

The document scroll area owns page overflow, including at high zoom.
Pages form a continuous vertical stack with 24px gaps and their own dimensions.
Only the viewport neighborhood mounts full renderers; off-screen page positions
are reserved by geometry, not miniature document screenshots. Page navigation
scrolls within the stack, and the counter/thumbnail selection follows the most
visible page. Resizing and zoom retain the reading anchor. Page Up/Down scroll
the viewport; Left/Right remain page navigation. The trailing workspace allows
even a short final page to align with the top when explicitly navigated to.
The page and zoom toolbar stays near the bottom of that area. A floating
vertical review toolbar groups selection/panning separately from annotations.
The mode bar now includes Read, Write, Comment, and Organize. Writing settings
replace the side-panel content without replacing the document canvas.

At 1199px and below the shell becomes denser. Tool panels become overlays
at 899px and below. Below 700px, the browser preview uses icon-only controls
with accessible names and an explicitly toggled thumbnail overlay. Entering
a compact layout closes panels rather than leaving them invisibly active.
Below 480px the title and mode toolbars tighten further so appearance, help,
history, and save controls remain in bounds in a narrow embedded preview.
The native desktop window has an 820px minimum width; narrow browser
previews do not imply a native mobile product.

## Elevation & Depth

Most chrome is flat, separated by subtle borders and tonal changes.
The paper, floating controls, and protected dialogs are the elevated layers.
Theme-specific shadow colors keep elevation appropriate to light and dark
surfaces; the following values describe the Light baseline.

- Paper: `0 2px 5px #24324816, 0 8px 26px #24324812`.
- Page/zoom controls: `0 8px 24px var(--shadow-soft), 0 1px 3px var(--shadow-soft)`.
- Dialog: `0 20px 64px var(--shadow-strong), 0 4px 16px var(--shadow-soft)`.
- Compact thumbnail drawer: `-6px 0 18px var(--shadow-soft)`.

Light chrome uses `#f5f7fb`, with `#f7f9fc` secondary surfaces. Light shadows
use `#182b4514` and `#182b4533`; Dark uses `#0000003d` and `#00000080`.
These are bounded, offset elevation cues, not glass, blur, or accent glows.

Shadows distinguish interaction layers, not generic content cards.

## Shapes

Standard controls use 8px corners; utility rows and page cards use 10px.
Floating controls use 12px and protected dialogs 16px. Compact numeric fields
and metadata badges use 6px. Shared CSS radius tokens own these roles.
Document paper remains rectangular; document annotations, miniature theme
swatches, and the authored brand mark retain their domain-specific shapes.
The document tab rounds only its top corners.

Icons use Lucide's consistent outline vocabulary, with a 1.8px stroke.
The original Folio mark is authored in `public/folio.svg`; native raster
icons are generated from that source.

## Components

Primary and secondary buttons share density and spacing. Primary actions
are blue; secondary actions are white with an outlined boundary. Icon-only
buttons carry explicit names and tooltips, including when text is hidden.
Tool rows are 40px tall; the retained instructional tip is compact enough
to keep keyboard help reachable without shrinking action typography.
Comment edit/remove buttons provide 26px by 28px targets.

The search field is a compact outlined group, with a visible focus-within
state. The mode navigation uses a softly filled active segment, blue underline,
and text/icon state;
color alone does not identify the current mode. Page thumbnails use both
an outlined selection and a selected page-number treatment.

The writing panel reuses secondary buttons and outlined setting fields for
text size, pen width, and on-document color. The page-authoring action row
wraps at narrow widths; it does not add a second navigation hierarchy.
Bookmarks are an indented list of standard buttons, with a compact selected-item
toolbar for renaming, sibling movement, nesting/outdenting, and group deletion.
Add-bookmark dialogs select a real page and parent; keep original targets when
renaming/organizing. Document links are
transparent hit targets over actual PDF geometry, with visible keyboard focus.
External links require an explicit confirmation that displays the address.

Write defaults to the existing-text editor. The panel lists source runs with
their font and size; unsupported entries explain why they cannot be changed.
Rendered PDF text gains keyboard-focusable edit targets only while Edit text
is active. Ambiguous clicks defer to the source list rather than guessing.
The replacement dialog uses the existing textarea/select/button system and
offers original-font preservation or an explicit embedded-font change.
Deletion drafts participate in exit protection even when the replacement
field is empty.

Redact uses the existing tool panel, floating toolbar, and protected dialog
system. Rectangle dragging is complemented by a keyboard-accessible coordinate
form. Review content scrolls above a fixed export footer; compact overlays
offer Mark on page to reveal the work surface, without a tip covering PDF
content. A persistent notice identifies the original and unapplied marks. While
marks exist, source edits/history/page operations are locked; Save and its
native/keyboard paths open redaction review instead of exporting the original.
The export dialog explains whole-document rasterization, requires an explicit
acknowledgment, offers 150/300 DPI, and shows cancellable page progress.
Export creates a separate PDF; it does not clear marks or imply that the
original shown in the window was redacted. Leaving or clearing marks requires
an explicit protected decision.

Text-entry dialogs retain their contents and display errors inline when a
PDF operation is rejected. Uncommitted note/text drafts participate in exit
protection. The on-page ink preview is a live rendering of the user's stroke,
not decorative artwork; it commits as a PDF annotation when the pointer lifts.

Forms extends the existing contextual panel, not the mode hierarchy. The
300px panel uses labeled native text/password/checkbox/radio/select controls,
qualified names, required/read-only states, field filtering, and page links
for repeated widgets. Signature slots have no text-entry affordance.
Values remain document-workspace drafts while panels/pages unmount. A visible
Unapplied label and banner distinguish those values from the rendered PDF.
Keep Apply form changes and confirmed Discard actions in a fixed footer.
Errors preserve drafts; save/history/source mutations and exit protection
must not quietly ignore them. Explain the embedded-font and unsupported
script/calculation/XFA boundaries without adding placeholder controls.

Design form fields extends the contextual tools panel at 310px, with native
labeled controls, a fixed Apply/Discard footer and practical physical-point
geometry. Separate logical-field properties from individual page widgets.
Use blue, keyboard-focusable page targets and clearly labeled draft outlines;
click places a widget and dragging changes its rectangle, never PDF pixels.
Numeric geometry provides the complete keyboard alternative. Preserve all five
palettes and reduced-motion behavior without adding document animation.
New signature controls say Unsigned signature field and are empty slots, not
drawn-signature claims. Designer settings live in DocumentWorkspace through
virtualization and panel/tab changes; all pending states share the existing
aggregate draft projection and synchronizer. Errors preserve input. Static-copy
export uses a focused confirmation of counts, masking, interactivity loss and
unsupported features; it never claims secure sanitization or replaces the
interactive original. Current values and defaults remain distinct.

Properties and attachments extend the existing 274px contextual panel, not
the shell's navigation hierarchy. Descriptive metadata uses labeled fields
with explicit Apply/Discard actions. Retain drafts at DocumentWorkspace level,
including cleared fields, across panel/tab changes; save/close/export must not
silently omit them. Explain XMP consistency failures inline instead of silently
keeping old metadata elsewhere. Inspectable metadata is not a sanitization claim.

Attachments show actual names and bounded decoded sizes, with user-driven Add,
Save, and Remove controls. Confirmation explains that payloads are untrusted,
never auto-opened/executed, and removal from the list is not secure erasure.
Use the same focused native save dialogs or explicit browser downloads, with
sanitized suggested basenames and no browser-preview filesystem-path API.

Export protection copy uses the protected dialog system: explicit separate
output, current owner authorization where needed, masked/confirmed new
passwords, and honest printing/copy/edit/annotation/form/assembly permissions.
Use AES-256 revision 6, clearly explain optional opening passwords and owner
override, and distinguish cooperative restrictions from absolute DRM. Wrong
passwords/cancellation preserve source/tab state. Pending drafts/redaction
marks block export; ordinary saves continue preserving protection.

Appearance is a nonmodal title-bar popover, not an interrupting settings
dialog. Its standard radio group supports keyboard navigation, visible
selection, Escape, outside dismissal, and focus restoration. Choices persist
locally; storage failures state that the theme applies only to the current
session and offer an explicit retry.

Dialogs are reserved for notes, protected destructive/unsaved decisions,
explicit output preparation/security review, and requested help/about content. They trap focus, support Escape where
safe, and restore focus on dismissal.

Print, image/text export and lossless optimization use the existing protected
dialog, themed setting fields and fixed action vocabulary, not another shell.
All tools and the native File menu expose the same operations; a compact
print icon and Command/Control+P lead to print settings. Scope, resolution and
quality stay per-document, above virtualized page renderers. Preparation shows
cancellable selected-page progress. Protected-source image/text export has an
unencrypted-output acknowledgment before preparation. Optimization reports
exact before/after byte counts and a plain no-reduction state.

Print is explicitly raster-based. A print-only surface outside the workspace
contains every selected page image with its renderer-derived physical size.
Never print the normal workspace, current viewport, chrome, comment popups or
password UI. The dialog says Print dialog requested rather than claiming the
driver printed. Keep the snapshot and URLs until the user explicitly finishes
the print dialog; disable Escape/outside dismissal while it is retained.
Unsupported geometry/printing, stale jobs, limits, cancellation and native
filename collisions are explicit states. Preserve documents and all tabs.

Focus uses a 2px outline with a 3px offset. Its color follows the palette,
with amber in High Contrast; controls on a PDF retain a blue focus indicator
that is visible against white paper. Palette changes switch text and surfaces
atomically, without interpolating through low-contrast intermediate colors.

## Document comparison

Extend the existing document-first workspace with a persistent comparison
surface below the unchanged tab bar, not a modal or a new shell. A compact
settings/indexed-page sidebar sits beside the document well; Return to PDFs and
the tab strip preserve access to normal document work. Inherit system type,
semantic theme tokens, outlined controls, keyboard focus and all five themes.
Use the existing 20px dialog heading step for the comparison title and summary.

Before/After selectors name open tabs and unsaved state. Explicit page-index
matching, DPI/tolerance, pending-input errors, cancellation and stale results
must remain readable. Never say identical: no observed differences is limited
to compared page text, pixels and geometry. Added/removed labels describe
unmatched indexes, not inferred middle insertions or moved pages.

Show real white-paper previews side by side or as a top-left-aligned opacity
overlay. The fixed magenta changed-pixel mask (`#be1464`, RGB 190/20/100) is
comparison markup on white paper, not a theme accent or a PDF mutation; label
it explicitly and allow it to be hidden. The missing-page placeholder likewise
uses fixed slate `#4a5c77` on white paper, not a theme-dependent light foreground.
Text insertion/deletion uses themed success/danger foreground/background pairs
plus underline/strike so color is not the only signal. Never animate PDF pixels,
mask geometry, paper size or opacity changes. Protected-source report export
uses an explicit unencrypted-content acknowledgment; no implied document trust
or whole-file verification.

## Motion

**Thesis:** the selected work surface travels; the document remains still.
The focal interaction is a 260ms shared selection surface that glides between
PDF tabs. Read/Write/Comment/Organize and the review-tool dock echo that
continuity more quietly. Move the selection paint, never the control targets
or PDF pixels. Use `cubic-bezier(0.16, 1, 0.3, 1)` for confident deceleration,
not elastic overshoot.

`SelectionIndicator.tsx` measures final geometry once and uses Web Animations
FLIP transforms rather than animating layout properties. Interrupted travel
starts from the currently visible position. Resizing snaps to accurate bounds;
hidden workspaces stop moving. The legacy selected styling remains the static
fallback. All marker surfaces use the current palette and ignore pointer input.

Panels use a 240ms cropped reveal with an 8px lateral settle only when opened
or changed. Dialogs settle 10px at 0.985 scale with a bounded shadow transition;
they stay fully opaque and focusable throughout. The appearance popover opens
from its trigger, with 180ms icon/label settles spaced 16ms apart, capped at
64ms total delay. Radio targets and focus rings do not move.

Supporting feedback uses 140ms colors/borders and 100ms press feedback.
The add-document icon turns without shifting its target; tool arrows lean
toward their destination. New tab labels crop into view, unsaved dots appear
once, and successful notifications draw their check mark. Busy feedback is
shown only after 160ms, avoiding a spinner flash for fast tab switches.
No action, document state change, or keyboard interaction waits for animation.

The work is bounded to chrome: no page/canvas/thumbnail animation, scroll
reveal, staggered tool rows, animated width/height, permanent will-change,
decorative loops, or new dependency. Live Reduce Motion changes cancel
selection travel immediately and remove spatial/keyframe effects and smooth
search scrolling. Static selection, focus, status text, and 80ms color feedback
remain. Theme colors still switch atomically without low-contrast crossfades.

## Do's and Don'ts

### Technical preflight and archival copies

Extend the existing Operate surface, never replace the document well with a
compliance dashboard. All tools and the native File menu expose Technical
preflight and PDF/A-2b archival copy as separate actions. The inspection panel
lists concrete observations with explicit fact/absent/unsupported/unchecked
states, scope and source revision; no green "compliant" badge is inferred from
tag flags or XMP declarations. The JSON report is a separate, permission-checked
disclosure, not a certificate.

Archival export uses the existing focused dialog, semantic warning/input/action
tokens, native keyboard focus and fixed visible footer actions. Put the
irreversible structure/signature losses before the acknowledgment. Show the
explicit page selection, 150/300-DPI choice, owner/unencrypted boundary,
cancellable rendering progress, exact resource limits and source invariance.
The dialog body scrolls on compact windows without hiding its actions. Reuse
all five themes and existing reduced-motion behavior; PDF pixels and geometry
never animate. Generated archival copies open read-only, with editing directed
back to the original rather than silently retaining a conformance declaration.

### Do
- Do keep the document as the primary work surface.
- Do use semantic action, selection, and focus colors consistently in every palette.
- Do preserve accessible names when compact layouts hide visible labels.
- Do show actual document state and actionable failures.
- Do keep app chrome independent of the PDF's fonts and artwork.

### Don't
- Don't use Adobe branding or assets extracted from Acrobat.
- Don't present unsupported Acrobat features as working controls.
- Don't replace PDF rendering with a static document mockup.
- Don't hide a panel in CSS while leaving its toggle unable to reveal it.
- Don't imply cloud upload, autosave, notarization, or complete Acrobat parity.
