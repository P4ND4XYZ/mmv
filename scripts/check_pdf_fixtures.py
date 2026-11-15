"""Check this generated fixture format, hashes and expected-invalid cases (not a general PDF validator)."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import zlib
from pathlib import Path

EXPECTED = {
    "normal.pdf": (1, "valid", 0),
    "japanese.pdf": (3, "valid", 0),
    "image-heavy.pdf": (12, "valid", 12),
    "many-pages.pdf": (300, "valid", 0),
    "malformed.pdf": (0, "invalid", 0),
    "truncated.pdf": (1, "truncated", 0),
}
SOURCE = Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "pdf"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def structure(data: bytes) -> tuple[dict[int, bytes], list[bytes]]:
    """Validate writer's classic xref, lengths, references, page tree and streams."""
    if not data.startswith(b"%PDF-1.7\n"):
        raise ValueError("missing PDF header")
    ending = re.search(rb"startxref\n(\d+)\n%%EOF\n\Z", data)
    if not ending:
        raise ValueError("missing complete xref/EOF")
    start = int(ending[1])
    xref = re.fullmatch(
        rb"xref\n0 (\d+)\n(.*?)trailer\n<< /Size (\d+) /Root 1 0 R >>\n",
        data[start:ending.start()], re.DOTALL,
    )
    if not xref or xref[1] != xref[3]:
        raise ValueError("invalid xref/trailer")
    lines = xref[2].splitlines()
    count = int(xref[1])
    if len(lines) != count or lines[0] != b"0000000000 65535 f ":
        raise ValueError("invalid xref count/free entry")
    offsets = []
    for line in lines[1:]:
        if not re.fullmatch(rb"\d{10} 00000 n ", line):
            raise ValueError("invalid xref entry")
        offsets.append(int(line[:10]))
    if offsets != sorted(set(offsets)) or not offsets or offsets[-1] >= start:
        raise ValueError("invalid object offsets")
    objects = {}
    streams = []
    for number, (offset, end) in enumerate(zip(offsets, offsets[1:] + [start], strict=True), 1):
        block = data[offset:end]
        prefix = f"{number} 0 obj\n".encode()
        if not block.startswith(prefix) or not block.endswith(b"\nendobj\n"):
            raise ValueError("xref does not point to complete object")
        body = block[len(prefix):-8]
        dictionary = body
        if b"\nstream\n" in body:
            dictionary, raw = body.split(b"\nstream\n", 1)
            length = re.search(rb"/Length (\d+)\b", dictionary)
            if not length or not raw.endswith(b"\nendstream") or len(raw[:-10]) != int(length[1]):
                raise ValueError("stream length mismatch")
            decoded = zlib.decompress(raw[:-10])
            if b"/Subtype /Image" in dictionary and len(decoded) != 512 * 512 * 3:
                raise ValueError("RGB image size mismatch")
            streams.append(decoded)
        objects[number] = body
        # Only dictionaries, never arbitrary compressed/font bytes, contain PDF references.
        for reference in re.findall(rb"\b(\d+) 0 R\b", dictionary):
            if not 1 <= int(reference) < count:
                raise ValueError("dangling object reference")
    if objects.get(1) != b"<< /Type /Catalog /Pages 2 0 R >>":
        raise ValueError("missing fixture catalog")
    tree = re.fullmatch(rb"<< /Type /Pages /Count (\d+) /Kids \[(.*?)\] >>", objects.get(2, b""))
    if not tree:
        raise ValueError("invalid page tree")
    kids = [int(value) for value in re.findall(rb"(\d+) 0 R", tree[2])]
    actual = [number for number, obj in objects.items() if b"/Type /Page " in obj.split(b"\nstream\n")[0]]
    if kids != actual or len(kids) != int(tree[1]):
        raise ValueError("page tree does not match pages")
    for kid in kids:
        if b"/Parent 2 0 R" not in objects[kid] or b"/MediaBox [0 0 595 842]" not in objects[kid]:
            raise ValueError("invalid page parent or dimensions")
    return objects, streams


def check(directory: Path) -> list[dict]:
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        raise ValueError("unsupported manifest schema")
    if manifest["generator_sha256"] != digest((SOURCE.parents[2] / "scripts" / "generate_pdf_fixtures.py").read_bytes()):
        raise ValueError("generator source identity mismatch; regenerate with current source")
    entries = manifest["files"]
    if len(entries) != len(EXPECTED) or {entry["filename"] for entry in entries} != set(EXPECTED):
        raise ValueError("fixture set mismatch")
    font = json.loads((SOURCE / "font-source.json").read_text(encoding="utf-8"))
    if manifest["font"] != font or (directory / "font-source.json").read_bytes() != (SOURCE / "font-source.json").read_bytes():
        raise ValueError("font provenance mismatch")
    if digest((directory / "OFL.txt").read_bytes()) != font["license_sha256"]:
        raise ValueError("OFL hash mismatch")
    results = []
    for entry in entries:
        name = entry["filename"]
        pages, classification, images = EXPECTED[name]
        if (entry["pages"], entry["classification"], entry["images"]) != (pages, classification, images):
            raise ValueError(f"{name}: specification mismatch")
        path = directory / name
        if path.is_symlink() or path.is_junction() or not path.is_file():
            raise ValueError(f"{name}: not a regular fixture")
        data = path.read_bytes()
        if len(data) != entry["bytes"] or digest(data) != entry["sha256"]:
            raise ValueError(f"{name}: manifest hash/size mismatch")
        if classification != "valid":
            if name == "malformed.pdf" and data != b"This is an intentionally malformed PDF fixture.\n":
                raise ValueError("malformed fixture differs from deliberate invalid content")
            if name == "truncated.pdf" and data != (directory / "normal.pdf").read_bytes()[:96]:
                raise ValueError("truncated fixture is not the expected prefix")
            try:
                structure(data)
            except ValueError:
                results.append({"filename": name, "result": "expected-" + classification, "intended_pages": pages})
                continue
            raise ValueError(f"{name}: invalid fixture accepted structurally")
        objects, streams = structure(data)
        page_count = sum(b"/Type /Page " in obj.split(b"\nstream\n")[0] for obj in objects.values())
        image_count = sum(b"/Subtype /Image" in obj.split(b"\nstream\n")[0] for obj in objects.values())
        if page_count != pages or image_count != images:
            raise ValueError(f"{name}: page/image count mismatch")
        if name == "japanese.pdf":
            if entry["embedded_font"] != font["sha256"] or not any(digest(stream) == font["sha256"] for stream in streams):
                raise ValueError("embedded complete font identity mismatch")
            unicode_maps = [stream for stream in streams if b"begincmap" in stream]
            if len(unicode_maps) != 1 or b"<65E5>" not in unicode_maps[0] or b"<672C>" not in unicode_maps[0]:
                raise ValueError("Japanese Unicode mapping missing")
        results.append({"filename": name, "result": "structure-checked", "pages": page_count, "images": image_count})
    return results


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        results = check(args.directory)
    except (OSError, ValueError, KeyError, TypeError, zlib.error) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"scope": "fixture structure and identity, not rendering or performance", "files": results}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
