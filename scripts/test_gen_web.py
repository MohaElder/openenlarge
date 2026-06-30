import importlib.util, json, pathlib, unittest
ROOT = pathlib.Path(__file__).resolve().parent.parent

class TestStringsSchema(unittest.TestCase):
    def test_all_locales_same_keys(self):
        data = json.loads((ROOT / "web/landing-strings.json").read_text())
        self.assertEqual(set(data.keys()), {"en", "zh", "zh-hant", "ja", "ko"})
        base = set(data["en"].keys())
        for lc in ("zh", "zh-hant", "ja", "ko"):
            self.assertEqual(set(data[lc].keys()), base, f"{lc} key set differs from en")

spec = importlib.util.spec_from_file_location("gen_web", ROOT / "scripts" / "gen-web.py")

class TestGenWeb(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        gw = importlib.util.module_from_spec(spec); spec.loader.exec_module(gw)
        cls.gw = gw; gw.build()
    def test_locale_pages_exist(self):
        for lc in ("zh", "zh-hant", "ja", "ko"):
            self.assertTrue((ROOT / f"web/{lc}/index.html").exists(), f"{lc} index missing")
            self.assertTrue((ROOT / f"web/{lc}/blog.html").exists(), f"{lc} blog missing")
    def test_html_lang_set(self):
        self.assertIn('<html lang="ja"', (ROOT / "web/ja/index.html").read_text())
        self.assertIn('lang="zh-Hans"', (ROOT / "web/zh/index.html").read_text())
        self.assertIn('lang="zh-Hant"', (ROOT / "web/zh-hant/index.html").read_text())
    def test_zh_hant_docs_link_falls_back_to_simplified(self):
        # zh-hant docs aren't translated; the Docs nav routes to /docs/zh/ (not a 404).
        html = (ROOT / "web/zh-hant/index.html").read_text()
        self.assertIn('/docs/zh/index.html', html)
        self.assertNotIn('/docs/zh-hant/', html)
    def test_hreflang_reciprocal(self):
        html = (ROOT / "web/index.html").read_text()
        self.assertIn('hreflang="ja"', html)
        self.assertIn('hreflang="ko"', html)
        self.assertIn('hreflang="x-default"', html)
    def test_canonical_self(self):
        self.assertIn('rel="canonical" href="https://openenlarge.io/ja/"', (ROOT / "web/ja/index.html").read_text())
    def test_text_baked_in(self):
        # zh hero text appears in the served HTML (crawlable, not JS-only)
        self.assertIn("开源", (ROOT / "web/zh/index.html").read_text())
    def test_internal_links_localized(self):
        # in /zh/, the Docs nav link points at the zh docs
        self.assertIn('/docs/zh/index.html', (ROOT / "web/zh/index.html").read_text())
    def test_sitemap_lists_locales(self):
        sm = (ROOT / "web/sitemap.xml").read_text()
        for u in ("https://openenlarge.io/", "https://openenlarge.io/zh/",
                  "https://openenlarge.io/zh-hant/",
                  "https://openenlarge.io/ja/", "https://openenlarge.io/ko/"):
            self.assertIn(u, sm)

if __name__ == "__main__":
    unittest.main()
