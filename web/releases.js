// Detect the visitor's OS and point the download buttons at the matching
// installer from the latest GitHub release. Falls back to /releases/latest.
(function () {
  var REPO = "mohaelder/openenlarge";
  var LATEST = "https://github.com/" + REPO + "/releases/latest";

  function detectOS() {
    var ua = (navigator.userAgent || "") + " " + (navigator.platform || "");
    if (/Mac|iPhone|iPad/i.test(ua)) return "macos";
    if (/Win/i.test(ua)) return "windows";
    if (/Linux|X11/i.test(ua)) return "linux";
    return null;
  }

  function label(os) {
    return os === "macos" ? "Download for macOS"
      : os === "windows" ? "Download for Windows"
      : os === "linux" ? "Download for Linux"
      : "Download";
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

  if (os && heroBtn) heroBtn.textContent = "↓ " + label(os);
  if (os && dlBtn) dlBtn.textContent = "↓ " + label(os);

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", {
    headers: { Accept: "application/vnd.github+json" }
  })
    .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(function (rel) {
      var assets = rel.assets || [];
      var asset = os ? pickAsset(assets, os) : null;
      var url = asset ? asset.browser_download_url : LATEST;
      [heroBtn, dlBtn, navBtn].forEach(function (b) { if (b) b.href = url; });

      if (meta && rel.tag_name) {
        meta.textContent = "Free · MIT licensed · macOS, Windows & Linux · " + rel.tag_name;
      }

      // Wire the per-OS quick links to their best asset, if present.
      var row = document.getElementById("os-row");
      if (row) {
        ["macos", "windows", "linux"].forEach(function (o, i) {
          var a = pickAsset(assets, o);
          if (a && row.children[i]) row.children[i].href = a.browser_download_url;
        });
      }
    })
    .catch(function () { /* no release yet / offline: links already point at /releases/latest */ });
})();
