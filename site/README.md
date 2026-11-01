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

## Appearance

The site defaults to **System**, following live `prefers-color-scheme` changes.
The header offers explicit Light and Dark overrides. Only the
`folio.site.appearance` browser preference is stored, separately from the desktop
app's `folio.appearance`; it is never sent to a service. A small blocking
bootstrap restores the choice before CSS loads, avoiding a wrong-theme flash.

Without JavaScript, CSS still follows the system preference and the enhancement
control stays hidden. Blocked or invalid browser storage produces a visible
notice; choices still apply for the current visit. Theme colors change atomically,
while original PDF/workspace imagery keeps its actual colors.

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

## Search discovery

The title and description identify a free, open-source desktop PDF reader/editor
and an independent Acrobat alternative. Search and social previews use consistent
copy; the original 1200x630 sharing image has explicit dimensions, type and
alternative text. `max-image-preview:large` permits larger search previews
without changing the actual image or promising a particular result.

JSON-LD connects the canonical `WebPage`, published `SoftwareApplication`,
`SoftwareSourceCode` and original `ImageObject` with stable IDs. Keep the app
version and download offer tied to the actual release. `codeRepository` belongs
to the source entity; newer source-only features must not be attributed to
the old installer. There are no invented ratings, reviews or endorsements.
Google's software-app rich results require a genuine rating or review, so this
markup does not claim eligibility for a rich result.

The XML sitemap lists only the canonical page and its three real workspace
images. Update `lastmod` when the public page, its structured data or imagery
meaningfully changes, not on every build. Do not add guessed URLs, duplicate
`index.html` entries, artificial priorities or deprecated sitemap ping calls.

For Google indexing, the site owner can verify the URL-prefix property
`https://vertex-shift.github.io/folio/` in Search Console, submit
`https://vertex-shift.github.io/folio/sitemap.xml`, and inspect the canonical
homepage. Verification requires the owner's real token or verification file;
do not invent one or add a placeholder. This repository does not configure or
claim Search Console submission.

GitHub project Pages deploys at `/folio/`, not the host root. Crawlers read
`https://vertex-shift.github.io/robots.txt`; a `site/robots.txt` deployed at
`/folio/robots.txt` would be ignored. The host-root robots URL currently returns
404, which does not block crawling, and the page allows indexing. If a separate
host-root Pages site or custom domain is introduced later, configure and verify
robots there. Preserve the existing redirect to the trailing-slash canonical.

References: [Google's sitemap guidance](https://developers.google.com/search/docs/crawling-indexing/sitemaps/build-sitemap),
[robots.txt placement](https://developers.google.com/crawling/docs/robots-txt/create-robots-txt)
and [software-app structured data](https://developers.google.com/search/docs/appearance/structured-data/software-app).

## Publish through a PR

`.github/workflows/pages.yml` checks the static files on pull requests and
uploads **only `site/`** on `main`. The deployment job uses scoped Pages and
OIDC permissions and the `github-pages` environment. It never deploys a PR
or publishes an application release.

Merge changes through a pull request with all required platform CI and resolved
conversations; do not push directly to protected `main`. After the Pages job
finishes, verify the public HTML, local assets, downloads and social card.
