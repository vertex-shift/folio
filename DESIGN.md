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
Bookmarks are an indented list of standard buttons. Document links are
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

Appearance is a nonmodal title-bar popover, not an interrupting settings
dialog. Its standard radio group supports keyboard navigation, visible
selection, Escape, outside dismissal, and focus restoration. Choices persist
locally; storage failures state that the theme applies only to the current
session and offer an explicit retry.

Dialogs are reserved for notes, protected destructive/unsaved decisions,
and requested help/about content. They trap focus, support Escape where
safe, and restore focus on dismissal.

Focus uses a 2px outline with a 3px offset. Its color follows the palette,
with amber in High Contrast; controls on a PDF retain a blue focus indicator
that is visible against white paper. Palette changes switch text and surfaces
atomically, without interpolating through low-contrast intermediate colors.

## Motion

**Thesis:** tools reveal context; the document remains still. The authored
moment is a 200ms side-panel mask reveal with a 5px lateral settle, using
`cubic-bezier(0.16, 1, 0.3, 1)`. It runs only when a panel opens or changes
purpose, never when a page scrolls or a source-text list refreshes.

Supporting feedback uses 140ms color/border transitions and 100ms press
feedback. Only the button's icon/label moves by 1px; its hit target does not
move or shrink. Popovers settle from 3px above; protected dialogs settle 5px
with a 0.99 scale. Dialog surfaces stay opaque throughout; the scrim changes
separately. Notifications use a short opacity acknowledgment.

The work is bounded to chrome; no page/canvas animation, scroll reveal,
staggered tool rows, width/height animation, persistent will-change, or motion
dependency. Reduced Motion removes spatial/keyframe animation and smooth
search scrolling, preserving 80ms color feedback and all visible state cues.

## Do's and Don'ts

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
