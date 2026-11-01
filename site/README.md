# Folio introduction

The public site is <https://vertex-shift.github.io/folio/>.
It introduces Folio as an independent open-source alternative for supported
Acrobat-style workflows, not a complete replacement or a hosted PDF editor.

This is deliberately a static site: no application bundle, OCR models,
backend, trackers, signup form, remote fonts or user-document uploads.
The self-hosted Commissioner font carries its SIL Open Font License in
`assets/fonts/OFL.txt`. Original Folio images, SVGs and fictional sample content
use the repository's Apache-2.0 license; `assets/provenance.json` records origins,
crop geometry and hashes. Raster sidecars retain their provenance too.

## Preview and check

From the repository root:

```sh
python3 -m http.server 1445 --bind 127.0.0.1 --directory site
```

Open <http://127.0.0.1:1445/>. Relative asset paths also work under GitHub's
`/folio/` project prefix, which the browser checks exercise.

```sh
node --test tests/landingPage.test.ts
npm run test:site
```

The browser command uses Playwright Chromium; set `FOLIO_SITE_BROWSER` to an
existing compatible Chromium executable, or install Chromium with
`npx playwright install chromium` if it is missing. It starts only an owned
loopback static server and closes it afterward. Review screenshots and evidence
go to the gitignored `.impeccable/review/site/` directory.

## Keep the release boundary honest

The download links intentionally name **v0.1.1**. Those installers contain
the earlier reader/editor, protected-file and multi-tab workflows. Newer
paragraph reflow, accessibility, multilingual OCR and offline trust work is
on `main`, not in those binaries. Source-build screenshots are explicitly
labeled. Do not advertise a new feature as downloadable before releasing it.

When a release is published, update the installer links, version descriptions
and structured data in `index.html` and `app.mjs`, and the matching assertions
in `tests/landingPage.test.ts`. Preserve platform architecture and unsigned /
ad-hoc-signing notices. Link only to actual published release assets.

## Original sample and sharing artwork

Regenerate the fictional four-page PDF:

```sh
node scripts/create-site-demo.mjs
```

Workspace images must be real captures of Folio working with this shareable
sample, never invented UI or private documents. Capture all desktop states at
the same viewport, make uniformly scaled detail crops for small screens, and
update origins, crop coordinates, hashes and raster provenance when replacing
them. The mobile note view intentionally shows the actual comments pane.

Recreate the original 1200x630 sharing card after changing its inputs:

```sh
node scripts/create-site-social.mjs
```

This uses the bundled font and real workspace screenshot. Open Graph / Twitter
cards, a canonical URL, sitemap and an explicit share action support discovery;
none is a guarantee of virality, indexing, ratings or adoption.

## Publish through a PR

`.github/workflows/pages.yml` checks the static files on pull requests and
uploads **only `site/`** on `main`. The deployment job uses scoped Pages and
OIDC permissions and the `github-pages` environment. It never deploys a PR
or publishes an application release.

Merge changes through a pull request with all required platform CI and resolved
conversations; do not push directly to protected `main`. After the Pages job
finishes, verify the public HTML, local assets, downloads and social card.
