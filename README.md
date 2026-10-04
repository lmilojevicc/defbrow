<h1 align="center">defbrow</h1>

<p align="center">A searchable default-browser picker for macOS and Linux, built with Rust and ratatui.</p>

<p align="center">
  <a href="https://github.com/lmilojevicc/defbrow/actions/workflows/ci.yml"><img alt="CI status" src="https://shieldcn.dev/github/ci/lmilojevicc/defbrow.svg?workflow=ci.yml&amp;branch=main&amp;variant=outline" /></a>
  <a href="https://github.com/lmilojevicc/defbrow/graphs/contributors"><img alt="GitHub contributors" src="https://shieldcn.dev/github/contributors/lmilojevicc/defbrow.svg?variant=outline" /></a>
  <a href="https://github.com/lmilojevicc/defbrow/blob/main/LICENSE"><img alt="MIT license" src="https://shieldcn.dev/github/license/lmilojevicc/defbrow.svg?variant=outline" /></a>
</p>

<p align="center">
  <img src="assets/cover.webp" alt="A defbrow window over a blurred sunset desktop: a Search field above a browser list reading Chrome, Helium [current], Zen, Safari and Firefox, with the selected row marked by a rail, and a footer reading: up/down move, Enter set, Esc cancel, Ctrl-U clear." width="760">
</p>

Choose from registered HTTP and HTTPS handlers with a fuzzy-search picker that inherits your terminal colors. Run `defbrow` for the picker, or use `list`, `current`, and `set <id-or-name>`.

## Build and install from source

Requires stable Rust; macOS 12+ also needs Apple's Command Line Tools. On Linux, install `xdg-utils` with your distribution's package manager and use your graphical desktop session.

```sh
cargo install --path . --locked --root "$HOME/.local"
# Add $HOME/.local/bin to PATH.
defbrow --help
```

See [usage, platform requirements, and build instructions](docs/usage.md).
