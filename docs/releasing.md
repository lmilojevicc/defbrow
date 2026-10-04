# Releases and Homebrew distribution

## Contract

`Cargo.toml` owns the CLI version; `Cargo.lock` must contain the same defbrow version. Only stable tags of the form `vMAJOR.MINOR.PATCH`, without leading zeros, are accepted. The tagged commit must be reachable from `main`. Main-branch CI retains the required check names `checks (ubuntu-latest)` and `checks (macos-latest)`; release automation reuses both checks before building.

Native release builds test and safely smoke-test `--help` and the exact `--version` on four targets:

| Target | Runner | Archive suffix |
| --- | --- | --- |
| aarch64-apple-darwin | macos-15 (ARM64) | darwin_arm64 |
| x86_64-apple-darwin | macos-15-intel | darwin_amd64 |
| aarch64-unknown-linux-gnu | ubuntu-24.04-arm | linux_arm64 |
| x86_64-unknown-linux-gnu | ubuntu-24.04 | linux_amd64 |

Archives are named `defbrow_0.1.0_darwin_arm64.tar.gz` (substitute the actual version and suffix). Each contains `defbrow`, `LICENSE`, `docs/usage.md`, and `THIRD_PARTY_NOTICES.txt`. No real browser setter, picker or default inspection runs during packaging or Homebrew tests.

macOS requires 12+ (`MACOSX_DEPLOYMENT_TARGET=12.0`). Linux binaries conservatively require glibc 2.39+ (Ubuntu 24.04 or equivalent), not musl or universal distribution compatibility. Builds check that referenced glibc symbol versions do not exceed that floor. Runtime integration still requires distribution-managed `xdg-utils` and a normal user's graphical desktop session; xdg-utils is not a Homebrew formula dependency. Source builds remain an option for older glibc environments.

## Before a release

1. Update the package version and its lockfile entry together in a reviewed PR. Keep the README's installation claims accurate.
2. Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked` and `python3 -m unittest discover -s scripts -p 'test_*.py'` (Python 3.11+ and Ruby required). Both mandatory CI platforms must pass. Only fake backends are allowed for automated switching tests.
3. Review locked dependency notices. Packaging collects the applicable Cargo dependency graph's published license/copyright/notice texts, including build-time dependencies conservatively. An unknown dependency lacking license text fails packaging. The locked objc2 family omits shared license files from its crate archives; a reviewed version allowlist uses vendored immutable upstream `LICENSE.md`, MIT text, and actual Cargo-author attribution instead. Update/review that allowlist when these dependencies change. Other dependencies retain their published alternative-license texts; defbrow's MIT license does not replace theirs.
4. Review the upstream objc2 caveat: its [license document](https://github.com/madsmtm/objc2/blob/8852b424193ca41602281b3d7540d7c8ed51e49a/LICENSE.md) notes uncertainty concerning Apple SDK-derived bindings and the Xcode license. Including notices does not resolve that upstream licensing uncertainty or assert new distribution permissions.
5. Confirm native runners and the optional tap credential. The repository should retain read-only default tokens and no PR-approval permission. Only the publishing job has `contents: write`; the tap job's defbrow token stays read-only. Actions are pinned to commits and defbrow checkouts do not persist credentials.

The maintainer, not development scripts, creates and pushes the approved version tag. No release is created by local validation. The first intended release is `v0.1.0`; this document does not assert it already exists.

## Publication and retries

The release workflow collects exactly four archives and creates `SHA256SUMS` with all four hashes. It validates archive contents, creates a draft, uploads the assets, downloads the entire draft asset set, verifies the downloaded hashes and compares its manifest with the build manifest. Only then is the release made public/latest. Failed builds never create a release; a failed upload/verification leaves a draft rather than a partial public release.

A workflow re-run may resume an existing draft. It refuses to modify an already published release. Unexpected draft assets cause verification failure: inspect/remove them before retrying. A missing tag/main relationship or version mismatch must be corrected explicitly; do not bypass the guard. Published assets are immutable by policy. If only the optional tap job fails after publication, rerun that failed job or perform the manual update below, not a full release publication.

## Optional shared-tap update

Provision the Actions secret `HOMEBREW_TAP_GITHUB_TOKEN` with a fine-grained PAT restricted to `lmilojevicc/homebrew-tap`, **Contents: read and write**, and only required repository metadata access. Set a bounded expiration and rotate it. Do not use an account-wide token or put its value in source, command output, artifacts, or documentation. The defbrow `GITHUB_TOKEN` cannot write another repository.

When the secret is present, the post-publication job downloads and verifies the public release, generates `Formula/defbrow.rb` with the real exact URLs/hashes, checks Ruby syntax and runs Homebrew style, strict audit, installation and safe formula tests on Linux x86_64. Only that file is staged/committed. Non-fast-forward pushes are retried against the latest tap branch without overwriting other formulas; a newer defbrow formula is not downgraded. Credentials persist only in the isolated tap checkout needed to push. No tap protection settings are changed.

When the secret is missing, the release still succeeds, with a visible warning and job summary explaining that Homebrew is pending. If the tap validation/push fails, the published release remains available and the failed tap job requires maintainer attention. Neither case authorizes a README claim that `brew install` is available.

## Manual first formula or recovery

After a release is public, the maintainer downloads **all** its assets into a clean directory, then runs:

```sh
python3 scripts/release.py verify v0.1.0 /path/to/downloaded-assets
python3 scripts/release.py formula v0.1.0 /path/to/downloaded-assets /path/to/defbrow.rb
ruby -c /path/to/defbrow.rb
```

Use the actual tag. No placeholder checksums or generated fixture assets are acceptable for the shared tap. The maintainer integrates this file as `Formula/defbrow.rb` in the up-to-date shared tap, preserving unrelated remote work, and commits only the formula after review. Test the formula on available macOS/Linux architectures with `brew style`, `brew audit --strict`, `brew install`, and `brew test`; the formula tests only `--help` and the exact package version. Validate macOS minimum-version and native architecture compatibility as machines become available. Actual OS consent/default changes require separately approved disposable-environment testing.

Only after successful publication, formula integration and installation should the README advertise `brew install lmilojevicc/homebrew-tap/defbrow`. Branch-protection settings, PAT provisioning, tags, releases and all shared-tap integration remain maintainer-owned control-plane actions.
