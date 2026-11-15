"""CLI/filesystem integration tests plus focused writer contract tests; no network."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import check_pdf_fixtures as checker
import generate_pdf_fixtures as generator


class WriterTests(unittest.TestCase):
    def test_baseline_is_one_real_page_with_consistent_xref(self):
        objects, streams = checker.structure(generator.document("normal.pdf", 1, b""))
        self.assertEqual(sum(b"/Type /Page " in obj for obj in objects.values()), 1)
        self.assertTrue(any(b"normal.pdf - page 1 of 1" in stream for stream in streams))

    def test_truncated_document_is_not_structural_success(self):
        data = generator.document("normal.pdf", 1, b"")
        with self.assertRaisesRegex(ValueError, "xref/EOF"):
            checker.structure(data[:96])

    def test_xref_corruption_is_rejected(self):
        data = generator.document("normal.pdf", 1, b"")
        data = data.replace(b"0000000015 00000 n", b"0000000016 00000 n")
        with self.assertRaisesRegex(ValueError, "xref"):
            checker.structure(data)


class CliTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        generator.OUTPUT_ROOT.mkdir(parents=True, exist_ok=True)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="cli-test-", dir=generator.OUTPUT_ROOT)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def cli(self, script, *arguments):
        environment = dict(os.environ)
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        return subprocess.run(
            [sys.executable, str(generator.ROOT / "scripts" / script), *map(str, arguments)],
            cwd=generator.ROOT, capture_output=True, text=True, timeout=60, env=environment,
            check=False,
        )

    def test_git_checkout_filters_preserve_exact_upstream_license(self):
        # Use real Git conversion and working-tree attributes, without changing the index/files.
        result = subprocess.run(
            ["git", "-c", "core.autocrlf=true", "cat-file", "--filters",
             "HEAD:tests/fixtures/pdf/OFL.txt"],
            cwd=generator.ROOT, capture_output=True, timeout=10, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout), 4388)
        self.assertEqual(
            generator.sha(result.stdout),
            "1c05c68c34f9708415aada51f17e1b0092d2cea709bf4a94cd38114f9e73d7d9",
        )
        self.assertEqual(result.stdout, (generator.SOURCE / "OFL.txt").read_bytes())

    def test_help_and_missing_argument_exit_contract(self):
        for script in ("generate_pdf_fixtures.py", "check_pdf_fixtures.py"):
            with self.subTest(script=script):
                self.assertEqual(self.cli(script, "--help").returncode, 0)
                result = self.cli(script)
                self.assertEqual(result.returncode, 2)
                self.assertIn("usage:", result.stderr)

    def test_escape_and_existing_directory_are_rejected_without_overwrite(self):
        result = self.cli("generate_pdf_fixtures.py", "--output", generator.ROOT / "outside.pdf")
        self.assertEqual(result.returncode, 2)
        self.assertIn("strictly below", result.stderr)
        sentinel = self.root / "keep.txt"
        sentinel.write_text("preserve", encoding="utf-8")
        result = self.cli("generate_pdf_fixtures.py", "--output", self.root)
        self.assertEqual(result.returncode, 2)
        self.assertIn("output exists", result.stderr)
        self.assertEqual(sentinel.read_text(encoding="utf-8"), "preserve")

    def test_linked_output_is_rejected_without_writing_destination(self):
        destination = self.root / "destination"
        destination.mkdir()
        linked = self.root / "linked"
        if os.name == "nt":
            result = subprocess.run(
                ["cmd", "/c", "mklink", "/J", str(linked), str(destination)],
                capture_output=True, text=True, timeout=10, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            linked.symlink_to(destination, target_is_directory=True)
        result = self.cli("generate_pdf_fixtures.py", "--output", linked / "corpus")
        self.assertEqual(result.returncode, 2)
        self.assertIn("linked output component", result.stderr)
        self.assertEqual(list(destination.iterdir()), [])
        linked.rmdir() if os.name == "nt" else linked.unlink()

    def test_invalid_manifest_shape_has_structured_cli_error(self):
        (self.root / "manifest.json").write_text("[]", encoding="utf-8")
        result = self.cli("check_pdf_fixtures.py", self.root)
        self.assertEqual(result.returncode, 2)
        self.assertIn("unsupported manifest schema", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_missing_and_corrupt_font_fail_before_output_creation(self):
        assets = self.root / "assets"
        output = self.root / "corpus"
        result = self.cli("generate_pdf_fixtures.py", "--output", output, "--assets", assets)
        self.assertEqual(result.returncode, 2)
        self.assertIn("font missing", result.stderr)
        self.assertFalse(output.exists())
        assets.mkdir()
        metadata = json.loads((generator.SOURCE / "font-source.json").read_text(encoding="utf-8"))
        (assets / metadata["file"]).write_bytes(b"wrong")
        result = self.cli("generate_pdf_fixtures.py", "--output", output, "--assets", assets)
        self.assertEqual(result.returncode, 2)
        self.assertIn("identity mismatch", result.stderr)
        self.assertFalse(output.exists())

    def test_real_cli_generation_and_manifest_tampering_rejected(self):
        output = self.root / "corpus"
        result = self.cli("generate_pdf_fixtures.py", "--output", output)
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.cli("check_pdf_fixtures.py", output)
        self.assertEqual(result.returncode, 0, result.stderr)
        entries = {entry["filename"]: entry for entry in json.loads(result.stdout)["files"]}
        self.assertEqual(entries["many-pages.pdf"]["pages"], 300)
        self.assertEqual(entries["truncated.pdf"]["result"], "expected-truncated")
        self.assertEqual(entries["malformed.pdf"]["result"], "expected-invalid")
        with (output / "normal.pdf").open("ab") as file:
            file.write(b"tamper")
        result = self.cli("check_pdf_fixtures.py", output)
        self.assertEqual(result.returncode, 2)
        self.assertIn("hash/size mismatch", result.stderr)

    def test_manifest_cannot_reclassify_invalid_file_as_valid(self):
        output = self.root / "corpus"
        self.assertEqual(self.cli("generate_pdf_fixtures.py", "--output", output).returncode, 0)
        path = output / "manifest.json"
        manifest = json.loads(path.read_text(encoding="utf-8"))
        manifest["files"][-1]["classification"] = "valid"
        path.write_text(json.dumps(manifest), encoding="utf-8")
        result = self.cli("check_pdf_fixtures.py", output)
        self.assertEqual(result.returncode, 2)
        self.assertIn("specification mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
