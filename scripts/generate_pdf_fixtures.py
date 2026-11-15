"""Self-authored PDF fixtures. Python 3.12 standard library only; not a PDF viewer."""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import shutil
import struct
import sys
import urllib.request
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUTPUT_ROOT = ROOT / "target" / "pdf-fixtures"
SOURCE = ROOT / "tests" / "fixtures" / "pdf"
JAPANESE = (
    "日本語の表示を確認するための自作文書です。",
    "春の朝、図書館で新しい本を読みました。",
    "本文の文字が欠けず、行の順番が保たれることを確認します。",
    "ひらがな、カタカナ、漢字、句読点を含みます。",
    "拡大と縮小の後も、中央付近の位置を確認します。",
    "上へ移動し、次に下へ移動して描画を観察します。",
    "この文書に個人情報や第三者の文書内容は含まれません。",
)
SPECS = (
    ("normal.pdf", 1, "valid", "simple baseline", "ASCII text", 0),
    ("japanese.pdf", 3, "valid", "Japanese glyph and layout inspection", "Japanese horizontal text; embedded Noto Sans JP default outlines", 0),
    ("image-heavy.pdf", 12, "valid", "image rendering and switching", "ASCII labels; self-generated RGB raster and vector diagram", 12),
    ("many-pages.pdf", 300, "valid", "long-document scrolling and resource lifetime", "numbered ASCII pages", 0),
    ("malformed.pdf", 0, "invalid", "non-PDF parser rejection", "deliberately invalid bytes, no document", 0),
    ("truncated.pdf", 1, "truncated", "incomplete PDF parser failure path", "first 96 bytes of normal.pdf; no page objects/xref", 0),
)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def confined(path: Path, *, new: bool = False) -> Path:
    """Reject escaping/reparse paths, including an already existing final output."""
    base = OUTPUT_ROOT
    # Checking every component also rejects a redirected target or output root.
    absolute = path.absolute()
    for component in (absolute, *absolute.parents):
        if component.is_symlink() or component.is_junction():
            raise ValueError(f"linked output component: {component}")
    resolved = absolute.resolve()
    if not resolved.is_relative_to(base.resolve()) or resolved == base.resolve():
        raise ValueError("output must be strictly below target/pdf-fixtures")
    if new and resolved.exists():
        raise ValueError("output exists; use a new directory (no overwrite)")
    return resolved


def load_asset(asset_dir: Path, download: bool) -> tuple[bytes, dict]:
    meta = json.loads((SOURCE / "font-source.json").read_text(encoding="utf-8"))
    license_bytes = (SOURCE / "OFL.txt").read_bytes()
    if sha(license_bytes) != meta["license_sha256"]:
        raise ValueError("saved OFL hash mismatch")
    font_path = confined(asset_dir / meta["file"])
    if not font_path.exists():
        if not download:
            raise ValueError("font missing; pass --download-font once or supply verified --assets")
        asset_dir.mkdir(parents=True, exist_ok=True)
        if not meta["url"].startswith("https://raw.githubusercontent.com/google/fonts/"):
            raise ValueError("font download must use the pinned HTTPS source")
        with urllib.request.urlopen(meta["url"], timeout=60) as response:  # noqa: S310 -- HTTPS source checked above
            data = response.read(meta["bytes"] + 1)
        if len(data) != meta["bytes"] or sha(data) != meta["sha256"]:
            raise ValueError("downloaded font identity mismatch; not saved")
        with font_path.open("xb") as output:
            output.write(data)
    data = font_path.read_bytes()
    if len(data) != meta["bytes"] or sha(data) != meta["sha256"]:
        raise ValueError("font identity mismatch")
    return data, meta


class Pdf:
    def __init__(self):
        self.objects: list[bytes] = []

    def add(self, data: bytes | str) -> int:
        self.objects.append(data.encode("ascii") if isinstance(data, str) else data)
        return len(self.objects)

    def stream(self, data: bytes, extra: str = "") -> int:
        compressed = zlib.compress(data, 9)
        return self.add(
            f"<< /Length {len(compressed)} /Filter /FlateDecode {extra} >>\nstream\n".encode()
            + compressed + b"\nendstream"
        )

    def finish(self) -> bytes:
        output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = [0]
        for number, data in enumerate(self.objects, 1):
            offsets.append(len(output))
            output.extend(f"{number} 0 obj\n".encode() + data + b"\nendobj\n")
        xref = len(output)
        output.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
        for offset in offsets[1:]:
            output.extend(f"{offset:010} 00000 n \n".encode())
        output.extend(
            f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
        )
        return bytes(output)


def truetype(data: bytes) -> tuple[dict[int, int], dict[int, int], tuple[int, ...]]:
    """Read sfnt cmap format 12 and metrics; preserve the complete font unchanged."""
    def u16(offset: int) -> int:
        return struct.unpack_from(">H", data, offset)[0]
    tables = {}
    for index in range(u16(4)):
        tag, _, offset, length = struct.unpack_from(">4sIII", data, 12 + index * 16)
        tables[tag.decode("ascii")] = (offset, length)
    cmap = tables["cmap"][0]
    selected = None
    for index in range(u16(cmap + 2)):
        _, _, offset = struct.unpack_from(">HHI", data, cmap + 4 + index * 8)
        if u16(cmap + offset) == 12:
            selected = cmap + offset
            break
    if selected is None:
        raise ValueError("font requires Unicode cmap format 12")
    mapping = {}
    for index in range(struct.unpack_from(">I", data, selected + 12)[0]):
        first, last, glyph = struct.unpack_from(">III", data, selected + 16 + index * 12)
        mapping.update((code, glyph + code - first) for code in range(first, last + 1))
    head = tables["head"][0]
    units = u16(head + 18)
    if units != 1000 or u16(tables["OS/2"][0] + 8) != 0:
        raise ValueError("unexpected font metrics/embedding restrictions")
    hhea = tables["hhea"][0]
    count = u16(hhea + 34)
    hmtx = tables["hmtx"][0]
    widths = {gid: u16(hmtx + min(gid, count - 1) * 4)
              for gid in range(u16(tables["maxp"][0] + 4))}
    bbox = struct.unpack_from(">hhhh", data, head + 36)
    return mapping, widths, bbox


def japanese_font(pdf: Pdf, data: bytes) -> tuple[int, dict[int, int]]:
    mapping, widths, bbox = truetype(data)
    characters = sorted(set("".join(JAPANESE)))
    if any(mapping.get(ord(char), 0) == 0 for char in characters):
        raise ValueError("Japanese fixture contains unsupported glyphs")
    used = {mapping[ord(char)]: ord(char) for char in characters}
    embedded = pdf.stream(data, f"/Length1 {len(data)}")
    descriptor = pdf.add(
        f"<< /Type /FontDescriptor /FontName /NotoSansJP-Thin /Flags 4 "
        f"/FontBBox [{' '.join(map(str, bbox))}] /ItalicAngle 0 /Ascent 1160 "
        f"/Descent -288 /CapHeight 733 /StemV 80 /FontFile2 {embedded} 0 R >>"
    )
    width_list = " ".join(f"{gid} [{widths[gid]}]" for gid in sorted(used))
    descendant = pdf.add(
        f"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NotoSansJP-Thin "
        f"/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> "
        f"/FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{width_list}] >>"
    )
    cmap = [
        "/CIDInit /ProcSet findresource begin", "12 dict begin", "begincmap",
        "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def",
        "/CMapName /FixtureUnicode def", "/CMapType 2 def",
        "1 begincodespacerange", "<0000> <FFFF>", "endcodespacerange",
    ]
    pairs = sorted(used.items())
    for offset in range(0, len(pairs), 100):
        group = pairs[offset:offset + 100]
        cmap.append(f"{len(group)} beginbfchar")
        cmap.extend(f"<{gid:04X}> <{code:04X}>" for gid, code in group)
        cmap.append("endbfchar")
    cmap.extend(["endcmap", "CMapName currentdict /CMap defineresource pop", "end", "end"])
    unicode_map = pdf.stream("\n".join(cmap).encode("ascii"))
    font = pdf.add(
        f"<< /Type /Font /Subtype /Type0 /BaseFont /NotoSansJP-Thin "
        f"/Encoding /Identity-H /DescendantFonts [{descendant} 0 R] /ToUnicode {unicode_map} 0 R >>"
    )
    return font, mapping


def raster(page: int) -> bytes:
    # Integer arithmetic only: deterministic original color field, grid, and diagonal.
    pixels = bytearray()
    for y in range(512):
        for x in range(512):
            grid = x % 64 < 3 or y % 64 < 3 or abs(x - y) < 4
            pixels.extend((20, 30, 45) if grid else (
                (x * 7 + y * 3 + page * 31) % 256,
                (x * 2 + y * 5 + page * 17) % 256,
                (x * 11 + y * 13 + page * 47) % 256,
            ))
    return bytes(pixels)


def document(kind: str, pages: int, font_data: bytes) -> bytes:
    pdf = Pdf()
    pdf.add("<< /Type /Catalog /Pages 2 0 R >>")
    pdf.add("")  # Page tree filled after page IDs are known.
    font = pdf.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    mapping = {}
    if kind == "japanese.pdf":
        font, mapping = japanese_font(pdf, font_data)
    kids = []
    for page in range(1, pages + 1):
        commands = [f"BT /F1 {14 if mapping else 18} Tf 50 790 Td"]
        if mapping:
            commands.extend(
                f"<{''.join(f'{mapping[ord(c)]:04X}' for c in line[offset:offset + 30])}> Tj 0 -30 Td"
                for line in JAPANESE for offset in range(0, len(line), 30)
            )
        else:
            commands.append(f"({kind} - page {page} of {pages}) Tj")
        commands.append("ET")
        resources = f"/Font << /F1 {font} 0 R >>"
        if kind == "image-heavy.pdf":
            image = pdf.stream(raster(page), "/Type /XObject /Subtype /Image /Width 512 /Height 512 /ColorSpace /DeviceRGB /BitsPerComponent 8")
            resources += f" /XObject << /Im1 {image} 0 R >>"
            commands.extend(["q 495 0 0 620 50 100 cm /Im1 Do Q",
                             "0 0 1 RG 2 w 50 80 m 545 80 l 545 60 l S"])
        else:
            frame_height = 500 if mapping else 650
            commands.append(f"0.2 0.4 0.7 RG 50 60 495 {frame_height} re S")
        stream = pdf.stream("\n".join(commands).encode("ascii"))
        kids.append(pdf.add(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] "
            f"/Resources << {resources} >> /Contents {stream} 0 R >>"
        ))
    pdf.objects[1] = (
        f"<< /Type /Pages /Count {pages} /Kids [{' '.join(f'{kid} 0 R' for kid in kids)}] >>".encode()
    )
    return pdf.finish()


def generate(output: Path, assets: Path, download: bool = False) -> dict:
    output = confined(output, new=True)
    assets = confined(assets)
    font, metadata = load_asset(assets, download)
    output.mkdir(parents=True, exist_ok=False)
    entries = []
    normal = b""
    for name, pages, classification, purpose, text, images in SPECS:
        if classification == "valid":
            data = document(name, pages, font)
            if name == "normal.pdf":
                normal = data
        elif classification == "invalid":
            data = b"This is an intentionally malformed PDF fixture.\n"
        else:
            data = normal[:96]
        (output / name).write_bytes(data)
        entries.append({
            "filename": name, "purpose": purpose, "pages": pages,
            "classification": classification, "bytes": len(data), "sha256": sha(data),
            "text": text, "images": images, "authorship": "Self-authored synthetic text, raster and diagrams; no customer/private data",
            "content_license": "CC0-1.0", "embedded_font": metadata["sha256"] if name == "japanese.pdf" else None,
        })
    for name in ("OFL.txt", "font-source.json"):
        shutil.copyfile(SOURCE / name, output / name)
    manifest = {
        "schema_version": 1, "generator": "scripts/generate_pdf_fixtures.py",
        "generator_sha256": sha(Path(__file__).read_bytes()), "python": platform.python_version(),
        "zlib": zlib.ZLIB_RUNTIME_VERSION,
        "command": "python scripts/generate_pdf_fixtures.py --output <new-directory> --assets <verified-assets>",
        "determinism": "No timestamps/randomness; PDF bytes fixed for same source/font/zlib. Manifest records Python version; paths omitted.",
        "font": metadata, "files": entries,
        "rendering_validation": "See docs/testing/pdf-validation.md; structural checker is not a production PDF parser or renderer.",
    }
    (output / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new directory strictly below target/pdf-fixtures")
    parser.add_argument("--assets", type=Path, default=OUTPUT_ROOT / "assets", help="verified font cache below target/pdf-fixtures")
    parser.add_argument("--download-font", action="store_true", help="fetch only the pinned font if missing; hash required")
    args = parser.parse_args()
    try:
        result = generate(args.output, args.assets, args.download_font)
    except (OSError, ValueError, KeyError, struct.error) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"output": str(args.output.resolve()), "files": len(result["files"])}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
