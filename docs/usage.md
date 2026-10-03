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

Type to search names, IDs, and details using case-insensitive fuzzy subsequence matching: `brv` matches Brave and `lwf` matches LibreWolf. Compact matches near the start rank first; ties are deterministic. Clearing the search restores discovery order. Unicode input and lowercase matching are supported (not locale-specific case folding); Backspace removes one Unicode scalar. Use Ctrl-U to clear, arrows to navigate, and Enter to set. After each attempt, successful or rejected, the same picker stays open with an inline result and refreshed browser entries/current markers. The query and selected browser identity are retained when available, allowing further selections. Escape or Ctrl-C exits without making another change; earlier changes are not undone. Rows show browser names and a `[current]` marker for either current default; when the defaults differ, both browsers are marked without protocol labels. Details appear only to distinguish duplicate names. Use `defbrow current` for per-scheme defaults.

Success requires fresh HTTP and HTTPS defaults to match the selected ID. A failed readback clears current markers rather than displaying stale defaults; failed discovery retains previous entries with an explicit status warning. Inline status is sanitized and clipped to the available width. The explicit `defbrow set` command remains one-shot, prints a human-readable confirmation on verified success, and exits nonzero on failure.

The compact picker has a one-cell outer inset around the Search box, browser list and one-line help footer, with no app banner. The Browsers header is green (ANSI 2) and the Search header is pink (ANSI 13); browser names and query text use the terminal's default foreground. Only the literal `[current]` tag is yellow (ANSI 3), even on the selected row. Borders, help and unselected disambiguating details remain dark gray (ANSI 8); selected details use the default foreground for contrast. Keyboard selection uses a red (ANSI 1) vertical `▌` rail only on the selected line and a neutral dark-gray (ANSI 8) row background, not reversed or recolored text; other backgrounds stay inherited, with no fixed light/dark theme. Inline results occupy one additional row only when present. Set a nonempty `NO_COLOR` to disable all explicit colors, including row shading, while retaining the uncolored rail and bold current tags. There is no theme configuration or OSC color probing.

No matches or an empty handler list cannot be selected. At sizes below 32 columns by 10 rows, resize the terminal or cancel; Enter is disabled while the list is hidden. Resizing redraws the picker. The terminal is restored on return, error, or panic, and before invoking a setter that may show an OS confirmation prompt. After the attempt, raw mode and the picker screen resume; terminal I/O failures can prevent resumption. Forced process termination (such as SIGKILL) cannot run cleanup.

Interactive invocation requires terminals on both stdin and stdout. For pipes or scripts, use an explicit subcommand instead. Invalid/ambiguous names fail without changes; prefer the exact ID printed by `list`. Subcommand and terminal/session errors exit nonzero; interactive switching errors appear in the picker instead of ending the session.

## Discovery and platform requirements

Browser candidates are installed, registered handlers for **both HTTP and HTTPS**, not a hardcoded list of browser brands. Safari, Chromium/Brave, Firefox/LibreWolf, and other handlers qualify automatically when registered. Unregistered executables are not detected; an OS-registered URL handler is not guaranteed to be a full browser.

- **macOS 12+:** Native NSWorkspace APIs discover handlers. IDs are canonical installed application paths, preserving distinct copies even when their bundle IDs match. Candidates exclude recognized automation/cache locations (including `.cloakbrowser`, Playwright and Puppeteer downloads), temporary roots, dedicated Google Chrome for Testing builds, and Mozilla updater staging bundles by default. This is not a browser-family allowlist or an `/Applications` restriction: normal Chromium, release channels, forks and custom installations elsewhere remain eligible. Unknown automation cache layouts may still appear; genuine installs placed in excluded cache/temporary locations are also excluded. Discovery filtering does not change OS registrations or the defaults reported by `current`. Changing defaults uses supported APIs and may require OS consent; approval cannot be bypassed. Fresh defaults already matching the requested canonical identity for both schemes skip setter requests. After a native callback error, fresh matching HTTP and HTTPS identities establish success despite the callback error; mismatched or unreadable defaults remain errors. The failed request is never retried and no subsequent scheme request is issued after its error. This verifies the resulting state, not the underlying cause of an OS error or permanence against later external changes. Each scheme request waits up to 120 seconds. A timed-out request cannot be cancelled and may still apply later: respond to any pending OS prompt, inspect System Settings and `defbrow current`, and only then retry. Only HTTP/HTTPS URL schemes are changed, not local HTML file associations.
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
