# CSV Columns

A Zed extension for CSV/TSV files: rainbow column highlighting that stays fast
on large files (see "Grammar"), plus column alignment. Highlighting builds on
[zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv); Align/Shrink
follow [vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
and run in a small LSP server. Zed extensions cannot edit buffers directly, so
both alignment modes go through LSP:

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

## Installation

Install **CSV Columns** from Zed's extension page. On first use the
extension downloads the `csv-lsp` language server for your platform from this
repository's GitHub releases and keeps it up to date. Supported: Linux, macOS
and Windows on x86_64 and aarch64.

To use your own build instead, either put `csv-lsp` on your `PATH`, or point
Zed at it in `settings.json`:

```json
{
  "lsp": {
    "csv-lsp": {
      "binary": { "path": "/absolute/path/to/csv-lsp" }
    }
  }
}
```

If you also have the Rainbow CSV extension installed, both claim
`.csv`/`.tsv`. Pick one with Zed's `file_types` setting, e.g.
`"file_types": { "CSV Columns (,)": ["csv"] }`.

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
`target/wasm32-wasip2/debug/csv_columns.wasm`. CI (`.github/workflows/ci.yml`)
runs fmt, clippy, tests, and the wasm build on Ubuntu, Windows, and macOS.

## Development: use as a dev extension

1. Build the server with `cargo build --release -p csv_lsp` and add
   `target/release` to `PATH` — the extension resolves it via
   `worktree.which("csv-lsp")`, ahead of the downloaded release. Avoid
   `target/debug` here: debug builds are several times slower on large files.
2. Zed command palette: `zed: extensions` -> `Install Dev Extension` ->
   pick this directory.
3. Open a `.csv` / `.tsv` file. Highlighting comes from the tree-sitter
   grammars in `grammar/` (see "Grammar" below).
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

- `extension.toml` — grammar pins + `csv-lsp` language server for
  the 4 languages (named "CSV Columns (,)" etc. so they don't clash with the
  Rainbow CSV extension's languages). Language `name`s must match
  `languages/*/config.toml`; `language_ids` fixes the LSP `languageId`
  explicitly instead of relying on display names.
- `grammar/` — the Tree-sitter grammars (`csv`, `ssv`, `psv`, `tsv`), all
  generated from `grammar/common/define-grammar.js`. `src/` holds the
  generated parsers Zed compiles; `test/corpus/` the grammar tests. (Not
  `grammars/`: that is where Zed checks grammars out during dev-install.)
- `languages/` — copied from upstream (`csv,tsv,ssv,psv` + `highlights.scm`),
  names adjusted as above.
- `crates/rainbow_csv_core` — dialect detection, span-tracked quoted/simple
  split (port of `csv_utils.js:split_quoted_str`, extended to tolerate
  whitespace around quoted fields), single-pass `analyze_document`
  (flat per-field widths and UTF-16 columns, warning/needs flags), `align_document`,
  `shrink_document`, range-limited `pads_for_range` (port of
  `rainbow_utils.js` column stats / align / shrink / inlay computation).
  Property tests assert align idempotency, shrink stability, and virtual
  delimiter alignment.
- `crates/csv_lsp` — `csv-lsp` (LSP: codeAction + resolve, inlayHint) +
  `csv-align` CLI. Analysis is lazy (at most once per text version, on the
  first request after edits); edits compute on resolve.
- `src/lib.rs` + root `Cargo.toml` — the extension WASM package itself
  (`zed_extension_api 0.7.0`). Resolves `csv-lsp` from settings, then
  `PATH`, then downloads it from GitHub releases. The root
  doubles as workspace root; `default-members = ["."]` so Zed's plain
  `cargo build --target wasm32-wasip2` only builds this package.
- `samples/` — copied from upstream for manual testing, plus
  `usgs_earthquakes_2025h1.csv` (13.4k rows, 22 columns, many quoted fields;
  U.S. public domain, see `THIRD_PARTY_NOTICES.md`) for large-file testing.
- `docs/setup.md` — per-OS dependency setup.

## Limits

- Single-line records only; unbalanced-quote lines and multiline records are
  refused, never reflowed. Highlighting is single-line as well: a quoted
  field spanning lines is colored line by line.
- No comment-prefix, dynamic separator, or whitespace-dialect support.
- Virtual alignment uses display widths and does not model editor tab stops;
  CSV fields containing tabs may be visually misaligned.
- All columns left-aligned (no decimal-point numeric alignment yet).
- Non-UTF8 files: CLI decodes lossily; LSP path is UTF-8 (JSON).

## Grammar

Highlighting uses our own Tree-sitter grammar, tuned for large files (Zed
parses the whole file, and extension grammars lex in WASM):

- One token per field, quoted or not. Quoted fields used to be lexed one
  character per tree node, which made quote-heavy files ~7x slower to parse.
- Rows are built from 7-column groups via left recursion instead of `repeat`,
  so edits reuse whole rows (repeat nodes are "fragile" in tree-sitter) and
  the parse table is ~3x smaller.
- Whitespace-padded quoted fields (Align's own output) and stray or unclosed
  quotes parse without errors; typing a `"` only affects the current row.

Node names (`row`, `first`..`seventh`) match upstream, so `highlights.scm` is
unchanged. After editing `grammar/common/define-grammar.js`, regenerate and
test every dialect with the tree-sitter CLI (needs Node.js; CI pins 0.25.8):

```sh
cd grammar/csv && npx tree-sitter-cli@0.25.8 generate && npx tree-sitter-cli@0.25.8 test
```

(repeat for `ssv`, `psv`, `tsv`), commit, then point the `commit` of all four
`[grammars.*]` entries in `extension.toml` at that commit. Zed builds grammars
from a Git commit, not from the working tree.

## Releasing

1. Bump `version` in `extension.toml` (and the crate versions, to keep them in
   step), commit, and push.
2. Tag and push, e.g. `git tag v0.1.6 && git push origin v0.1.6`.
3. `.github/workflows/release.yml` builds `csv-lsp` for all six targets and
   publishes the GitHub release only once every build has succeeded. It
   refuses tags that don't match `extension.toml`.
4. After the release is live, open the PR against `zed-industries/extensions`.

To test the builds without releasing, run the workflow manually from the
Actions tab.

## Attribution

This extension builds on two MIT-licensed projects; their license texts are
in `THIRD_PARTY_NOTICES.md`.

- Syntax highlighting queries, language configs (`languages/*/config.toml`,
  `highlights.scm`), and `samples/` (except the USGS file)
  come from [Kalmaegi/zed-rainbow-csv](https://github.com/Kalmaegi/zed-rainbow-csv)
  (MIT, Copyright (c) 2024 Hans). Language display names were changed to
  avoid clashing with that extension in Zed.
- The Align/Shrink/inlay algorithms in `crates/rainbow_csv_core` are a Rust
  port of the logic in [mechatroner/vscode_rainbow_csv](https://github.com/mechatroner/vscode_rainbow_csv)
  (MIT, Copyright (c) 2017 Dmitry Ignatovich) — specifically the quoted-field
  splitter (`rbql_core/rbql-js/csv_utils.js`) and the column-stat / align /
  shrink / inlay-hint computation (`rainbow_utils.js`). No VS Code source
  files are vendored; the logic was reimplemented in Rust.
- The Tree-sitter grammars in `grammar/` were written for this extension,
  following the structure and node naming of
  [coroa/rainbow-csv-tree-sitter](https://github.com/coroa/rainbow-csv-tree-sitter)
  (which upstream `zed-rainbow-csv` uses), so the upstream highlight queries
  work unchanged. The grammar rules themselves were rewritten for
  performance (see "Grammar" above).

Our own code (`crates/`, `grammar/`, `src/`, `extension.toml` arrangement, tasks, docs)
is MIT licensed, see `LICENSE`.
