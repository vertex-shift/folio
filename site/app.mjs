export const pageUrl = "https://vertex-shift.github.io/folio/";
export const downloads = Object.freeze({
  macos: {
    href: "https://github.com/vertex-shift/folio/releases/download/v0.1.2/Folio_0.1.2_macos_arm64.dmg",
    label: "Download for macOS",
    detail: "Apple Silicon · v0.1.2 · ad-hoc signed",
  },
  windows: {
    href: "https://github.com/vertex-shift/folio/releases/download/v0.1.2/Folio_0.1.2_windows_x64_setup.exe",
    label: "Download for Windows",
    detail: "x64 · v0.1.2 · unsigned installer",
  },
  linux: {
    href: "https://github.com/vertex-shift/folio/releases/download/v0.1.2/Folio_0.1.2_linux_x64.deb",
    label: "Download for Linux",
    detail: "x64 · v0.1.2 · .deb package",
  },
});

export function platformDownload(platform) {
  if (/win/i.test(platform)) return downloads.windows;
  if (/mac/i.test(platform)) return downloads.macos;
  if (/linux/i.test(platform) && !/android|arm|aarch64/i.test(platform)) return downloads.linux;
  return null;
}

const previews = {
  read: {
    alt: "Actual Folio desktop workspace reading an original sample PDF, with a real highlight and note.",
    caption: "A real PDF. A real workspace. Original sample content.",
  },
  comment: {
    alt: "Actual Folio comments view showing a portable highlight and a note added to the original sample PDF.",
    caption: "Highlights and notes travel with the PDF, not a cloud account.",
  },
  organize: {
    alt: "Actual Folio page-organization view showing the four original sample pages and working page controls.",
    caption: "Every page in its place. Reorder, rotate, duplicate and extract.",
  },
};

if (typeof document !== "undefined") {
  const element = (id) => {
    const result = document.getElementById(id);
    if (!result) throw new Error(`The Folio introduction is missing its ${id} control.`);
    return result;
  };
  const motionPreference = window.matchMedia("(prefers-reduced-motion: reduce)");
  const animations = new Set();
  let motionRevision = 0;
  const sheetFrames = [
    { transform: "perspective(1200px) rotateX(9deg) scale(.96)",
      clipPath: "polygon(0 0, 86% 0, 100% 14%, 100% 100%, 0 100%)" },
    { transform: "perspective(1200px) rotateX(0deg) scale(1)",
      clipPath: "polygon(0 0, 100% 0, 100% 0, 100% 100%, 0 100%)" },
  ];
  function stopMotion() {
    motionRevision++;
    for (const animation of animations) animation.cancel();
  }
  function playMotion(target, frames, duration, name) {
    const bounds = target.getBoundingClientRect();
    if (document.hidden || window.matchMedia("print").matches || bounds.bottom <= 0 || bounds.top >= innerHeight
      || typeof target.animate !== "function") return;
    const reduced = motionPreference.matches;
    const animation = target.animate(reduced ? [{ opacity: .92 }, { opacity: 1 }] : frames,
      { duration: reduced ? 120 : duration, easing: "cubic-bezier(.16, 1, .3, 1)" });
    animation.id = `folio-${name}`;
    animations.add(animation);
    target.style.willChange = reduced ? "opacity"
      : frames[0].transform ? "transform, clip-path" : frames[0].clipPath ? "clip-path" : "opacity";
    const clean = () => {
      animations.delete(animation);
      if (![...animations].some((other) => other.effect?.target === target)) {
        target.style.removeProperty("will-change");
      }
    };
    animation.onfinish = clean;
    animation.oncancel = clean;
  }
  motionPreference.addEventListener("change", stopMotion);
  document.addEventListener("visibilitychange", () => { if (document.hidden) stopMotion(); });
  window.addEventListener("pagehide", stopMotion);
  window.addEventListener("beforeprint", stopMotion);

  const appearance = window.folioSiteAppearance;
  const appearanceStatus = element("appearance-status");
  if (appearance) {
    const select = element("website-appearance");
    const showAppearance = (state) => {
      select.value = state.preference;
      appearanceStatus.textContent = state.message ?? "";
      appearanceStatus.hidden = !state.message;
    };
    showAppearance(appearance.snapshot());
    element("appearance-control").hidden = false;
    select.addEventListener("change", () => {
      stopMotion();
      try { showAppearance(appearance.choose(select.value)); }
      catch (error) {
        appearanceStatus.textContent = error instanceof Error ? error.message : "The website appearance could not be changed. Reload the page.";
        appearanceStatus.hidden = false;
      }
    });
  } else {
    appearanceStatus.textContent = "Website appearance controls could not load. Reload the page to choose an override; System is still the default.";
    appearanceStatus.hidden = false;
  }
  const mobile = navigator.userAgentData?.mobile || /Android|iPhone|iPad|iPod/.test(navigator.userAgent);
  const detected = mobile ? null : platformDownload(navigator.userAgentData?.platform ?? navigator.platform);
  if (detected) {
    element("primary-download").href = detected.href;
    element("primary-download-label").textContent = detected.label;
    element("primary-download-detail").textContent = detected.detail;
  }

  const tabs = [...document.querySelectorAll("[data-preview]")];
  tabs[0].closest(".preview-tabs").hidden = false;
  const previewImage = element("preview-image");
  const previewFrame = element("workspace-preview");
  const previewCaption = element("preview-caption");
  let previewRevision = 0;
  let activePreview = "read";
  let pendingPreview = false;
  let entranceSeen = false;
  function previewReady() {
    if (!previewImage.complete || !previewImage.naturalWidth
      || ![`workspace-${activePreview}.webp`, `workspace-${activePreview}-detail.webp`]
        .some((file) => previewImage.currentSrc.endsWith(`/${file}`))) return;
    if (!entranceSeen) {
      entranceSeen = true;
      const revision = previewRevision;
      const motion = motionRevision;
      document.fonts.ready.then(() => {
        if (revision !== previewRevision || motion !== motionRevision
          || activePreview !== "read" || motionPreference.matches) return;
        playMotion(document.querySelector(".hero h1 span"),
          [{ clipPath: "inset(0 68% 0 0)" }, { clipPath: "inset(0 0 0 0)" }],
          560, "ownership-reveal");
        playMotion(previewFrame, sheetFrames, 740, "document-open");
      });
    }
    if (pendingPreview) {
      pendingPreview = false;
      playMotion(previewFrame, sheetFrames, 420, "preview-sheet");
      playMotion(previewCaption, [{ opacity: .92 }, { opacity: 1 }], 240, "preview-caption");
    }
  }
  function selectPreview(tab, focus = false) {
    const view = tab.dataset.preview, preview = previews[view];
    if (!preview) throw new Error("This workspace preview is not supported.");
    stopMotion();
    previewRevision++;
    pendingPreview = view !== activePreview;
    activePreview = view;
    for (const button of tabs) {
      const active = button === tab;
      button.setAttribute("aria-selected", String(active));
      button.tabIndex = active ? 0 : -1;
    }
    previewFrame.setAttribute("aria-labelledby", tab.id);
    previewImage.src = `./assets/workspace-${view}.webp`;
    previewImage.alt = preview.alt;
    element("preview-detail").srcset = `./assets/workspace-${view}-detail.webp`;
    previewCaption.textContent = preview.caption;
    if (focus) tab.focus();
    previewReady();
  }
  for (const [index, tab] of tabs.entries()) {
    tab.addEventListener("click", () => selectPreview(tab));
    tab.addEventListener("keydown", (event) => {
      const target = { ArrowRight: (index + 1) % tabs.length,
        ArrowLeft: (index + tabs.length - 1) % tabs.length, Home: 0, End: tabs.length - 1 }[event.key];
      if (target === undefined) return;
      event.preventDefault();
      selectPreview(tabs[target], true);
    });
  }
  previewImage.addEventListener("load", previewReady);
  previewImage.addEventListener("error", () => {
    stopMotion();
    pendingPreview = false;
    previewCaption.textContent = "This screenshot could not load. Download the sample PDF or visit the source to explore Folio.";
  });
  previewReady();

  const share = element("share-button"), status = element("share-status");
  share.hidden = false;
  share.addEventListener("click", async () => {
    share.disabled = true;
    element("share-fallback").hidden = true;
    status.textContent = "Preparing to share...";
    try {
      if (typeof navigator.share === "function") {
        await navigator.share({ title: "Folio — Your PDFs. Your rules.",
          text: "An independent, open-source alternative to Adobe Acrobat. A little less paperwork. No account or subscription.",
          url: pageUrl });
        status.textContent = "Thanks for passing Folio on.";
      } else {
        if (!navigator.clipboard?.writeText) throw new Error("Clipboard access is unavailable.");
        await navigator.clipboard.writeText(pageUrl);
        status.textContent = "Link copied. Thanks for spreading the word.";
      }
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") status.textContent = "Share canceled.";
      else {
        status.textContent = "The page could not be shared automatically. Copy its address below.";
        element("share-fallback").hidden = false;
        element("share-address").focus();
        element("share-address").select();
      }
    } finally { share.disabled = false; }
  });
}
