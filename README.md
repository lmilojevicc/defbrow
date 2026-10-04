# defbrow

**A — framing as shot** (WebP, 45 KB, down from a 1.2 MB PNG)

<img src="assets/cover.webp" alt="The defbrow picker: a Search field above a browser list reading Chrome, Helium [current], Zen, Safari and Firefox, with the selected row marked by a rail, and a footer reading: up/down move, Enter set, Esc cancel, Ctrl-U clear." width="760">

**B — empty list area squeezed** (WebP, 29 KB) — same layout, but the terminal's vertical gradient is compressed ~15x where nothing is drawn, which shows as a soft shadow band above the footer.

<img src="assets/cover-trimmed.webp" alt="The same picker with the empty space below the browser list compressed so the UI fills more of the frame." width="760">

A small Rust CLI for choosing the default browser on macOS and Linux, with a searchable ratatui picker that inherits your terminal colors.

Run `defbrow` for the picker, or use `list`, `current`, and `set <id-or-name>`.

See [usage, requirements, and build instructions](docs/usage.md).
