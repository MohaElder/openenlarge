// Per-URL i18n for the landing/blog pages. Page text AND the language switcher (a globe
// dropdown of locale links) are baked into each locale's static HTML by scripts/gen-web.py,
// so this runtime no longer swaps page text or wires the switcher. It only:
//  - derives the active locale from the URL path, and
//  - exposes window.OE = { t, locale } so releases.js can localize OS-aware download labels.
window.OE = (function () {
  var LOCALES = ["en", "zh", "zh-hant", "ja", "ko"];

  function localeFromPath() {
    var seg = (location.pathname.split("/")[1] || "").toLowerCase();
    return LOCALES.indexOf(seg) > 0 ? seg : "en";
  }
  var locale = localeFromPath();

  // STRINGS are fetched once for OE.t (used by releases.js for OS-specific labels).
  var STRINGS = { en: {}, zh: {}, "zh-hant": {}, ja: {}, ko: {} };
  function t(key) {
    return (STRINGS[locale] && STRINGS[locale][key]) || STRINGS.en[key] || key;
  }

  // Resolve the path to landing-strings.json from any locale depth (root or /<locale>/).
  function stringsUrl() {
    return (locale === "en" ? "" : "../") + "landing-strings.json";
  }

  function init() {
    document.documentElement.lang =
      locale === "zh" ? "zh-Hans" : locale === "zh-hant" ? "zh-Hant" : locale;
    fetch(stringsUrl(), { cache: "no-cache" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (data) {
        if (data) STRINGS = data;
        // Let releases.js (and anything else) re-localize now that strings are loaded.
        window.dispatchEvent(new CustomEvent("oe-locale", { detail: locale }));
      })
      .catch(function () { /* offline / blocked: OE.t falls back to keys */ });
  }
  init();

  return { t: t, get locale() { return locale; } };
})();
