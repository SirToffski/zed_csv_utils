//! Align/shrink core for the Rainbow CSV Zed extension.
//!
//! Algorithm port of `vscode_rainbow_csv` (`rbql_core/rbql-js/csv_utils.js`
//! quoted-field splitter, `rainbow_utils.js` column stats / align / shrink /
//! inlay computation). No VS Code sources are vendored; this is a Rust
//! reimplementation. See `README.md` attribution.
//!
//! Data-safety contract (see repo issues on corruption bugs):
//! - The quoted parser tolerates whitespace around quoted fields, so
//!   whitespace-aligned output re-parses identically (align is idempotent).
//! - Lines with unbalanced quotes (incl. RFC 4180 multiline records split
//!   across lines) are flagged, and [`align_document`] / [`shrink_document`]
//!   return the input unchanged for such documents. Callers that must report
//!   can check [`analyze_document`]`.has_warnings`.
//!
//! MVP scope: single-line records. Multiline quoted records are detected and
//! refused rather than mis-aligned.

use unicode_width::UnicodeWidthStr;

/// Delimiter + quoting policy, mirroring `dialect_map` in `extension.js`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `,` quoted
    Csv,
    /// `\t` simple
    Tsv,
    /// `;` quoted
    Ssv,
    /// `|` simple
    Psv,
}

impl Dialect {
    pub fn delimiter(self) -> char {
        match self {
            Dialect::Csv => ',',
            Dialect::Tsv => '\t',
            Dialect::Ssv => ';',
            Dialect::Psv => '|',
        }
    }

    pub fn quoted(self) -> bool {
        matches!(self, Dialect::Csv | Dialect::Ssv)
    }

    /// LSP `languageId` values, set via `[language_servers.*.language_ids]`
    /// in `extension.toml`. Primary signal; independent of display names.
    pub fn from_language_id(id: &str) -> Option<Dialect> {
        match id {
            "csv" => Some(Dialect::Csv),
            "tsv" => Some(Dialect::Tsv),
            "csv-semicolon" => Some(Dialect::Ssv),
            "csv-pipe" => Some(Dialect::Psv),
            _ => None,
        }
    }

    /// Map from Zed `languages/*/config.toml` `name` fields (fallback).
    pub fn from_language_name(name: &str) -> Option<Dialect> {
        match name {
            "Rainbow CSV Align (,)" => Some(Dialect::Csv),
            // The TSV name contains a tab glyph (U+2B72); match by prefix so
            // encoding round-trips can't break detection.
            n if n.starts_with("Rainbow TSV Align") => Some(Dialect::Tsv),
            "Rainbow CSV Align (;)" => Some(Dialect::Ssv),
            "Rainbow CSV Align (|)" => Some(Dialect::Psv),
            _ => None,
        }
    }

    /// Map from file extension, for the CLI fallback and unknown language ids.
    pub fn from_extension(ext: &str) -> Option<Dialect> {
        match ext.to_ascii_lowercase().as_str() {
            "csv" => Some(Dialect::Csv),
            "tsv" | "tab" => Some(Dialect::Tsv),
            _ => None,
        }
    }
}

/// One parsed field: trimmed value plus its raw byte span in the line.
/// The span is what anchors inlay hints and UTF-16 columns to the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedField {
    /// Field text with surrounding whitespace removed (width basis).
    pub value: String,
    /// Byte offset of the raw field start (incl. surrounding whitespace).
    pub start: usize,
    /// Byte offset of the raw field end.
    pub end: usize,
}

/// Split one line into trimmed values. Returns `(fields, warning)`.
/// `warning=true` means unbalanced quotes; the line must not be reflowed.
pub fn split_line(line: &str, dialect: Dialect) -> (Vec<String>, bool) {
    let (fields, warning) = split_line_spans(line, dialect);
    (fields.into_iter().map(|f| f.value).collect(), warning)
}

/// Split one line, tracking each field's raw span.
pub fn split_line_spans(line: &str, dialect: Dialect) -> (Vec<SpannedField>, bool) {
    if !dialect.quoted() {
        return split_simple_spans(line, dialect.delimiter());
    }
    split_quoted_spans(line, dialect.delimiter())
}

fn spanned(value: String, start: usize, end: usize) -> SpannedField {
    SpannedField { value, start, end }
}

/// Literal split that records byte offsets. Values are trimmed; spans stay raw.
fn split_simple_spans(line: &str, delim: char) -> (Vec<SpannedField>, bool) {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (idx, ch) in line.char_indices() {
        if ch == delim {
            out.push(spanned(line[start..idx].trim().to_string(), start, idx));
            start = idx + ch.len_utf8();
        }
    }
    out.push(spanned(line[start..].trim().to_string(), start, line.len()));
    (out, false)
}

fn is_outer_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Port of `split_quoted_str` in `csv_utils.js`, with one deliberate
/// extension: whitespace around a quoted field is accepted (matching VS Code's
/// `field_rgx_external_whitespaces`). Whitespace-align pads after the closing
/// quote, so without this the padded output would mis-parse as unbalanced on
/// the next pass and corrupt quoted commas.
fn split_quoted_spans(src: &str, dlm: char) -> (Vec<SpannedField>, bool) {
    if !src.contains('"') {
        return split_simple_spans(src, dlm);
    }
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut warning = false;
    let mut i = 0usize;
    while i < n {
        // Tolerate `   "a,b"   ,c`.
        let mut j = i;
        while j < n && is_outer_ws(bytes[j]) {
            j += 1;
        }
        if j < n && bytes[j] == b'"' {
            if let Some((_field, close_end)) = parse_quoted_at(src, j, dlm) {
                let mut k = close_end;
                while k < n && is_outer_ws(bytes[k]) {
                    k += 1;
                }
                if k == n || bytes[k] == dlm as u8 {
                    out.push(spanned(src[i..k].trim().to_string(), i, k));
                    i = k;
                    if i < n && bytes[i] == dlm as u8 {
                        i += dlm.len_utf8();
                        if i == n {
                            out.push(spanned(String::new(), n, n));
                        }
                    }
                    continue;
                }
            }
            // Unbalanced quote or garbage after it: warn and consume
            // literally up to the next delimiter.
            warning = true;
            let rest = &src[i..];
            if let Some(pos) = rest.find(dlm) {
                let end = i + pos;
                out.push(spanned(rest[..pos].to_string(), i, end));
                i = end + dlm.len_utf8();
                if i == n {
                    out.push(spanned(String::new(), n, n));
                }
            } else {
                out.push(spanned(rest.to_string(), i, n));
                i = n;
            }
            continue;
        }
        let rest = &src[i..];
        if let Some(pos) = rest.find(dlm) {
            let end = i + pos;
            let text = &src[i..end];
            warning = warning || text.contains('"');
            out.push(spanned(text.trim().to_string(), i, end));
            i = end + dlm.len_utf8();
            if i == n {
                out.push(spanned(String::new(), n, n));
            }
        } else {
            warning = warning || rest.contains('"');
            out.push(spanned(rest.trim().to_string(), i, n));
            i = n;
        }
    }
    if src.is_empty() {
        out.push(spanned(String::new(), 0, 0));
    }
    (out, warning)
}

/// Parse `"..."` starting at byte index `start` (which must be `"`).
/// Returns `(field_text_with_quotes, next_byte_index_past_closing_quote)`.
/// The closing quote may be followed by spaces/tabs (whitespace-align pads
/// there) as long as only the delimiter or end of line comes after; anything
/// else (e.g. `"a"x`) is rejected as unbalanced.
fn parse_quoted_at(src: &str, start: usize, dlm: char) -> Option<(String, usize)> {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut i = start + 1;
    while i < n {
        if bytes[i] == b'"' {
            if i + 1 < n && bytes[i + 1] == b'"' {
                i += 2; // escaped quote
                continue;
            }
            let end = i + 1;
            let mut k = end;
            while k < n && is_outer_ws(bytes[k]) {
                k += 1;
            }
            if k == n || src[k..].starts_with(dlm) {
                return Some((src[start..end].to_string(), end));
            }
            return None; // garbage after quote -> unbalanced for our purposes
        }
        // Advance by one char (quotes/newlines can't appear inside single-line input
        // except as data, but be correct for multibyte).
        let ch_len = src[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        i += ch_len;
    }
    None
}

pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Length in UTF-16 code units (LSP `character` offsets are UTF-16).
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Whole-document parse result. Computed once per document version and shared
/// by align, shrink, and inlay-hint serving.
#[derive(Debug, Clone)]
pub struct DocAnalysis {
    /// Trimmed field values per line.
    pub records: Vec<Vec<String>>,
    /// UTF-16 end offset of each raw field in its (CR-stripped) line.
    pub end_cols: Vec<Vec<u32>>,
    /// Max display width per column.
    pub widths: Vec<usize>,
    /// True if any line has unbalanced quotes (incl. multiline records).
    /// Such documents must not be reflowed.
    pub has_warnings: bool,
    /// True if [`align_document`] would change the text.
    pub needs_align: bool,
    /// True if any field has surrounding whitespace.
    pub needs_shrink: bool,
}

/// Parse + measure a document in a single pass.
pub fn analyze_document(text: &str, dialect: Dialect) -> DocAnalysis {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut records = Vec::new();
    let mut end_cols = Vec::new();
    let mut has_warnings = false;
    let mut needs_shrink = false;
    for (idx, line) in lines.iter().enumerate() {
        let stripped = line.strip_suffix('\r').unwrap_or(line);
        // Skip the final artifact of trailing newline.
        if stripped.is_empty() && idx + 1 == lines.len() && text.ends_with('\n') {
            continue;
        }
        let (fields, warning) = split_line_spans(stripped, dialect);
        has_warnings = has_warnings || warning;
        let mut record = Vec::with_capacity(fields.len());
        let mut cols = Vec::with_capacity(fields.len());
        for f in &fields {
            needs_shrink = needs_shrink || stripped[f.start..f.end] != f.value;
            record.push(f.value.clone());
            cols.push(utf16_len(&stripped[..f.end]) as u32);
        }
        records.push(record);
        end_cols.push(cols);
    }
    let mut widths: Vec<usize> = Vec::new();
    for rec in &records {
        for (i, f) in rec.iter().enumerate() {
            if widths.len() <= i {
                widths.push(0);
            }
            widths[i] = widths[i].max(display_width(f));
        }
    }
    let mut needs_align = needs_shrink;
    if !needs_align {
        'outer: for rec in &records {
            for (i, f) in rec.iter().enumerate() {
                let is_last = i + 1 == rec.len();
                if !is_last && display_width(f) < widths[i] {
                    needs_align = true;
                    break 'outer;
                }
            }
        }
    }
    DocAnalysis {
        records,
        end_cols,
        widths,
        has_warnings,
        needs_align,
        needs_shrink,
    }
}

/// Align whole document with spaces (whitespace-align). Returns new text.
///
/// Returns the input unchanged when any line has unbalanced quotes: reflowing
/// such input corrupts data (e.g. splits quoted commas on a second pass).
/// Idempotent: `align(align(x)) == align(x)` for warning-free input.
pub fn align_document(text: &str, dialect: Dialect) -> String {
    if text.is_empty() {
        return String::new();
    }
    let analysis = analyze_document(text, dialect);
    if analysis.has_warnings {
        return text.to_string();
    }
    let trailing_nl = text.ends_with('\n');
    let mut raw_lines: Vec<&str> = text.split('\n').collect();
    if trailing_nl && raw_lines.last() == Some(&"") {
        raw_lines.pop();
    }
    debug_assert_eq!(raw_lines.len(), analysis.records.len());
    let endings: Vec<&str> = raw_lines
        .iter()
        .map(|l| if l.ends_with('\r') { "\r\n" } else { "\n" })
        .collect();
    let delim = dialect.delimiter().to_string();
    let mut out_lines = Vec::with_capacity(analysis.records.len());
    for rec in &analysis.records {
        let mut cells = Vec::with_capacity(rec.len());
        for (i, f) in rec.iter().enumerate() {
            let is_last = i + 1 == rec.len();
            if is_last {
                cells.push(f.clone()); // no trailing pad on last column
            } else {
                let pad = analysis.widths[i].saturating_sub(display_width(f));
                let mut cell = String::with_capacity(f.len() + pad);
                cell.push_str(f);
                cell.push_str(&" ".repeat(pad));
                cells.push(cell);
            }
        }
        out_lines.push(cells.join(&delim));
    }
    let mut out = String::new();
    for (i, l) in out_lines.iter().enumerate() {
        out.push_str(l);
        if i + 1 < out_lines.len() || trailing_nl {
            out.push_str(endings.get(i).copied().unwrap_or("\n"));
        }
    }
    out
}

/// Shrink: trim leading/trailing whitespace of every field.
/// Returns `(new_text, changed)`. Returns `(input, false)` unchanged when any
/// line has unbalanced quotes.
pub fn shrink_document(text: &str, dialect: Dialect) -> (String, bool) {
    if text.is_empty() {
        return (String::new(), false);
    }
    let analysis = analyze_document(text, dialect);
    if analysis.has_warnings {
        return (text.to_string(), false);
    }
    let trailing_nl = text.ends_with('\n');
    let mut raw_lines: Vec<&str> = text.split('\n').collect();
    if trailing_nl && raw_lines.last() == Some(&"") {
        raw_lines.pop();
    }
    let endings: Vec<&str> = raw_lines
        .iter()
        .map(|l| if l.ends_with('\r') { "\r\n" } else { "\n" })
        .collect();
    let delim = dialect.delimiter().to_string();
    let mut out_lines: Vec<String> = analysis
        .records
        .iter()
        .map(|rec| rec.join(&delim))
        .collect();
    for (line, ending) in out_lines.iter_mut().zip(endings.iter()) {
        if *ending == "\r\n" {
            line.push('\r');
        }
    }
    let mut out = out_lines.join("\n");
    if trailing_nl {
        out.push('\n');
    }
    let changed = out != text;
    (out, changed)
}

/// One virtual-align pad: insert `spaces` at `line:col` (end of a raw field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualPad {
    pub line: u32,
    pub col: u32,
    pub spaces: usize,
}

/// Virtual-align pads for lines in `[start_line, end_line]` from a cached
/// analysis. Columns are raw-field ends in UTF-16 units, so hints land after
/// existing whitespace instead of doubling it.
pub fn pads_for_range(analysis: &DocAnalysis, start_line: u32, end_line: u32) -> Vec<VirtualPad> {
    let mut pads = Vec::new();
    for (lnum, rec) in analysis.records.iter().enumerate() {
        let lnum = lnum as u32;
        if lnum < start_line || lnum > end_line {
            continue;
        }
        for (i, f) in rec.iter().enumerate() {
            let is_last = i + 1 == rec.len();
            if !is_last {
                let pad = analysis.widths[i].saturating_sub(display_width(f));
                if pad > 0 {
                    pads.push(VirtualPad {
                        line: lnum,
                        col: analysis.end_cols[lnum as usize][i],
                        spaces: pad,
                    });
                }
            }
        }
    }
    pads
}

/// All virtual-align pads (convenience wrapper; servers should prefer the
/// cached [`analyze_document`] + [`pads_for_range`]).
pub fn virtual_pads(text: &str, dialect: Dialect) -> Vec<VirtualPad> {
    let analysis = analyze_document(text, dialect);
    pads_for_range(&analysis, 0, u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_simple() {
        let (f, w) = split_line("a\tb\tc", Dialect::Tsv);
        assert_eq!(f, vec!["a", "b", "c"]);
        assert!(!w);
    }

    #[test]
    fn split_quoted_with_comma() {
        let (f, w) = split_line("1,\"a,b\",c", Dialect::Csv);
        assert_eq!(f, vec!["1", "\"a,b\"", "c"]);
        assert!(!w);
    }

    #[test]
    fn split_quoted_escaped() {
        let (f, w) = split_line("2,\"1.7 Cubic Foot \"\"Cube\"\" X\",y", Dialect::Csv);
        assert_eq!(f.len(), 3);
        assert!(!w);
    }

    #[test]
    fn split_unbalanced_warns() {
        let (_f, w) = split_line("a,\"b,c", Dialect::Csv);
        assert!(w);
    }

    #[test]
    fn split_tolerates_whitespace_around_quotes() {
        // Whitespace-align pads after the closing quote; that output must
        // re-parse as quoted (VS Code `field_rgx_external_whitespaces` parity).
        let (f, w) = split_line("1,  \"Smith, J\"   ,3", Dialect::Csv);
        assert_eq!(f, vec!["1", "\"Smith, J\"", "3"]);
        assert!(!w);
    }

    #[test]
    fn split_rejects_garbage_after_quote() {
        let (_f, w) = split_line("1,\"a\"x,3", Dialect::Csv);
        assert!(w);
    }

    #[test]
    fn align_basic_csv() {
        let input = "a,bb,ccc\ndddd,e,f\n";
        let out = align_document(input, Dialect::Csv);
        assert_eq!(out, "a   ,bb,ccc\ndddd,e ,f\n");
    }

    #[test]
    fn align_trims_and_pads() {
        let input = "  a , bb \nccc, d\n";
        let out = align_document(input, Dialect::Csv);
        assert_eq!(out, "a  ,bb\nccc,d\n");
    }

    #[test]
    fn align_quoted_padding_is_idempotent() {
        // Regression: padded quoted fields must survive a second align.
        let input = "Rank,Name,Score\n1,\"Smith, J\",3\n100,Alexander Hamilton-Smith,5\n22,Li,30\n";
        let once = align_document(input, Dialect::Csv);
        assert!(
            once.contains("\"Smith, J\""),
            "quoted comma must survive: {once}"
        );
        assert_eq!(align_document(&once, Dialect::Csv), once);
    }

    #[test]
    fn align_is_idempotent_on_sample() {
        let sample = include_str!("../../../samples/sample_csv_file.csv");
        let once = align_document(sample, Dialect::Csv);
        assert_eq!(align_document(&once, Dialect::Csv), once);
    }

    #[test]
    fn shrink_after_align_matches_shrink() {
        let inputs = [
            "  a , bb \nccc, d\n",
            "Rank,Name,Score\n1,\"Smith, J\",3\n100,Alexander Hamilton-Smith,5\n",
            include_str!("../../../samples/sample_csv_file.csv"),
        ];
        for input in inputs {
            let aligned = align_document(input, Dialect::Csv);
            assert_eq!(
                shrink_document(&aligned, Dialect::Csv).0,
                shrink_document(input, Dialect::Csv).0
            );
        }
    }

    #[test]
    fn align_refuses_unbalanced() {
        let bad = "a,b\n1,\"oops,2\n";
        assert!(analyze_document(bad, Dialect::Csv).has_warnings);
        assert_eq!(align_document(bad, Dialect::Csv), bad);
        assert_eq!(shrink_document(bad, Dialect::Csv), (bad.to_string(), false));
    }

    #[test]
    fn align_refuses_multiline_record() {
        let multiline = "\"a\nb\",c\nd,e\n";
        assert!(analyze_document(multiline, Dialect::Csv).has_warnings);
        assert_eq!(align_document(multiline, Dialect::Csv), multiline);
    }

    #[test]
    fn shrink_trims() {
        let (out, changed) = shrink_document("  a , bb \nccc,d\n", Dialect::Csv);
        assert_eq!(out, "a,bb\nccc,d\n");
        assert!(changed);
    }

    #[test]
    fn shrink_noop() {
        let (_out, changed) = shrink_document("a,b\nc,d\n", Dialect::Csv);
        assert!(!changed);
    }

    #[test]
    fn virtual_pads_basic() {
        let pads = virtual_pads("a,bb\ncccc,d\n", Dialect::Csv);
        // col0 widths: max(1,4)=4 -> line0 needs 3 pads after col1; col1 is last -> none.
        assert!(pads.contains(&VirtualPad {
            line: 0,
            col: 1,
            spaces: 3
        }));
    }

    #[test]
    fn pads_for_range_subset() {
        let analysis = analyze_document("a,bb\ncccc,d\n", Dialect::Csv);
        assert_eq!(analysis.widths, vec![4, 2]);
        let all = pads_for_range(&analysis, 0, u32::MAX);
        assert_eq!(all, virtual_pads("a,bb\ncccc,d\n", Dialect::Csv));
        // Line 0 needs padding ("a" -> width 4); line 1 is already widest.
        let line0 = pads_for_range(&analysis, 0, 0);
        assert_eq!(
            line0,
            vec![VirtualPad {
                line: 0,
                col: 1,
                spaces: 3
            }]
        );
        assert!(pads_for_range(&analysis, 1, 1).is_empty());
    }

    #[test]
    fn pads_anchor_after_existing_whitespace() {
        // `a  ,b`: hint must go after the raw field end (col 3), not after `a`.
        let analysis = analyze_document("a  ,b\ncccc,d\n", Dialect::Csv);
        let pads = pads_for_range(&analysis, 0, 0);
        assert_eq!(
            pads,
            vec![VirtualPad {
                line: 0,
                col: 3,
                spaces: 3
            }]
        );
    }

    #[test]
    fn utf16_columns() {
        assert_eq!(utf16_len("a"), 1);
        assert_eq!(utf16_len("é"), 1);
        assert_eq!(utf16_len("😀"), 2);
        // Hint after an emoji field accounts for the surrogate pair.
        let analysis = analyze_document("😀,bb\ncccc,d\n", Dialect::Csv);
        let pads = pads_for_range(&analysis, 0, 0);
        assert_eq!(
            pads,
            vec![VirtualPad {
                line: 0,
                col: 2,
                spaces: 2
            }]
        );
    }

    #[test]
    fn upstream_sample_parses_without_warnings() {
        let sample = include_str!("../../../samples/sample_csv_file.csv");
        assert!(!analyze_document(sample, Dialect::Csv).has_warnings);
    }

    #[cfg(test)]
    mod proptests {
        use super::*;
        use proptest::prelude::*;

        fn csv_field() -> impl Strategy<Value = String> {
            prop::string::string_regex("[A-Za-z0-9 ,.\"'éü]{0,12}").unwrap()
        }

        fn tsv_field() -> impl Strategy<Value = String> {
            // Tabs would split TSV fields, so well-formed input excludes them.
            prop::string::string_regex("[A-Za-z0-9 ,.\"'éü]{0,12}").unwrap()
        }

        fn render_quoted(fields: &[String], delim: char) -> String {
            fields
                .iter()
                .map(|f| {
                    if f.contains(delim) || f.contains('"') {
                        format!("\"{}\"", f.replace('"', "\"\""))
                    } else {
                        f.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(&delim.to_string())
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(128))]
            #[test]
            fn align_idempotent_and_shrink_stable_csv(
                rows in prop::collection::vec(prop::collection::vec(csv_field(), 1..6), 0..20)
            ) {
                let text = rows.iter().map(|r| render_quoted(r, ',')).collect::<Vec<_>>().join("\n") + "\n";
                prop_assert!(!analyze_document(&text, Dialect::Csv).has_warnings, "generated well-formed csv warned");
                let once = align_document(&text, Dialect::Csv);
                let twice = align_document(&once, Dialect::Csv);
                prop_assert_eq!(&twice, &once);
                let shrink_once = shrink_document(&once, Dialect::Csv).0;
                let shrink_orig = shrink_document(&text, Dialect::Csv).0;
                prop_assert_eq!(shrink_once, shrink_orig);
            }

            #[test]
            fn align_idempotent_and_shrink_stable_tsv(
                rows in prop::collection::vec(prop::collection::vec(tsv_field(), 1..6), 0..20)
            ) {
                let text = rows.iter().map(|r| r.join("\t")).collect::<Vec<_>>().join("\n") + "\n";
                prop_assert!(!analyze_document(&text, Dialect::Tsv).has_warnings, "generated well-formed tsv warned");
                let once = align_document(&text, Dialect::Tsv);
                let twice = align_document(&once, Dialect::Tsv);
                prop_assert_eq!(&twice, &once);
                let shrink_once = shrink_document(&once, Dialect::Tsv).0;
                let shrink_orig = shrink_document(&text, Dialect::Tsv).0;
                prop_assert_eq!(shrink_once, shrink_orig);
            }
        }
    }
}
