#!/usr/bin/env python3
"""Regenerate app/src/lib/i18n/dict.ts from i18n-strings.csv.

The CSV is the source of truth for UI strings. Its header is
`key,<locale>,<locale>,...,file,note`; every column that is not `key`, `file`,
or `note` is treated as a locale. Run from the repo root:  python3 scripts/gen-i18n.py
"""
import csv, json, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
CSV = ROOT / "i18n-strings.csv"
OUT = ROOT / "app/src/lib/i18n/dict.ts"
META_COLUMNS = {"key", "file", "note"}


def locale_columns(header):
    return [c for c in header if c not in META_COLUMNS]


def emit(d):
    return "\n".join(
        f"    {json.dumps(k, ensure_ascii=False)}: {json.dumps(v, ensure_ascii=False)},"
        for k, v in d.items()
    )


def main():
    reader = csv.DictReader(CSV.open(newline=""))
    rows = list(reader)
    locales = locale_columns(reader.fieldnames)
    dicts = {lc: {r["key"]: r[lc] for r in rows} for lc in locales}
    body = "".join(
        f"  {json.dumps(lc, ensure_ascii=False)}: {{\n{emit(dicts[lc])}\n  }},\n"
        for lc in locales
    )
    OUT.write_text(
        "// AUTO-GENERATED from /i18n-strings.csv — do not edit by hand.\n"
        "// To change strings, edit the CSV and regenerate (see scripts/gen-i18n.py).\n"
        "export const dict: Record<string, Record<string, string>> = {\n"
        f"{body}"
        "};\n"
    )
    print(f"wrote {OUT.relative_to(ROOT)} with {len(rows)} keys × {len(locales)} locales")


if __name__ == "__main__":
    main()
