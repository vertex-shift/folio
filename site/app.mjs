export const pageUrl = "https://vertex-shift.github.io/folio/";
export const downloads = Object.freeze({
  macos: {
    href: "https://github.com/vertex-shift/folio/releases/download/v0.1.1/Folio_0.1.1_macos_arm64.dmg",
    label: "Download for macOS",
    detail: "Apple Silicon · v0.1.1 · ad-hoc signed",
  },
  windows: {
    href: "https://github.com/vertex-shift/folio/releases/download/v0.1.1/Folio_0.1.1_windows_x64_setup.exe",
    label: "Download for Windows",
    detail: "x64 · v0.1.1 · unsigned installer",
  },
  linux: {
    href: "https://github.com/vertex-shift/folio/releases/download/v0.1.1/Folio_0.1.1_linux_x64.deb",
    label: "Download for Linux",
    detail: "x64 · v0.1.1 · .deb package",
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
  function selectPreview(tab, focus = false) {
    const view = tab.dataset.preview, preview = previews[view];
    if (!preview) throw new Error("This workspace preview is not supported.");
    for (const button of tabs) {
      const active = button === tab;
      button.setAttribute("aria-selected", String(active));
      button.tabIndex = active ? 0 : -1;
    }
    element("workspace-preview").setAttribute("aria-labelledby", tab.id);
    element("preview-image").src = `./assets/workspace-${view}.webp`;
    element("preview-image").alt = preview.alt;
    element("preview-detail").srcset = `./assets/workspace-${view}-detail.webp`;
    element("preview-caption").textContent = preview.caption;
    if (focus) tab.focus();
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
  element("preview-image").addEventListener("error", () => {
    element("preview-caption").textContent = "This screenshot could not load. Download the sample PDF or visit the source to explore Folio.";
  });

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
