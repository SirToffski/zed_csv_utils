# Development setup

Machine-agnostic prerequisites. Nothing here is specific to any one computer;
pick the section for your OS. No Node.js required (the language server is pure
Rust); Python is optional (only used for ad-hoc throwaway scripts).

## All platforms

- **Git** — any recent version.
- **Rust (stable) via rustup** — https://rustup.rs. After installing, open a
  *new* shell so `PATH` picks up `cargo`/`rustc`, then verify:

  ```sh
  cargo --version
  rustc --version
  ```

- **wasm32-wasip2 target** — needed to compile the Zed extension itself
  (Zed also auto-installs it on first dev-install if missing):

  ```sh
  rustup target add wasm32-wasip2
  ```

- **wasi-sdk** — needed to compile the Tree-sitter grammars. Zed downloads it
  automatically on first dev-install; you only need to care if you want to
  pin it yourself via the `WASI_SDK_PATH` environment variable.

## Windows

- Install rustup: `winget install Rustlang.Rustup` (or the installer from
  https://rustup.rs).
- A C linker for native binaries (`csv-lsp`, `csv-align`): Visual Studio
  Build Tools or Visual Studio Community with the C++ workload.
- If `npm.ps1`-style errors ever mention PowerShell execution policy, prefer
  `*.cmd`/`*.exe` invocations; nothing in this repo needs policy changes.

## macOS

- Install rustup via the installer from https://rustup.rs
  (`brew install rustup` also works, then `rustup-init`).
- A C linker: Xcode Command Line Tools — `xcode-select --install`.

## Linux

- Install rustup via the installer from https://rustup.rs (distro packages
  also work, e.g. Arch: `pacman -S rustup`, Debian/Ubuntu: `curl` the
  installer; avoid mixing distro `rustc` with rustup toolchains).
- A C linker + standard build tools, e.g. Arch: `pacman -S gcc`,
  Debian/Ubuntu: `apt install build-essential`, Fedora: `dnf install gcc`.

## Building and testing

```sh
cargo test --workspace
cargo build -p csv_lsp
cargo build --target wasm32-wasip2   # same command Zed runs on dev-install
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Trying it in Zed

1. If `csv-lsp` should resolve from `PATH`, ensure the directory containing
   the built `csv-lsp` binary (e.g. `target/debug`) is on `PATH`.
2. Zed command palette → `zed: extensions` → `Install Dev Extension` →
   pick this directory.
3. Open a `.csv`/`.tsv` file. Align/Shrink appear as code actions
   (enable *Code Actions* in the editor toolbar settings to get the button);
   inlay hints give the virtual-align view.
4. To see server debug lines in Zed's language-server logs:
   `CSV_LSP_VERBOSE=1` in the environment Zed is launched from.

## Cleanup

- `cargo clean` removes `target/` (including the multi-MB `csv-lsp` binaries).
- Zed's own caches live outside this repo (`wasi-sdk`, grammar checkouts).
- Removing the toolchain: `rustup self uninstall`, then delete
  `~/.cargo`/`~/.rustup` (`%USERPROFILE%\.cargo`/`.rustup` on Windows).
