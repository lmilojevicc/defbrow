import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

import release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.tag = "v0.1.0"
        self.version = "0.1.0"
        (self.root / "Cargo.toml").write_text('[package]\nname="defbrow"\nversion="0.1.0"\n')
        (self.root / "Cargo.lock").write_text('[[package]]\nname="defbrow"\nversion="0.1.0"\n')
        self.assets = self.root / "assets"
        self.assets.mkdir()

    def make_assets(self, duplicate_member=False):
        for name in release.archive_names(self.version):
            with tarfile.open(self.assets / name, "w:gz") as archive:
                names = ["defbrow", "LICENSE", "docs/usage.md", "THIRD_PARTY_NOTICES.txt"]
                if duplicate_member:
                    names.append("defbrow")
                for member in names:
                    data = f"fixture {name} {member}".encode()
                    info = tarfile.TarInfo(member)
                    info.size = len(data)
                    info.mode = 0o755 if member == "defbrow" else 0o644
                    archive.addfile(info, io.BytesIO(data))

    def test_stable_tag_syntax(self):
        self.assertEqual(release.version_from_tag(self.tag), self.version)
        for tag in ["0.1.0", "v01.0.0", "v1.2", "v1.2.3-rc.1", "v1.2.3+build", "v1.2.3\n", "v1.2.3;echo bad"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.version_from_tag(tag)

    def test_guard_requires_manifest_and_lock_agreement(self):
        self.assertEqual(release.guard(self.tag, self.root), self.version)
        for file in ["Cargo.toml", "Cargo.lock"]:
            path = self.root / file
            original = path.read_text()
            path.write_text(original.replace("0.1.0", "0.2.0"))
            with self.subTest(file=file), self.assertRaises(ValueError):
                release.guard(self.tag, self.root)
            path.write_text(original)
        with self.assertRaises(ValueError):
            release.guard("v0.2.0", self.root)

    def test_guard_rejects_duplicate_lock_package(self):
        with (self.root / "Cargo.lock").open("a") as lock:
            lock.write('[[package]]\nname="defbrow"\nversion="0.1.0"\n')
        with self.assertRaises(ValueError):
            release.guard(self.tag, self.root)

    def test_complete_assets_roundtrip_and_formula_mapping(self):
        self.make_assets()
        release.manifest(self.assets, self.tag)
        hashes = release.verify(self.assets, self.tag)
        self.assertEqual(len(hashes), 4)
        formula = release.formula(self.assets, self.tag)
        expected = {"aarch64-apple-darwin": "darwin_arm64", "x86_64-apple-darwin": "darwin_amd64",
                    "aarch64-unknown-linux-gnu": "linux_arm64", "x86_64-unknown-linux-gnu": "linux_amd64"}
        self.assertEqual(release.TARGETS, expected)
        for os_name, arm, intel in [("macos", "darwin_arm64", "darwin_amd64"),
                                   ("linux", "linux_arm64", "linux_amd64")]:
            section = formula.split(f"  on_{os_name} do\n")[1].split("  end\n\n")[0]
            for cpu, platform in [("arm", arm), ("intel", intel)]:
                branch = section.split(f"    on_{cpu} do\n")[1].split("    end")[0]
                name = f"defbrow_{self.version}_{platform}.tar.gz"
                self.assertIn(f"/releases/download/{self.tag}/{name}", branch)
                self.assertIn(hashes[name], branch)
        self.assertIn('depends_on macos: :monterey', formula)
        self.assertNotIn('depends_on "xdg-utils"', formula)
        self.assertIn("glibc 2.39", formula)
        self.assertIn('--help', formula)
        self.assertIn('--version', formula)
        self.assertNotIn('defbrow set', formula)
        ruby = self.root / "defbrow.rb"
        ruby.write_text(formula)
        result = subprocess.run(["ruby", "-c", str(ruby)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_or_extra_asset_rejected(self):
        self.make_assets()
        name = release.archive_names(self.version)[0]
        original = (self.assets / name).read_bytes()
        (self.assets / name).unlink()
        with self.assertRaises(ValueError):
            release.manifest(self.assets, self.tag)
        (self.assets / name).write_bytes(original)
        (self.assets / "duplicate.tar.gz").write_bytes(original)
        with self.assertRaises(ValueError):
            release.manifest(self.assets, self.tag)

    def test_duplicate_archive_members_rejected(self):
        self.make_assets(duplicate_member=True)
        with self.assertRaises(ValueError):
            release.manifest(self.assets, self.tag)

    def test_wrong_checksum_rejected(self):
        self.make_assets()
        release.manifest(self.assets, self.tag)
        sums = self.assets / "SHA256SUMS"
        lines = sums.read_text().splitlines()
        lines[0] = "0" * 64 + lines[0][64:]
        sums.write_text("\n".join(lines) + "\n")
        with self.assertRaises(ValueError):
            release.verify(self.assets, self.tag)
        with self.assertRaises(ValueError):
            release.formula(self.assets, self.tag)

    def test_missing_duplicate_or_malformed_checksum_rejected(self):
        self.make_assets()
        release.manifest(self.assets, self.tag)
        sums = self.assets / "SHA256SUMS"
        original = sums.read_text()
        for content in [original.splitlines()[0] + "\n", original + original.splitlines()[0] + "\n",
                        original + "bad\n"]:
            sums.write_text(content)
            with self.subTest(content=content), self.assertRaises(ValueError):
                release.verify(self.assets, self.tag)

    def metadata(self, dependency):
        return {"packages": [{"id": "root", "name": "defbrow", "version": self.version}, dependency],
                "resolve": {"root": "root", "nodes": [
                    {"id": "root", "dependencies": [dependency["id"]]},
                    {"id": dependency["id"], "dependencies": []}]}}

    def test_notices_include_declared_license_and_copyright(self):
        (self.root / "LICENSE-MIT").write_text("fixture license")
        (self.root / "COPYRIGHT").write_text("fixture copyright")
        dependency = {"id": "dep", "name": "fixture", "version": "1.0.0", "license": "MIT",
                      "manifest_path": str(self.root / "Cargo.toml")}
        notices = release.dependency_notices(self.metadata(dependency))
        self.assertIn("fixture license", notices)
        self.assertIn("fixture copyright", notices)
        (self.root / "LICENSE-MIT").unlink()
        (self.root / "COPYRIGHT").unlink()
        with self.assertRaisesRegex(ValueError, "missing dependency license"):
            release.dependency_notices(self.metadata(dependency))

    def test_objc_notice_is_scoped_to_reviewed_versions(self):
        dependency = {"id": "dep", "name": "objc2", "version": "0.6.4", "license": "MIT",
                      "repository": "https://github.com/madsmtm/objc2", "authors": ["Fixture Author"],
                      "manifest_path": str(self.root / "Cargo.toml")}
        notices = release.dependency_notices(self.metadata(dependency))
        self.assertIn("Fixture Author", notices)
        self.assertIn("Apple SDKs", notices)
        self.assertIn("Permission is hereby granted", notices)
        dependency["version"] = "0.7.0"
        with self.assertRaises(ValueError):
            release.dependency_notices(self.metadata(dependency))

    def test_package_contains_only_expected_files_and_refuses_overwrite(self):
        binary = self.root / "defbrow"
        binary.write_bytes(b"fixture executable")
        binary.chmod(0o755)
        metadata = self.root / "metadata.json"
        metadata.write_text(json.dumps({"packages": [{"id": "root", "name": "defbrow", "version": self.version}],
                                       "resolve": {"root": "root", "nodes": [{"id": "root", "dependencies": []}]}}))
        tag = "v" + release.tomllib.loads((release.ROOT / "Cargo.toml").read_text())["package"]["version"]
        archive = release.package(tag, "aarch64-apple-darwin", binary, metadata, self.assets)
        with tarfile.open(archive) as tar:
            self.assertEqual({m.name for m in tar.getmembers()}, {
                "defbrow", "LICENSE", "docs/usage.md", "THIRD_PARTY_NOTICES.txt"})
            self.assertTrue(tar.getmember("defbrow").mode & 0o111)
        with self.assertRaisesRegex(ValueError, "refusing to replace"):
            release.package(tag, "aarch64-apple-darwin", binary, metadata, self.assets)
        binary.chmod(0o644)
        with self.assertRaisesRegex(ValueError, "executable"):
            release.package(tag, "aarch64-apple-darwin", binary, metadata, self.assets)


if __name__ == "__main__":
    unittest.main()
