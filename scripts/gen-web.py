#!/usr/bin/env python3
"""Generate per-locale static landing + blog pages from the English templates.

web/index.html and web/blog.html are the English (canonical) source. This script
bakes translated text (from web/landing-strings.json) into per-locale copies under
web/<locale>/, sets <html lang>, localized <title>/<meta>, canonical + hreflang, and
rewrites internal links to stay within the active locale. English is rewritten in
place at the root. Run from repo root:  python3 scripts/gen-web.py
Never hand-edit web/<locale>/ output.
"""
import json, pathlib, re, html

ROOT = pathlib.Path(__file__).resolve().parent.parent
WEB = ROOT / "web"
SITE = "https://openenlarge.io"
LOCALES = ["en", "zh", "zh-hant", "ja", "ko"]
HREFLANG = {"en": "en", "zh": "zh-Hans", "zh-hant": "zh-Hant", "ja": "ja", "ko": "ko"}
LABELS = {"en": "English", "zh": "简体中文", "zh-hant": "繁體中文", "ja": "日本語", "ko": "한국어"}
# Locale whose /docs/ translation a landing page links to. Defaults to the page's own
# locale; zh-hant falls back to Simplified docs until Traditional docs are translated.
DOCS_LOCALE = {"zh-hant": "zh"}
# Globe icon for the language switcher (kept verbatim in sync with scripts/gen-docs.py).
# Inline SVG renders on every platform; flag emoji do not (Windows shows letters), and a
# globe avoids the "which flag for English?" problem.
GLOBE_SVG = (
    '<svg class="globe" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" '
    'fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">'
    '<circle cx="12" cy="12" r="9"/><path d="M3 12h18"/>'
    '<path d="M12 3c2.7 2.6 2.7 15.4 0 18c-2.7-2.6-2.7-15.4 0-18z"/></svg>'
)
# page -> (template filename, title key, desc key, site path stem)
PAGES = {
    "index": ("index.html", "meta.title", "meta.desc", ""),
    "blog":  ("blog.html", "blog.metaTitle", "blog.metaDesc", "blog.html"),
}

def langswitch_url(page, locale):
    """Root-absolute URL to the same page in another locale (en at root, else /<locale>/)."""
    stem = PAGES[page][3]
    return ("/" if locale == "en" else f"/{locale}/") + stem

def langswitch_html(page, locale):
    """Globe-triggered language dropdown, baked per page+locale (current marked, real links)."""
    items = []
    for lc in LOCALES:
        cur = ' aria-current="true"' if lc == locale else ""
        items.append(f'<a{cur} href="{langswitch_url(page, lc)}">{LABELS[lc]}</a>')
    return ('<details class="langmenu"><summary aria-label="Language">' + GLOBE_SVG + '</summary>'
            '<div class="langmenu-list">' + "".join(items) + '</div></details>')

def strings():
    return json.loads((WEB / "landing-strings.json").read_text())

def url_for(stem, locale):
    base = f"{SITE}/" if locale == "en" else f"{SITE}/{locale}/"
    return base + stem  # stem "" -> directory URL; "blog.html" -> file

def out_path(page, locale):
    fname = PAGES[page][0]
    return WEB / (fname if locale == "en" else f"{locale}/{fname}")

def seo_head(page, locale):
    stem = PAGES[page][3]
    canon = url_for(stem, locale)
    alts = [f'<link rel="canonical" href="{canon}">']
    for lc in LOCALES:
        alts.append(f'<link rel="alternate" hreflang="{HREFLANG[lc]}" href="{url_for(stem, lc)}">')
    alts.append(f'<link rel="alternate" hreflang="x-default" href="{url_for(stem, "en")}">')
    return "\n".join(alts)

def localize_links(doc, locale):
    """Rewrite same-site internal links so navigation stays within the locale.
    - Docs link: /docs/index.html -> /docs/<locale>/index.html (en unchanged)
    - Blog/home cross-links within the landing set get the locale prefix.
    Anchor (#...), external (http), and asset (img/, *.js, *.css, releases*.json) links are left alone."""
    if locale == "en":
        return doc
    # docs root link. zh-hant docs aren't translated yet, so Traditional readers
    # are routed to the Simplified docs (readable Chinese) rather than a 404.
    dloc = DOCS_LOCALE.get(locale, locale)
    doc = doc.replace('href="/docs/index.html"', f'href="/docs/{dloc}/index.html"')
    doc = doc.replace('href="/docs/"', f'href="/docs/{dloc}/"')
    # landing cross-links (root-absolute forms used in the nav)
    doc = doc.replace('href="/blog"', f'href="/{locale}/blog.html"')
    doc = doc.replace('href="/index.html"', f'href="/{locale}/index.html"')
    doc = doc.replace('href="/"', f'href="/{locale}/"')
    return doc

def fix_asset_paths(doc, locale):
    """Locale pages live one directory deeper; make root-relative asset refs absolute.
    The templates use relative refs like src="img/..", "i18n.js", "releases.js",
    href="img/..", and fetch('./releases.json') in releases.js (handled there).
    Convert leading relative asset refs to root-absolute so they resolve from /<locale>/."""
    if locale == "en":
        return doc
    doc = re.sub(r'(src|href)="(?!https?:|/|#|mailto:)([^"]+)"', r'\1="/\2"', doc)
    return doc

def apply_strings(doc, st, locale, page):
    s = st[locale]
    base_en = st["en"]
    def tr(key):
        return s.get(key) or base_en.get(key) or key
    # <html lang>
    doc = re.sub(r'<html lang="[^"]*"', f'<html lang="{HREFLANG[locale]}"', doc, count=1)
    # title + description (page-specific keys)
    _, tkey, dkey, _ = PAGES[page]
    doc = re.sub(r"(<title[^>]*>).*?(</title>)", lambda m: m.group(1) + html.escape(tr(tkey), quote=False) + m.group(2), doc, count=1, flags=re.S)
    doc = re.sub(r'(<meta name="description" content=")[^"]*(")',
                 lambda m: m.group(1) + html.escape(tr(dkey), quote=False) + m.group(2), doc, count=1)
    # data-i18n (textContent) — replace inner text
    doc = re.sub(r'(<(\w+)[^>]*\bdata-i18n="([^"]+)"[^>]*)>.*?</\2>',
                 lambda m: m.group(1) + ">" + html.escape(tr(m.group(3)), quote=False) + "</" + m.group(2) + ">",
                 doc, flags=re.S)
    # data-i18n-html (innerHTML) — replace inner markup, do NOT escape
    doc = re.sub(r'(<(\w+)[^>]*\bdata-i18n-html="([^"]+)"[^>]*)>.*?</\2>',
                 lambda m: m.group(1) + ">" + tr(m.group(3)) + "</" + m.group(2) + ">",
                 doc, flags=re.S)
    return doc

def render(page, locale, st):
    template = (WEB / PAGES[page][0]).read_text()
    doc = template
    # Normalize: collapse previously generated blocks back to their markers so re-running
    # build() on the in-place English root file stays idempotent.
    doc = re.sub(r'<link rel="canonical"[^>]*>(?:\s*<link rel="alternate"[^>]*>)+',
                 "<!--OE-SEO-->", doc, count=1)
    doc = re.sub(r'<details class="langmenu">.*?</details>',
                 "<!--OE-LANGSWITCH-->", doc, count=1, flags=re.S)
    doc = apply_strings(doc, st, locale, page)
    doc = doc.replace("<!--OE-SEO-->", seo_head(page, locale))
    doc = localize_links(doc, locale)
    doc = fix_asset_paths(doc, locale)
    # Expand the language switcher LAST: its links are already locale-correct
    # (root-absolute /, /zh/, …), so they must bypass localize_links / fix_asset_paths,
    # which would otherwise rewrite the English "/" link to "/<locale>/".
    doc = doc.replace("<!--OE-LANGSWITCH-->", langswitch_html(page, locale))
    return doc

def write_sitemap():
    urls = []
    for page in PAGES:
        stem = PAGES[page][3]
        for lc in LOCALES:
            loc = url_for(stem, lc)
            links = "".join(f'<xhtml:link rel="alternate" hreflang="{HREFLANG[l]}" href="{url_for(stem, l)}"/>' for l in LOCALES)
            urls.append(f"<url><loc>{loc}</loc>{links}</url>")
    xml = ('<?xml version="1.0" encoding="UTF-8"?>\n'
           '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" '
           'xmlns:xhtml="http://www.w3.org/1999/xhtml">\n' + "\n".join(urls) + "\n</urlset>\n")
    (WEB / "sitemap.xml").write_text(xml)

def build():
    st = strings()
    # English first: the templates ARE the en source; render(en) re-bakes en text
    # (idempotent) and injects the SEO marker, then writes back to root.
    for page in PAGES:
        for lc in LOCALES:
            p = out_path(page, lc)
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(render(page, lc, st))
    write_sitemap()
    print(f"wrote {len(PAGES)*len(LOCALES)} landing pages + sitemap")

if __name__ == "__main__":
    build()
