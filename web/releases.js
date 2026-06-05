// Detect the visitor's OS and point the download buttons at the matching
// installer from the latest GitHub release. Falls back to /releases/latest.
// Labels are localized via window.OE (see i18n.js) and re-applied on language change.
(function () {
  var REPO = "mohaelder/openenlarge";
  var LATEST = "https://github.com/" + REPO + "/releases/latest";

  function detectOS() {
    var ua = (navigator.userAgent || "") + " " + (navigator.platform || "");
    if (/Android/i.test(ua)) return null; // Android UA contains "Linux"; no desktop build for it
    if (/Mac|iPhone|iPad/i.test(ua)) return "macos";
    if (/Win/i.test(ua)) return "windows";
    if (/Linux|X11/i.test(ua)) return "linux";
    return null;
  }

  function t(key, fallback) {
    return (window.OE && window.OE.t) ? window.OE.t(key) : fallback;
  }

  // Pick the best asset for an OS from a release's asset list.
  function pickAsset(assets, os) {
    var isArm = /arm|aarch64/i.test(navigator.userAgent + navigator.platform);
    var rank = {
      macos: function (n) {
        if (!/\.dmg$/i.test(n)) return -1;
        var arm = /aarch64|arm64/i.test(n);
        return isArm === arm ? 2 : 1;
      },
      windows: function (n) { return /\.msi$/i.test(n) ? 2 : /\.exe$/i.test(n) ? 1 : -1; },
      linux: function (n) { return /\.AppImage$/i.test(n) ? 2 : /\.deb$/i.test(n) ? 1 : -1; }
    }[os];
    if (!rank) return null;
    var best = null, bestScore = 0;
    assets.forEach(function (a) {
      var s = rank(a.name);
      if (s > bestScore) { bestScore = s; best = a; }
    });
    return best;
  }

  var os = detectOS();
  var heroBtn = document.getElementById("hero-download");
  var dlBtn = document.getElementById("dl-download");
  var navBtn = document.getElementById("nav-download");
  var meta = document.getElementById("release-meta");
  var lastTag = null;

  // Set the OS-specific download label (localized) on the primary buttons.
  function applyOsLabels() {
    if (!os) return; // unknown OS: leave the localized base label from data-i18n
    var label = "↓ " + t("dl.os." + os, "Download");
    if (heroBtn) heroBtn.textContent = label;
    if (dlBtn) dlBtn.textContent = label;
  }

  // Append the release tag to the localized meta line, e.g. "... · v0.1.0".
  function applyMeta() {
    if (meta && lastTag) meta.textContent = t("hero.metaLine", meta.textContent) + " · " + lastTag;
  }

  applyOsLabels();

  // Re-localize when the user switches language (i18n.js reset textContent first).
  window.addEventListener("oe-locale", function () {
    applyOsLabels();
    applyMeta();
  });

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", {
    headers: { Accept: "application/vnd.github+json" }
  })
    .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(function (rel) {
      var assets = rel.assets || [];
      var asset = os ? pickAsset(assets, os) : null;
      var url = asset ? asset.browser_download_url : LATEST;
      [heroBtn, dlBtn, navBtn].forEach(function (b) { if (b) b.href = url; });

      if (rel.tag_name) { lastTag = rel.tag_name; applyMeta(); }

      // Wire the per-OS quick links to their best asset, if present.
      var row = document.getElementById("os-row");
      if (row) {
        ["macos", "windows", "linux"].forEach(function (o) {
          var a = pickAsset(assets, o);
          var link = row.querySelector('[data-os="' + o + '"]');
          if (a && link) link.href = a.browser_download_url;
        });
      }
    })
    .catch(function () { /* no release yet / offline: links already point at /releases/latest */ });
})();
