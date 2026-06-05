#!/usr/bin/env python3
"""Regenerate app/src/lib/i18n/dict.ts from i18n-strings.csv.

The CSV (key, en, zh, file, note) is the source of truth for UI strings.
Run from the repo root:  python3 scripts/gen-i18n.py
"""
import csv, json, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
CSV = ROOT / "i18n-strings.csv"
OUT = ROOT / "app/src/lib/i18n/dict.ts"


def emit(d):
    return "\n".join(
        f"    {json.dumps(k, ensure_ascii=False)}: {json.dumps(v, ensure_ascii=False)},"
        for k, v in d.items()
    )


def main():
    rows = list(csv.DictReader(CSV.open(newline="")))
    en = {r["key"]: r["en"] for r in rows}
    zh = {r["key"]: r["zh"] for r in rows}
    OUT.write_text(
        "// AUTO-GENERATED from /i18n-strings.csv — do not edit by hand.\n"
        "// To change strings, edit the CSV and regenerate (see scripts/gen-i18n.py).\n"
        "export const dict: Record<string, Record<string, string>> = {\n"
        f"  en: {{\n{emit(en)}\n  }},\n"
        f"  zh: {{\n{emit(zh)}\n  }},\n"
        "};\n"
    )
    print(f"wrote {OUT.relative_to(ROOT)} with {len(en)} keys")


if __name__ == "__main__":
    main()
