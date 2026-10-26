# Contributing to Folio

Contributions to Folio's PDF reader, editing tools, accessibility, documentation,
and platform compatibility are welcome. Folio is an independent, local-first
application, not a complete Acrobat replacement. Read the
[supported workflows and boundaries](README.md#what-works) before proposing a
feature, and discuss substantial changes in an issue before implementing them.

## Report a bug or propose a feature

Search [existing issues](https://github.com/vertex-shift/folio/issues) first.
For a bug, include the Folio version or commit, operating system and CPU
architecture, reproduction steps, expected behavior, and actual behavior.
Include relevant error messages and whether the problem occurs in the native
app or browser preview. For a feature, explain the document workflow it would
improve and any compatibility or accessibility requirements.

Never post credentials, signing keys, private PDFs, or personal information in
issues, pull requests, screenshots, or logs. Use a minimal, synthetic PDF that
you have permission to share, and inspect its metadata and attachments too.
Do not use a black rectangle or an ordinary annotation to conceal sensitive
content in a reproduction file. Do not report suspected security
vulnerabilities with sensitive details in a public issue.

## Set up development

Install Git, Node.js 24, and the current stable Rust toolchain through
[rustup](https://rustup.rs/). CI uses Node.js 24 and Rust stable.
Install the native requirements for your operating system before building:

| Platform | Native prerequisites |
| --- | --- |
| macOS | macOS 13+ and Xcode Command Line Tools |
| Windows | Microsoft C++ Build Tools with Desktop development with C++, and Microsoft Edge WebView2 |
| Linux | WebKitGTK 4.1, GTK and related development libraries, and a C/C++ build toolchain |

Follow the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for
your platform. The [build workflow](.github/workflows/build.yml) lists the exact
Ubuntu packages used in CI.

Fork the repository, then clone your fork. Replace `YOUR-USERNAME` below with
your GitHub username:

```sh
git clone https://github.com/YOUR-USERNAME/folio.git
cd folio
git switch -c describe-your-change
npm ci
npm run desktop
```

Install Rust before running `npm ci`: its prepare hook uses Cargo metadata to
collect dependency licenses. It also prepares bundled PDF.js assets and the
original quick-start PDF. No Acrobat installation or external PDF engine is
required.

For the [browser preview](README.md#browser-preview), run these commands in
separate terminals:

```sh
npm run preview:backend
```

```sh
npm run dev
```

Open <http://127.0.0.1:1420>. The preview uses the same Rust document engine,
but native dialogs, file associations, menus, and saving behavior must also
be checked in the desktop app. Keep both development servers on loopback;
do not expose the preview bridge to a network.

## Find the relevant code

| Location | Purpose |
| --- | --- |
| `src/App.tsx`, `src/DocumentWorkspace.tsx`, `src/components/` | Tab coordination, per-document React workspaces, and document tools |
| `src/lib/` | PDF.js integration, navigation, rendering geometry, themes, and native/preview bridge |
| `src-tauri/src/` | Rust document mutations, revision guards, persistence, and desktop integration |
| `tests/` | Frontend behavior tests using Node's test runner |
| `src-tauri/tests/` | Rust integration tests for PDF authoring, text editing, and redaction |
| `scripts/` | Reproducible example PDF, bundled assets, and dependency notices |

See the [architecture overview](README.md#architecture) for the document
pipeline. [PRODUCT.md](PRODUCT.md) records product constraints;
[DESIGN.md](DESIGN.md) records the existing visual system.

## Make and validate a change

Keep changes focused and follow the existing code patterns. Update directly
related documentation, and add a regression test for a bug fix or behavior
change. Use existing helpers rather than introducing another document engine,
parallel preview implementation, or unnecessary dependency.

Run the checks relevant to your change. Before submitting code that affects
the application, run:

```sh
npm test
npm run build
npm run test:rust -- --locked
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

If the Rust components are missing, install them with
`rustup component add rustfmt clippy`. The frontend build includes strict
TypeScript checking. Documentation-only changes do not require rebuilding
the application, but verify links and documented commands.

For PDF changes, test saved output by reopening it, preserve the original
document, and cover invalid input and unsupported cases without silently
discarding data. Keep revision checks, undo/redo, unsaved-change protection,
signed-document guards, and atomic saving intact. Redaction must remove the
underlying sensitive content, not just cover it visually; preserve the
documented image-only export boundary.
For protected PDFs, use synthetic passwords and verify that user and owner
passwords still work after save/reopen and exports. Do not strip encryption,
relax permissions, log passwords, or save the decrypted rendering copy.
For multi-document changes, verify inactive tabs retain edits, undo history,
reading/search state, and pending redactions. Target operations by document ID
and revision, and test both per-tab closing and quitting with unsaved inactive
documents. Opening another PDF must not discard the current one.

For interface changes, check keyboard access, focus, narrow layouts, all five
themes, and reduced motion. Keep PDF colors independent of workspace themes
and show explicit errors when an operation is unsupported or fails.

Use synthetic, redistributable test fixtures. Do not commit build output,
local configuration, `node_modules/`, `dist/`, or `src-tauri/target/`.
If dependencies or generated assets change, update the appropriate lockfile,
run `npm run prepare`, and review the generated PDF and third-party notices.
Do not add proprietary Adobe code, fonts, icons, or other extracted assets.

## Submit a pull request

Open a pull request from your fork's branch to `vertex-shift/folio`'s `main`
branch. Explain what changed and why, link any related issue, and list the
checks you ran. Include screenshots for visual changes using only shareable
example documents. Note platform limitations or behavior changes explicitly.

The [Desktop builds and releases workflow](.github/workflows/build.yml) runs
frontend and Rust document-engine tests and creates native bundles for Windows
x64, macOS Apple Silicon, and Linux x64. Fork pull requests may need a
maintainer's approval before workflows run. A successful build is not a
substitute for exercising the affected native workflow.

Only a pushed version tag publishes installers to GitHub Releases, after every
platform build succeeds. Pull requests and manual builds cannot publish
releases. See [Publishing a version](README.md#publishing-a-version) for version
synchronization, checksums, and release behavior.

Keep review follow-ups focused. Do not include credentials or signing material
to make a build pass; development builds do not require production signing
secrets. The local `npm run bundle` command builds a macOS `.app` only; CI
selects each platform's installer formats explicitly.

## Licensing

By submitting original work for inclusion, you agree to license it under
Folio's [Apache License, Version 2.0](LICENSE). Only contribute work you have
the right to submit. Third-party code, fonts, and other assets retain their
own licenses and must include the required attribution and redistribution
notices; do not relicense them as original Folio work.
