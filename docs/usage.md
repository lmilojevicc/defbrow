# Usage

```sh
defbrow                         # interactive picker
defbrow list                    # names, IDs, per-scheme current markers, details
defbrow current                 # HTTP and HTTPS defaults separately
defbrow set <id-or-name>         # exact ID or unambiguous name (quote spaces)
defbrow --help
defbrow --version
```

## Picker

Type to search names, IDs, and details using case-insensitive fuzzy subsequence matching: `brv` matches Brave and `lwf` matches LibreWolf. Compact matches near the start rank first; ties are deterministic. Clearing the search restores discovery order. Unicode input and lowercase matching are supported (not locale-specific case folding); Backspace removes one Unicode scalar. Use Ctrl-U to clear, arrows to navigate, and Enter to set. Escape or Ctrl-C cancels without changing anything. Rows show browser names and a `[current]` marker for either current default; when the defaults differ, both browsers are marked without protocol labels. Details appear only to distinguish duplicate names. Use `defbrow current` for per-scheme defaults.

The compact picker has a one-cell outer inset around the Search box, browser list and one-line help footer, with no app banner. Browser names, list labels and Search text use the terminal's default foreground; only the literal `[current]` tag is yellow (ANSI 3), even on the selected row. Borders, help and unselected disambiguating details remain dark gray (ANSI 8); selected details use the default foreground for contrast. Keyboard selection uses a `>` pointer and a neutral dark-gray (ANSI 8) row background, not reversed or recolored text; other backgrounds stay inherited, with no fixed light/dark theme. Set a nonempty `NO_COLOR` to disable all explicit colors, including row shading, while retaining the pointer and bold current tags. There is no theme configuration or OSC color probing.

No matches or an empty handler list cannot be selected. At sizes below 32 columns by 10 rows, resize the terminal or cancel; Enter is disabled while the list is hidden. Resizing redraws the picker. The terminal is restored on return, error, or panic, and before an OS confirmation prompt. Forced process termination (such as SIGKILL) cannot run cleanup.

Interactive invocation requires terminals on both stdin and stdout. For pipes or scripts, use an explicit subcommand instead. Invalid/ambiguous names fail without changes; prefer the exact ID printed by `list`. Errors exit nonzero.

## Discovery and platform requirements

Browser candidates are installed, registered handlers for **both HTTP and HTTPS**, not a hardcoded list of browser brands. Safari, Chromium/Brave, Firefox/LibreWolf, and other handlers qualify automatically when registered. Unregistered executables are not detected; an OS-registered URL handler is not guaranteed to be a full browser.

- **macOS 12+:** Native NSWorkspace APIs discover handlers. IDs are canonical installed application paths, preserving distinct copies even when their bundle IDs match. Candidates exclude recognized automation/cache locations (including `.cloakbrowser`, Playwright and Puppeteer downloads), temporary roots, dedicated Google Chrome for Testing builds, and Mozilla updater staging bundles by default. This is not a browser-family allowlist or an `/Applications` restriction: normal Chromium, release channels, forks and custom installations elsewhere remain eligible. Unknown automation cache layouts may still appear; genuine installs placed in excluded cache/temporary locations are also excluded. Discovery filtering does not change OS registrations or the defaults reported by `current`. Changing defaults uses supported APIs and may require OS consent; approval cannot be bypassed. Each scheme request waits up to 120 seconds. A timed-out request cannot be cancelled and may still apply later: respond to any pending OS prompt, inspect System Settings and `defbrow current`, and only then retry. Only HTTP/HTTPS URL schemes are changed, not local HTML file associations.
- **Linux:** Discovery uses desktop entries in the session's XDG application paths. Desktop-file IDs identify handlers, including exported package entries when the session exposes them. Install `xdg-utils` using your distribution's package manager and run within your user's graphical desktop session, not with sudo. Desktop/session configuration and overrides such as `BROWSER` can affect actual URL opening. Changes use `xdg-settings`, whose desktop-specific implementation may also update other browser associations; success still requires both URL-scheme readbacks to match.

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

Actual OS consent and graphical desktop behavior require separate, explicitly approved manual verification in a disposable environment; automated tests do not establish those behaviors.
