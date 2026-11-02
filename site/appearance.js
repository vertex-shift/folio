(() => {
  const key = "folio.site.appearance";
  const choices = ["system", "light", "dark", "sepia", "forest", "midnight"];
  let preference = "system";
  let message = null;

  try {
    const saved = localStorage.getItem(key);
    if (saved !== null && choices.includes(saved)) preference = saved;
    else if (saved !== null) message = "The saved website appearance was not recognized. Using System; choose an appearance to replace it.";
  } catch {
    message = "Browser storage is unavailable. Appearance changes will last only for this visit.";
  }
  document.documentElement.dataset.appearance = preference;

  window.folioSiteAppearance = Object.freeze({
    snapshot() { return { preference, message }; },
    choose(value) {
      if (!choices.includes(value)) throw new Error("Choose a listed website appearance.");
      preference = value;
      document.documentElement.dataset.appearance = value;
      try {
        localStorage.setItem(key, value);
        message = null;
      } catch {
        message = "Appearance changed for this visit, but could not be saved. Browser storage is unavailable.";
      }
      return { preference, message };
    },
  });
})();
