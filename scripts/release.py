#!/usr/bin/env python3
"""Locked-version packaging and binary Homebrew formula generation (stdlib only)."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "aarch64-apple-darwin": "darwin_arm64",
    "x86_64-apple-darwin": "darwin_amd64",
    "aarch64-unknown-linux-gnu": "linux_arm64",
    "x86_64-unknown-linux-gnu": "linux_amd64",
}
# These published crates omit the repository's shared license file.
OBJC_VERSIONS = {
    "objc2": "0.6.4", "block2": "0.6.2", "objc2-encode": "4.1.0",
    "objc2-foundation": "0.3.2", "objc2-app-kit": "0.3.2",
    "objc2-core-foundation": "0.3.2", "dispatch2": "0.3.1",
}


def version_from_tag(tag):
    if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag):
        raise ValueError("release tag must be stable vMAJOR.MINOR.PATCH (no leading zeros)")
    return tag[1:]


def guard(tag, root=ROOT):
    version = version_from_tag(tag)
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    packages = [p for p in lock["package"] if p["name"] == "defbrow"]
    if (manifest["package"]["name"] != "defbrow"
            or manifest["package"]["version"] != version
            or len(packages) != 1 or packages[0]["version"] != version):
        raise ValueError("tag, Cargo.toml and Cargo.lock defbrow versions must agree")
    return version


def archive_names(version):
    return [f"defbrow_{version}_{platform}.tar.gz" for platform in TARGETS.values()]


def dependency_notices(metadata):
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    seen, pending = set(), [metadata["resolve"]["root"]]
    while pending:
        ident = pending.pop()
        if ident in seen:
            continue
        seen.add(ident)
        pending.extend(nodes[ident]["dependencies"])
    notices = ["Third-party dependency notices\nLicenses below apply to their respective dependencies, not defbrow.\n"]
    for p in sorted((packages[i] for i in seen), key=lambda p: (p["name"], p["version"])):
        if p["name"] == "defbrow":
            continue
        directory = Path(p["manifest_path"]).parent
        files = sorted(f for f in directory.iterdir() if f.is_file() and re.match(
            r"^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)(?:$|[._-])", f.name, re.I))
        if p.get("license_file"):
            declared = directory / p["license_file"]
            if declared not in files:
                files.append(declared)
        notices.append(f"\n=== {p['name']} {p['version']} ({p.get('license')}) ===\n")
        if files:
            for file in files:
                notices.append(f"\n--- {file.name} ---\n{file.read_text()}\n")
        elif (OBJC_VERSIONS.get(p["name"]) == p["version"]
              and p.get("repository") == "https://github.com/madsmtm/objc2"):
            notices.append("Upstream Cargo authors: " + "; ".join(p["authors"]) + "\n")
            notices.append((ROOT / "scripts/licenses/objc2-LICENSE.md").read_text())
            notices.append((ROOT / "scripts/licenses/MIT.txt").read_text())
        else:
            raise ValueError(f"missing dependency license text: {p['name']} {p['version']}")
    return "\n".join(notices)


def package(tag, target, binary, metadata, output):
    version = guard(tag)
    if not binary.is_file() or not binary.stat().st_mode & 0o111:
        raise ValueError("release binary must exist and be executable")
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"defbrow_{version}_{TARGETS[target]}.tar.gz"
    if archive.exists():
        raise ValueError(f"refusing to replace {archive}")
    with tempfile.TemporaryDirectory() as temporary:
        notice = Path(temporary) / "THIRD_PARTY_NOTICES.txt"
        notice.write_text(dependency_notices(json.loads(metadata.read_text())))
        with tarfile.open(archive, "w:gz") as tar:
            for path, name in [(binary, "defbrow"), (ROOT / "LICENSE", "LICENSE"),
                               (ROOT / "docs/usage.md", "docs/usage.md"),
                               (notice, "THIRD_PARTY_NOTICES.txt")]:
                tar.add(path, arcname=name, recursive=False)
    return archive


def check_assets(directory, version, has_manifest):
    expected = set(archive_names(version))
    if has_manifest:
        expected.add("SHA256SUMS")
    if {p.name for p in directory.iterdir()} != expected:
        raise ValueError("release must contain exactly four expected archives" +
                         (" and SHA256SUMS" if has_manifest else ""))
    hashes = {}
    for name in archive_names(version):
        path = directory / name
        if not path.is_file() or path.is_symlink():
            raise ValueError("release assets must be regular files")
        with tarfile.open(path, "r:gz") as tar:
            members = tar.getmembers()
            if (len(members) != 4 or {m.name for m in members} != {
                    "defbrow", "LICENSE", "docs/usage.md", "THIRD_PARTY_NOTICES.txt"}
                    or any(not m.isfile() or m.size == 0 for m in members)
                    or not tar.getmember("defbrow").mode & 0o111):
                raise ValueError(f"invalid archive contents: {name}")
        hashes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return hashes


def manifest(directory, tag):
    hashes = check_assets(directory, version_from_tag(tag), False)
    (directory / "SHA256SUMS").write_text("".join(
        f"{digest}  {name}\n" for name, digest in hashes.items()))


def verify(directory, tag):
    hashes = check_assets(directory, version_from_tag(tag), True)
    recorded = {}
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([a-zA-Z0-9_.-]+)", line)
        if not match or match[2] in recorded:
            raise ValueError("invalid or duplicate checksum entry")
        recorded[match[2]] = match[1]
    if recorded != hashes:
        raise ValueError("SHA256SUMS must match all four release archives")
    return hashes


def formula(directory, tag):
    version = version_from_tag(tag)
    hashes = verify(directory, tag)

    def stanza(platform, indent):
        name = f"defbrow_{version}_{platform}.tar.gz"
        prefix = " " * indent
        return (f'{prefix}url "https://github.com/lmilojevicc/defbrow/releases/download/{tag}/{name}"\n'
                f'{prefix}sha256 "{hashes[name]}"\n')

    return (f'class Defbrow < Formula\n'
            '  desc "Searchable default-browser picker for macOS and Linux"\n'
            '  homepage "https://github.com/lmilojevicc/defbrow"\n'
            f'  version "{version}"\n'
            '  license "MIT"\n\n'
            '  on_macos do\n'
            '    depends_on macos: :monterey\n'
            '    on_arm do\n' + stanza("darwin_arm64", 6) + '    end\n'
            '    on_intel do\n' + stanza("darwin_amd64", 6) + '    end\n'
            '  end\n\n'
            '  on_linux do\n'
            '    on_arm do\n' + stanza("linux_arm64", 6) + '    end\n'
            '    on_intel do\n' + stanza("linux_amd64", 6) + '    end\n'
            '  end\n\n'
            '  def install\n'
            '    bin.install "defbrow"\n'
            '    doc.install "LICENSE", "THIRD_PARTY_NOTICES.txt", "docs/usage.md"\n'
            '  end\n\n'
            '  def caveats\n'
            '    <<~EOS\n'
            '      On Linux, install xdg-utils with your distribution package manager\n'
            '      and use a graphical desktop session as your normal user.\n'
            '      The Linux binaries require glibc 2.39 or newer (Ubuntu 24.04+).\n'
            '      On macOS, changing defaults may require OS consent.\n'
            '    EOS\n'
            '  end\n\n'
            '  test do\n'
            '    assert_match "Usage:", shell_output("#{bin}/defbrow --help")\n'
            '    assert_equal "defbrow #{version}", shell_output("#{bin}/defbrow --version").strip\n'
            '  end\n'
            'end\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("guard").add_argument("tag")
    pack = sub.add_parser("package")
    pack.add_argument("tag")
    pack.add_argument("target", choices=TARGETS)
    pack.add_argument("binary", type=Path)
    pack.add_argument("metadata", type=Path)
    pack.add_argument("output", type=Path)
    for name in ("manifest", "verify", "formula"):
        command = sub.add_parser(name)
        command.add_argument("tag")
        command.add_argument("directory", type=Path)
        if name == "formula":
            command.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "guard":
            print(guard(args.tag))
        elif args.command == "package":
            print(package(args.tag, args.target, args.binary, args.metadata, args.output))
        elif args.command == "manifest":
            manifest(args.directory, args.tag)
        elif args.command == "verify":
            verify(args.directory, args.tag)
        elif args.command == "formula":
            args.output.write_text(formula(args.directory, args.tag))
    except (ValueError, OSError, tarfile.TarError) as error:
        parser.exit(1, f"release: {error}\n")


if __name__ == "__main__":
    main()
