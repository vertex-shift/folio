---
name: Folio
description: An independent, document-first PDF workspace.
colors:
  primary: "#225ae6"
  primary-ink: "#174cc6"
  primary-soft: "#ecf2ff"
  surface: "#ffffff"
  canvas: "#e4e7ec"
  ink: "#293241"
  muted: "#626d7e"
  divider: "#e3e7ed"
  on-primary: "#ffffff"
  input: "#fcfdff"
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
  control: "5px"
  panel: "6px"
  toolbar: "9px"
  dialog: "12px"
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
    rounded: "{rounded.control}"
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
| Light | `#ffffff` | `#e4e7ec` | `#293241` | `#225ae6` |
| Dark | `#202630` | `#11151b` | `#e8edf5` | `#91b9ff` |
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

## Layout

The full desktop shell has a 52px title/document bar, a 55px action bar,
and a 29px status bar. The central workspace fills the remaining height.
The tool panel is 226px wide; search/comments use 274px. The thumbnail
rail is 157px, and the utility rail is 43px.

The document scroll area owns page overflow, including at high zoom.
The page and zoom toolbar stays near the bottom of that area. A floating
vertical review toolbar groups selection/panning separately from annotations.
The mode bar now includes Read, Write, Comment, and Organize. Writing settings
replace the side-panel content without replacing the document canvas.

At 1199px and below the shell becomes denser. Tool panels become overlays
at 899px and below. Below 700px, the browser preview uses icon-only controls
with accessible names and an explicitly toggled thumbnail overlay. Entering
a compact layout closes panels rather than leaving them invisibly active.
The native desktop window has an 820px minimum width; narrow browser
previews do not imply a native mobile product.

## Elevation & Depth

Most chrome is flat, separated by subtle borders and tonal changes.
The paper, floating controls, and protected dialogs are the elevated layers.
Theme-specific shadow colors keep elevation appropriate to light and dark
surfaces; the following values describe the Light baseline.

- Paper: `0 2px 5px #24324816, 0 8px 26px #24324812`.
- Page/zoom controls: `0 4px 14px #25334f14`.
- Dialog: `0 15px 65px #0b23454a`.
- Compact thumbnail drawer: `-6px 0 18px #25334f14`.

Shadows distinguish interaction layers, not generic content cards.

## Shapes

Standard controls use 5px corners; utility panels and page cards use 6px.
Floating view controls use 9px and dialogs 12px. Document paper remains
rectangular. The document tab rounds only its top corners.

Icons use Lucide's consistent outline vocabulary, with a 1.8px stroke.
The original Folio mark is authored in `public/folio.svg`; native raster
icons are generated from that source.

## Components

Primary and secondary buttons share density and spacing. Primary actions
are blue; secondary actions are white with an outlined boundary. Icon-only
buttons carry explicit names and tooltips, including when text is hidden.

The search field is a compact outlined group, with a visible focus-within
state. The mode navigation uses a blue underline plus text/icon state;
color alone does not identify the current mode. Page thumbnails use both
an outlined selection and a selected page-number treatment.

The writing panel reuses secondary buttons and outlined setting fields for
text size, pen width, and on-document color. The page-authoring action row
wraps at narrow widths; it does not add a second navigation hierarchy.
Bookmarks are an indented list of standard buttons. Document links are
transparent hit targets over actual PDF geometry, with visible keyboard focus.
External links require an explicit confirmation that displays the address.

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
that is visible against white paper. Buttons use 140ms hover transitions,
but palette changes switch text and surfaces atomically to prevent transient
contrast failures. Dialog entry takes 180ms with the recorded ease-out curve.
Reduced-motion preferences disable ongoing animation and smooth scrolling.

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
