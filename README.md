# Rainbow CSV with Align (Zed)

Combines [zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv) syntax
highlighting with [vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
Align/Shrink, via a small LSP server. Zed extensions cannot edit buffers
directly, so both modes go through LSP:

- **Whitespace Align / Shrink** (destructive): explicit `source` code actions
  ("Align CSV columns (spaces)", "Shrink CSV columns (trim spaces)"). Edits
  are computed on demand via `codeAction/resolve`. Align is deliberately *not*
  a formatter: padding changes cell values for downstream consumers, so it
  never runs on save or on `editor: format` — same as VS Code, where Align is
  an explicit command.
- **Virtual Align** (non-destructive): `textDocument/inlayHint` space pads,
  served from a per-version cache for the visible range only.

Documents with unbalanced quotes (including multiline quoted records) are
never reflowed: Align/Shrink return no edits for them.

## Build

Prerequisites per OS: see `docs/setup.md`. Then:

```sh
cargo test --workspace
cargo build -p csv_lsp
cargo build --target wasm32-wasip2   # same command Zed runs on dev-install
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Binaries: `target/debug/csv-lsp(.exe)`, `target/debug/csv-align(.exe)`,
`target/wasm32-wasip2/debug/zed_csv_align.wasm`. CI (`.github/workflows/ci.yml`)
runs fmt, clippy, tests, and the wasm build on Ubuntu, Windows, and macOS.

## Use in Zed (dev extension)

1. Add the directory containing the built `csv-lsp` binary (e.g.
   `target/debug`) to `PATH` — the extension resolves it via
   `worktree.which("csv-lsp")`.
2. Zed command palette: `zed: extensions` -> `Install Dev Extension` ->
   pick this directory.
3. Open a `.csv` / `.tsv` file. Highlighting comes from the bundled
   tree-sitter grammars (unchanged from upstream).
4. Align/Shrink appear as code actions (`editor: toggle code actions`) and —
   once *Code Actions* is enabled in the editor toolbar settings — as a
   toolbar button. Enable inlay hints in settings for the virtual-align view.

## Use without installing the extension (CLI fallback)

```sh
cargo run --quiet -p csv_lsp --bin csv-align -- align <file> --in-place
cargo run --quiet -p csv_lsp --bin csv-align -- shrink <file> --in-place
```

Or in Zed: `task: spawn` -> `CSV: align/shrink current file` (see
`.zed/tasks.json`). The CLI refuses files with unbalanced quotes instead of
risking corruption.

## Troubleshooting

- Rebuilding `csv-lsp` while Zed runs fails with "Access is denied": Zed
  holds the binary open. Close the CSV tabs (or Zed), rebuild, reopen.
- Code-action menu empty: check the per-server logs (command palette, search
  "language server", open the `csv-lsp` logs). With `CSV_LSP_VERBOSE=1` in
  Zed's environment the server logs every code-action/resolve call; slow
  inlay requests are always logged.
- No actions offered on a file: by design when the file has unbalanced
  quotes or multiline records.

## Layout

- `extension.toml` — grammars (upstream pins) + `csv-lsp` language server for
  the 4 Rainbow languages (renamed with an `Align` suffix so they don't clash
  with the existing Rainbow CSV extension). Language `name`s must match
  `languages/*/config.toml`; `language_ids` fixes the LSP `languageId`
  explicitly instead of relying on display names.
- `languages/` — copied from upstream (`csv,tsv,ssv,psv` + `highlights.scm`),
  names adjusted as above.
- `crates/rainbow_csv_core` — dialect detection, span-tracked quoted/simple
  split (port of `csv_utils.js:split_quoted_str`, extended to tolerate
  whitespace around quoted fields), single-pass `analyze_document`
  (records, widths, warning/needs flags), `align_document`,
  `shrink_document`, range-limited `pads_for_range` (port of
  `rainbow_utils.js` column stats / align / shrink / inlay computation).
  Property tests assert align idempotency and shrink stability.
- `crates/csv_lsp` — `csv-lsp` (LSP: codeAction + resolve, inlayHint) +
  `csv-align` CLI. Open docs are cached per sync; list-time action gating is
  O(1), edits compute on resolve.
- `src/lib.rs` + root `Cargo.toml` — the extension WASM package itself
  (`zed_extension_api 0.7.0`) launching `csv-lsp` from `PATH`. The root
  doubles as workspace root; `default-members = ["."]` so Zed's plain
  `cargo build --target wasm32-wasip2` only builds this package.
- `samples/` — copied from upstream for manual testing, plus `vgsales.csv`
  (public Kaggle data, 16.5k rows).
- `docs/setup.md` — per-OS dependency setup.

## Limits

- Single-line records only; unbalanced-quote lines and multiline records are
  refused, never reflowed.
- No comment-prefix, dynamic separator, or whitespace-dialect support.
- All columns left-aligned (no decimal-point numeric alignment yet).
- Non-UTF8 files: CLI decodes lossily; LSP path is UTF-8 (JSON).
- Publishing (prebuilt per-OS binaries via `download_file`) not wired yet;
  dev flow expects `csv-lsp` on `PATH`.

## Attribution

This extension combines work from two MIT-licensed projects:

- Syntax highlighting, language configs (`languages/*/config.toml`,
  `highlights.scm`), grammar pins, and `samples/` (except `vgsales.csv`)
  come from [Kalmaegi/zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv)
  (MIT, Copyright (c) 2024 Hans). Language display names were changed to
  avoid clashing with that extension in Zed.
- The Align/Shrink/inlay algorithms in `crates/rainbow_csv_core` are a Rust
  port of the logic in [mechatroner/vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
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
