"""Reproduce the committed OCR-only static TrueType fonts from pinned sources."""

import argparse
import hashlib
from pathlib import Path

import fontTools
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont


SOURCES = [
    ("NotoSansArabic", "FolioOcrArabic", "63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e"),
    ("NotoSansDevanagari", "FolioOcrDevanagari", "14ec4af41f27482216d1c2229f417ff9b1425e1babb014e57d1d40d03229853e"),
    ("NotoSansSC", "FolioOcrCjkSC", "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da"),
    ("NotoSansJP", "FolioOcrCjkJP", "c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f"),
    ("NotoSansKR", "FolioOcrCjkKR", "194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True, help="Directory containing the five pinned upstream variable fonts")
    args = parser.parse_args()
    if fontTools.__version__ != "4.62.1":
        raise RuntimeError("Reproducible OCR font instancing requires fonttools 4.62.1.")
    destination = Path(__file__).resolve().parent.parent / "src-tauri/resources/ocr/fonts"
    destination.mkdir(parents=True, exist_ok=True)
    for original, family, checksum in SOURCES:
        path = args.source / f"{original}.ttf"
        if hashlib.sha256(path.read_bytes()).hexdigest() != checksum:
            raise RuntimeError(f"{original}: upstream SHA-256 verification failed")
        font = TTFont(path, recalcTimestamp=False)
        axes = {axis.axisTag: 400 if axis.axisTag == "wght" else axis.defaultValue for axis in font["fvar"].axes}
        font = instantiateVariableFont(font, axes, inplace=True)
        for record in list(font["name"].names):
            replacements = {
                1: family, 2: "Regular", 3: f"{family}-Regular-1",
                4: f"{family} Regular", 6: f"{family}-Regular",
                16: family, 17: "Regular",
            }
            if record.nameID in replacements:
                font["name"].setName(replacements[record.nameID], record.nameID,
                                     record.platformID, record.platEncID, record.langID)
        if any(table in font for table in ["fvar", "gvar", "CFF ", "CFF2"]) or "glyf" not in font:
            raise RuntimeError(f"{original}: a static TrueType glyf instance is required")
        output = destination / f"{family}-Regular.ttf"
        font.save(output, reorderTables=True)
        print(output.name, output.stat().st_size, hashlib.sha256(output.read_bytes()).hexdigest())


if __name__ == "__main__":
    main()
