/**
 * Rainbow CSV grammar shared by the csv/ssv/psv/tsv dialects.
 *
 * Output shape (what `languages/*\/highlights.scm` relies on): each `row`
 * holds its fields as `first` .. `seventh`, cycling by column index modulo 7.
 * Empty fields produce no node but still advance the column.
 *
 * Performance notes (Zed parses the whole file, lexing in WASM):
 * - Every field is a single token, including quoted ones. Lexing a quoted
 *   field char by char produced one tree node per character.
 * - Rows are left-recursive over whole 7-column groups instead of using
 *   `repeat`: tree-sitter marks repetition nodes as fragile, which blocks
 *   reuse of entire rows on incremental reparse. It also keeps the parse
 *   table small (~400 states vs ~1150 for a flat cycle + remainder).
 * - Quoted fields are single-line and may be padded with spaces/tabs (Align
 *   pads after the closing quote). An unclosed quote lexes as a plain field,
 *   so typing `"` only affects the current row instead of turning the rest
 *   of the file into an error.
 */

const NAMES = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh"];

// Escape a single character for use inside a regex character class, where
// only `\`, `]`, `^` and `-` are special.
const classChar = (c) => ("\\]^-".includes(c) ? `\\${c}` : c);

module.exports = function defineGrammar(dialect, sep) {
  const s = classChar(sep);
  // Padding around quoted fields, minus the separator itself (TSV).
  const pad = sep === "\t" ? "[ ]*" : "[ \\t]*";
  const field = ($, k) => optional(alias($.field, $[NAMES[k]]));

  return grammar({
    name: dialect,
    extras: ($) => [],

    rules: {
      csv: ($) => seq(repeat(choice(seq($.row, $._newline), $._newline)), optional($.row)),

      row: ($) => choice($._tail, seq($._groups, optional($._tail))),

      // One reduce per 7 fields; left recursion keeps the parse stack flat.
      _groups: ($) => seq(optional($._groups), $._group),

      _group: ($) => seq(...NAMES.flatMap((_, k) => [field($, k), sep])),

      // Non-empty prefix of a group, without the group's trailing separator.
      _tail: ($) =>
        choice(
          alias($.field, $.first),
          seq(
            field($, 0),
            ...[1, 2, 3, 4, 5, 6].reduceRight(
              (rest, k) => [sep, field($, k), ...(rest.length ? [optional(seq(...rest))] : [])],
              [],
            ),
          ),
        ),

      _newline: ($) => /\r?\n/,

      // Longest match picks the quoted form for `"a,b"`; the unquoted form
      // covers everything else, including stray or unclosed quotes.
      field: ($) =>
        token(
          choice(
            new RegExp(`${pad}"([^"\\r\\n]|"")*"${pad}`),
            new RegExp(`[^${s}\\r\\n]+`),
          ),
        ),
    },
  });
};
