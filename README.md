# Rainbow CSV with Align (Zed)

Combines [zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv) syntax
highlighting (#1) with [vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
Align/Shrink (#2), via a small LSP server. Zed extensions cannot edit buffers
directly, so both modes go through LSP:

- **Whitespace Align / Shrink** (destructive): `textDocument/formatting`,
  `rangeFormatting`, `codeAction`, `workspace/executeCommand`
  (`csv.align`, `csv.shrink`).
- **Virtual Align** (non-destructive): `textDocument/inlayHint` space pads.

## Build

```powershell
$env:PATH = [System.Environment]::GetEnvironmentVariable('PATH','Machine') + ';' + [System.Environment]::GetEnvironmentVariable('PATH','User')
cargo test -p csv_core
cargo build -p csv_lsp
cargo build --target wasm32-wasip2   # same command Zed runs on dev-install
```

Binaries: `target/debug/csv-lsp.exe`, `target/debug/csv-align.exe`,
`target/wasm32-wasip2/debug/zed_ext.wasm`.

## Use in Zed (dev extension)

1. Add `target/debug` to `PATH` so `csv-lsp` resolves
   (`zed_ext` uses `worktree.which("csv-lsp")`).
2. Zed command palette: `zed: extensions` -> `Install Dev Extension` ->
   pick this directory.
3. Open a `.csv` / `.tsv` file. Highlighting comes from the bundled
   tree-sitter grammars (unchanged from upstream).
4. Align: `editor: format` (whitespace align). Shrink/Align also appear as
   code actions (`editor: toggle code actions`, default `cmd-.`).
   Enable inlay hints in settings for virtual align.

## Use without installing the extension (CLI fallback)

```powershell
cargo run --quiet -p csv_lsp --bin csv-align -- align <file> --in-place
cargo run --quiet -p csv_lsp --bin csv-align -- shrink <file> --in-place
```

Or in Zed: `task: spawn` -> `CSV: align/shrink current file` (see
`.zed/tasks.json`).

## Layout

- `extension.toml` — grammars (upstream pins) + `csv-lsp` language server for
  the 4 Rainbow languages. Language `name`s must match `languages/*/config.toml`.
- `languages/` — copied from upstream (`csv,tsv,ssv,psv` + `highlights.scm`).
- `crates/csv_core` — dialect detection, quoted/simple split (port of
  `csv_utils.js:split_quoted_str`), `align_document`, `shrink_document`,
  `virtual_pads` (port of `rainbow_utils.js:calculate_column_offsets`).
- `crates/csv_lsp` — `csv-lsp` (LSP: formatting, codeAction, inlayHint,
  executeCommand) + `csv-align` CLI. Open docs are cached per version
  (trimmed records + column widths), so scroll-triggered inlay requests
  answer from cache for the visible range only.
- `src/lib.rs` + root `Cargo.toml` — the extension WASM package itself
  (`zed_extension_api 0.6.0`, Zed `0.192.x` compatible) launching `csv-lsp`
  from `PATH`. The root doubles as workspace root; `default-members = ["."]`
  so Zed's plain `cargo build --target wasm32-wasip2` only builds this package.
- `samples/` — copied from upstream for manual testing.
- `_upstream/` — reference clones, git-ignored, not part of the extension.

## MVP limits

- Single-line records only; unbalanced-quote lines are left as-is.
- No comment-prefix, dynamic separator, or whitespace-dialect support.
- All columns left-aligned (no decimal-point numeric alignment yet).
- Non-UTF8 files: CLI decodes lossily; LSP path is UTF-8 (JSON).
- Publishing (prebuilt per-OS binaries via `download_file`) not wired yet;
  dev flow expects `csv-lsp` on `PATH`.

## Cleanup

System deps documented in `DEPENDENCIES.md` (winget uninstall commands).
`cargo clean` removes `target/`. `_upstream/` is git-ignored reference only.

## Attribution

This extension combines work from two MIT-licensed projects:

- Syntax highlighting, language configs (`languages/*/config.toml`,
  `highlights.scm`), grammar pins, and `samples/` (except `vgsales.csv`)
  come from [Kalmaegi/zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv)
  (MIT, Copyright (c) 2024 Hans).
- The Align/Shrink/inlay algorithms in `crates/csv_core` are a Rust port of
  the logic in [mechatroner/vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
  (MIT, Copyright (c) 2017 Dmitry Ignatovich) — specifically the quoted-field
  splitter (`rbql_core/rbql-js/csv_utils.js`) and the column-stat / align /
  shrink / inlay-hint computation (`rainbow_utils.js`). No VS Code source
  files are vendored; the implementation here is a clean-room port.
- At build time Zed fetches the Tree-sitter grammars from
  [coroa/rainbow-csv-tree-sitter](https://github.com/coroa/rainbow-csv-tree-sitter)
  (pinned commit in `extension.toml`). Note: that repo declares no license;
  it is referenced, not distributed, by this extension — same as upstream
  `zed-rainbow-csv` does.

Our own code (`crates/`, `src/`, `extension.toml` arrangement, tasks, docs)
is MIT licensed, see `LICENSE`.
