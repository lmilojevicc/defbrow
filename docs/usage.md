# Usage

```sh
defbrow                         # interactive picker
defbrow list                    # names, IDs, per-scheme current markers, details
defbrow current                 # HTTP and HTTPS defaults separately
defbrow set <id-or-name>         # exact ID or unambiguous name (quote spaces)
defbrow --help
defbrow --version
```

The platform backends are not integrated in this foundation yet. `list`, `current`, `set`, and the interactive picker currently return an explicit not-implemented error. The following describes the intended integrated interface.

## Picker

Type to search names, IDs, and details using case-insensitive fuzzy subsequence matching: `brv` matches Brave and `lbrwf` matches LibreWolf. Compact matches near the start rank first; ties are deterministic. Clearing the search restores discovery order. Unicode input and lowercase matching are supported (not locale-specific case folding); Backspace removes one Unicode scalar. Use Ctrl-U to clear, arrows to navigate, and Enter to set. Escape or Ctrl-C cancels without changing anything. HTTP/HTTPS markers show each current default, even when they differ.

The picker inherits the terminal's default foreground/background and uses an ANSI palette accent, not a fixed light/dark theme. Selection uses the terminal's reversed colors and a `>` indicator. Set a nonempty `NO_COLOR` to disable explicit accents. There is no theme configuration or OSC color probing.

No matches or an empty handler list cannot be selected. At sizes below 32 columns by 10 rows, resize the terminal or cancel; Enter is disabled while the list is hidden. Resizing redraws the picker. The terminal is restored on return, error, or panic, and before an OS confirmation prompt. Forced process termination (such as SIGKILL) cannot run cleanup.

Interactive invocation requires terminals on both stdin and stdout. For pipes or scripts, use an explicit subcommand instead. Invalid/ambiguous names fail without changes; prefer the exact ID printed by `list`. Errors exit nonzero.

## Discovery and platform requirements

Browser candidates are installed, registered handlers for **both HTTP and HTTPS**, not a hardcoded list of browser brands. Safari, Chromium/Brave, Firefox/LibreWolf, and other handlers qualify automatically when registered. Unregistered executables are not detected; an OS-registered URL handler is not guaranteed to be a full browser.

- **macOS 12+:** Native NSWorkspace APIs discover handlers. IDs identify installed applications deterministically (bundle ID or canonical app URL/path). Changing defaults uses supported APIs and may require OS consent; approval cannot be bypassed. Only HTTP/HTTPS URL schemes are changed, not local HTML file associations.
- **Linux:** Discovery uses desktop entries in the session's XDG application paths. Desktop-file IDs identify handlers, including exported package entries when the session exposes them. Install `xdg-utils` using your distribution's package manager and run within your user's graphical desktop session, not with sudo. Desktop/session configuration and overrides such as `BROWSER` can affect actual URL opening.

HTTP and HTTPS changes are not transactional. If a request is rejected or a scheme fails, one scheme may already have changed. Errors must report the actual per-scheme state rather than claim success; use `defbrow current` to inspect it. No automatic rollback is promised.

## Build and install

Use a current stable Rust toolchain. Native macOS compilation also requires Apple's Command Line Tools. Linux requires xdg-utils at runtime and a graphical desktop for system integration.

```sh
cargo build --release --locked
./target/release/defbrow --help
cargo install --path . --locked --root "$HOME/.local"
# Ensure $HOME/.local/bin is on PATH.
```

Development checks use fake backends and must never change real host defaults:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Actual OS consent and graphical desktop behavior require separate, explicitly approved manual verification after platform integration.
