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

/// Allocation-free field span: raw `start..end` plus the trimmed value's
/// `tstart..tend`, all byte offsets into the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawField {
    start: usize,
    end: usize,
    tstart: usize,
    tend: usize,
}

impl RawField {
    /// Span whose value is the whitespace-trimmed raw text.
    fn trimmed(line: &str, start: usize, end: usize) -> Self {
        let raw = &line[start..end];
        let lead = raw.len() - raw.trim_start().len();
        let tend = start + raw.trim_end().len();
        RawField {
            start,
            end,
            tstart: (start + lead).min(tend),
            tend,
        }
    }

    /// Span whose value is the raw text as-is.
    fn untrimmed(start: usize, end: usize) -> Self {
        RawField {
            start,
            end,
            tstart: start,
            tend: end,
        }
    }

    fn is_trimmed(&self) -> bool {
        self.tstart != self.start || self.tend != self.end
    }
}

/// Split one line into trimmed values. Returns `(fields, warning)`.
/// `warning=true` means unbalanced quotes; the line must not be reflowed.
pub fn split_line(line: &str, dialect: Dialect) -> (Vec<String>, bool) {
    let (fields, warning) = split_line_spans(line, dialect);
    (fields.into_iter().map(|f| f.value).collect(), warning)
}

/// Split one line, tracking each field's raw span.
pub fn split_line_spans(line: &str, dialect: Dialect) -> (Vec<SpannedField>, bool) {
    let mut raw = Vec::new();
    let warning = split_raw(line, dialect, &mut raw);
    let fields = raw
        .into_iter()
        .map(|f| SpannedField {
            value: line[f.tstart..f.tend].to_string(),
            start: f.start,
            end: f.end,
        })
        .collect();
    (fields, warning)
}

/// Split `line` into `out` (cleared first). Returns the unbalanced-quote flag.
fn split_raw(line: &str, dialect: Dialect, out: &mut Vec<RawField>) -> bool {
    out.clear();
    // Every delimiter is ASCII, so byte search can't hit inside a multibyte char.
    let dlm = dialect.delimiter() as u8;
    if !dialect.quoted() || !line.as_bytes().contains(&b'"') {
        split_simple(line, dlm, out);
        return false;
    }
    split_quoted(line, dlm, out)
}

/// Literal split. Values are trimmed; spans stay raw.
fn split_simple(line: &str, dlm: u8, out: &mut Vec<RawField>) {
    let mut start = 0usize;
    for (idx, &b) in line.as_bytes().iter().enumerate() {
        if b == dlm {
            out.push(RawField::trimmed(line, start, idx));
            start = idx + 1;
        }
    }
    out.push(RawField::trimmed(line, start, line.len()));
}

fn is_outer_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn find_byte(bytes: &[u8], from: usize, b: u8) -> Option<usize> {
    bytes[from..].iter().position(|&x| x == b).map(|p| from + p)
}

/// Port of `split_quoted_str` in `csv_utils.js`, with one deliberate
/// extension: whitespace around a quoted field is accepted (matching VS Code's
/// `field_rgx_external_whitespaces`). Whitespace-align pads after the closing
/// quote, so without this the padded output would mis-parse as unbalanced on
/// the next pass and corrupt quoted commas.
fn split_quoted(src: &str, dlm: u8, out: &mut Vec<RawField>) -> bool {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut warning = false;
    let mut i = 0usize;
    while i < n {
        // Tolerate `   "a,b"   ,c`.
        let mut j = i;
        while j < n && is_outer_ws(bytes[j]) {
            j += 1;
        }
        if j < n && bytes[j] == b'"' {
            if let Some(close_end) = parse_quoted_at(bytes, j, dlm) {
                let mut k = close_end;
                while k < n && is_outer_ws(bytes[k]) {
                    k += 1;
                }
                // `parse_quoted_at` guarantees `k == n || bytes[k] == dlm`.
                out.push(RawField::trimmed(src, i, k));
                i = k;
                if i < n {
                    i += 1;
                    if i == n {
                        out.push(RawField::untrimmed(n, n));
                    }
                }
                continue;
            }
            // Unbalanced quote or garbage after it: warn and consume
            // literally up to the next delimiter.
            warning = true;
            match find_byte(bytes, i, dlm) {
                Some(end) => {
                    out.push(RawField::untrimmed(i, end));
                    i = end + 1;
                    if i == n {
                        out.push(RawField::untrimmed(n, n));
                    }
                }
                None => {
                    out.push(RawField::untrimmed(i, n));
                    i = n;
                }
            }
            continue;
        }
        let end = find_byte(bytes, i, dlm).unwrap_or(n);
        warning = warning || bytes[i..end].contains(&b'"');
        out.push(RawField::trimmed(src, i, end));
        i = end;
        if i < n {
            i += 1;
            if i == n {
                out.push(RawField::untrimmed(n, n));
            }
        }
    }
    if src.is_empty() {
        out.push(RawField::untrimmed(0, 0));
    }
    warning
}

/// Parse `"..."` starting at byte index `start` (which must be `"`).
/// Returns the byte index just past the closing quote. The closing quote may
/// be followed by spaces/tabs (whitespace-align pads there) as long as only
/// the delimiter or end of line comes after; anything else (e.g. `"a"x`) is
/// rejected as unbalanced.
fn parse_quoted_at(bytes: &[u8], start: usize, dlm: u8) -> Option<usize> {
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
            if k == n || bytes[k] == dlm {
                return Some(end);
            }
            return None; // garbage after quote -> unbalanced for our purposes
        }
        // `"` is ASCII, so stepping bytewise through multibyte chars is safe.
        i += 1;
    }
    None
}

/// Cell width used for alignment. This does not model editor tab stops.
pub fn display_width(s: &str) -> usize {
    // Printable ASCII is one column per byte; skip the Unicode tables.
    if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        s.len()
    } else {
        UnicodeWidthStr::width(s)
    }
}

/// Length in UTF-16 code units (LSP `character` offsets are UTF-16).
pub fn utf16_len(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.encode_utf16().count()
    }
}

/// Per-field measurements cached by [`analyze_document`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldInfo {
    /// UTF-16 end offset of the raw field in its (CR-stripped) line.
    pub end_col: u32,
    /// Display width of the raw field, including surrounding whitespace.
    pub raw_width: u32,
    /// Display width of the trimmed field value.
    pub width: u32,
}

/// Whole-document parse result. Computed once per document version and shared
/// by action gating and inlay-hint serving. Stores only measurements (no
/// field text), in one flat array, so analysis stays cheap on large files.
#[derive(Debug, Clone)]
pub struct DocAnalysis {
    /// Every field of every line, in document order.
    fields: Vec<FieldInfo>,
    /// Line `i`'s fields are `fields[line_ends[i - 1]..line_ends[i]]`.
    line_ends: Vec<usize>,
    /// Max raw display width per column, used as the virtual-align target.
    pub raw_width_targets: Vec<usize>,
    /// Max display width of trimmed field values per column, used by Align.
    pub widths: Vec<usize>,
    /// True if any line has unbalanced quotes (incl. multiline records).
    /// Such documents must not be reflowed.
    pub has_warnings: bool,
    /// True if [`align_document`] would change the text.
    pub needs_align: bool,
    /// True if any field has surrounding whitespace.
    pub needs_shrink: bool,
}

impl DocAnalysis {
    /// Number of records (lines; a trailing newline doesn't start a new one).
    pub fn line_count(&self) -> usize {
        self.line_ends.len()
    }

    /// Fields of record `line`.
    pub fn line(&self, line: usize) -> &[FieldInfo] {
        let start = if line == 0 {
            0
        } else {
            self.line_ends[line - 1]
        };
        &self.fields[start..self.line_ends[line]]
    }
}

/// Lines of `text` without their terminators, paired with the terminator
/// (`"\n"`, `"\r\n"`, or `""`/`"\r"` for a final unterminated line).
fn lines_with_endings(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.split_inclusive('\n').map(|seg| {
        if let Some(line) = seg.strip_suffix("\r\n") {
            (line, "\r\n")
        } else if let Some(line) = seg.strip_suffix('\n') {
            (line, "\n")
        } else if let Some(line) = seg.strip_suffix('\r') {
            (line, "\r")
        } else {
            (seg, "")
        }
    })
}

fn raise(maxes: &mut Vec<usize>, i: usize, value: usize) {
    if maxes.len() <= i {
        maxes.resize(i + 1, 0);
    }
    maxes[i] = maxes[i].max(value);
}

/// Parse + measure a document.
pub fn analyze_document(text: &str, dialect: Dialect) -> DocAnalysis {
    let mut fields = Vec::new();
    let mut line_ends = Vec::new();
    let mut raw_width_targets: Vec<usize> = Vec::new();
    let mut widths: Vec<usize> = Vec::new();
    let mut has_warnings = false;
    let mut needs_shrink = false;
    let mut raw = Vec::new();
    // A trailing newline doesn't start another record.
    let body = text.strip_suffix('\n').unwrap_or(text);
    for line in body.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        has_warnings |= split_raw(line, dialect, &mut raw);
        let ascii = line.is_ascii();
        // UTF-16 column, advanced incrementally from field to field.
        let (mut col_byte, mut col) = (0usize, 0usize);
        for (i, f) in raw.iter().enumerate() {
            let end_col = if ascii {
                f.end
            } else {
                col += utf16_len(&line[col_byte..f.end]);
                col_byte = f.end;
                col
            };
            let raw_width = display_width(&line[f.start..f.end]);
            let width = if f.is_trimmed() {
                needs_shrink = true;
                display_width(&line[f.tstart..f.tend])
            } else {
                raw_width
            };
            raise(&mut raw_width_targets, i, raw_width);
            raise(&mut widths, i, width);
            fields.push(FieldInfo {
                end_col: end_col as u32,
                raw_width: raw_width as u32,
                width: width as u32,
            });
        }
        line_ends.push(fields.len());
    }
    let mut analysis = DocAnalysis {
        fields,
        line_ends,
        raw_width_targets,
        widths,
        has_warnings,
        needs_align: needs_shrink,
        needs_shrink,
    };
    if !analysis.needs_align {
        analysis.needs_align = (0..analysis.line_count()).any(|l| {
            let line = analysis.line(l);
            let padded = line.len().saturating_sub(1); // last column isn't padded
            line[..padded]
                .iter()
                .zip(&analysis.widths)
                .any(|(f, &w)| (f.width as usize) < w)
        });
    }
    analysis
}

/// Rewrite every line of a warning-free document; `render` receives the
/// CR/LF-stripped line and its field spans and appends the new line content.
fn rewrite_lines(
    text: &str,
    dialect: Dialect,
    mut render: impl FnMut(&str, &[RawField], &mut String),
) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut raw = Vec::new();
    for (line, ending) in lines_with_endings(text) {
        split_raw(line, dialect, &mut raw);
        render(line, &raw, &mut out);
        out.push_str(ending);
    }
    out
}

/// Align whole document with spaces (whitespace-align). Returns new text.
///
/// Returns the input unchanged when any line has unbalanced quotes: reflowing
/// such input corrupts data (e.g. splits quoted commas on a second pass).
/// Idempotent: `align(align(x)) == align(x)` for warning-free input.
pub fn align_document(text: &str, dialect: Dialect) -> String {
    let analysis = analyze_document(text, dialect);
    if analysis.has_warnings {
        return text.to_string();
    }
    let delim = dialect.delimiter();
    rewrite_lines(text, dialect, |line, fields, out| {
        for (i, f) in fields.iter().enumerate() {
            let value = &line[f.tstart..f.tend];
            out.push_str(value);
            if i + 1 < fields.len() {
                // No trailing pad on the last column.
                let pad = analysis.widths[i].saturating_sub(display_width(value));
                out.extend(std::iter::repeat_n(' ', pad));
                out.push(delim);
            }
        }
    })
}

/// Shrink: trim leading/trailing whitespace of every field.
/// Returns `(new_text, changed)`. Returns `(input, false)` unchanged when any
/// line has unbalanced quotes.
pub fn shrink_document(text: &str, dialect: Dialect) -> (String, bool) {
    let analysis = analyze_document(text, dialect);
    if analysis.has_warnings || !analysis.needs_shrink {
        return (text.to_string(), false);
    }
    let delim = dialect.delimiter();
    let out = rewrite_lines(text, dialect, |line, fields, out| {
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                out.push(delim);
            }
            out.push_str(&line[f.tstart..f.tend]);
        }
    });
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
    let lines = analysis.line_count();
    let start = (start_line as usize).min(lines);
    let end = (end_line as usize).saturating_add(1).min(lines);
    let mut pads = Vec::new();
    for lnum in start..end {
        let fields = analysis.line(lnum);
        let padded = fields.len().saturating_sub(1); // last column isn't padded
        for (f, &target) in fields[..padded].iter().zip(&analysis.raw_width_targets) {
            let pad = target.saturating_sub(f.raw_width as usize);
            if pad > 0 {
                pads.push(VirtualPad {
                    line: lnum as u32,
                    col: f.end_col,
                    spaces: pad,
                });
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
        let raw_widths: Vec<Vec<u32>> = (0..analysis.line_count())
            .map(|l| analysis.line(l).iter().map(|f| f.raw_width).collect())
            .collect();
        assert_eq!(raw_widths, vec![vec![3, 1], vec![4, 1]]);
        assert_eq!(analysis.raw_width_targets, vec![4, 1]);
        let pads = pads_for_range(&analysis, 0, 0);
        assert_eq!(
            pads,
            vec![VirtualPad {
                line: 0,
                col: 3,
                spaces: 1
            }]
        );
    }

    #[test]
    fn aligned_document_has_no_virtual_pads() {
        let aligned = align_document("x,y\nxxxxxx,z\n", Dialect::Csv);
        assert_eq!(aligned, "x     ,y\nxxxxxx,z\n");
        assert!(virtual_pads(&aligned, Dialect::Csv).is_empty());
    }

    #[test]
    fn quoted_aligned_document_has_no_virtual_pads() {
        let aligned = align_document("a,\"x\",z\nlong,\"bb\",q\n", Dialect::Csv);
        assert_eq!(aligned, "a   ,\"x\" ,z\nlong,\"bb\",q\n");
        assert!(virtual_pads(&aligned, Dialect::Csv).is_empty());
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

        fn render_field(field: &str, delim: char) -> String {
            if field.contains(delim) || field.contains('"') {
                format!("\"{}\"", field.replace('"', "\"\""))
            } else {
                field.to_owned()
            }
        }

        fn render_quoted(fields: &[String], delim: char) -> String {
            fields
                .iter()
                .map(|f| render_field(f, delim))
                .collect::<Vec<_>>()
                .join(&delim.to_string())
        }

        fn render_padded(fields: &[String], delim: char) -> String {
            fields
                .iter()
                .map(|f| format!("  {}  ", render_field(f, delim)))
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

            #[test]
            fn virtual_pads_align_ragged_rows(
                rows in prop::collection::vec(prop::collection::vec(csv_field(), 2..6), 1..20)
            ) {
                let text = rows
                    .iter()
                    .map(|r| render_padded(r, ','))
                    .collect::<Vec<_>>()
                    .join("\n");
                let analysis = analyze_document(&text, Dialect::Csv);
                prop_assert!(!analysis.has_warnings, "generated well-formed csv warned");
                let pads = pads_for_range(&analysis, 0, u32::MAX);
                let mut delimiter_columns: Vec<Option<usize>> = Vec::new();

                for (line_num, line) in text.split('\n').enumerate() {
                    let (fields, warning) = split_line_spans(line, Dialect::Csv);
                    prop_assert!(!warning, "generated well-formed csv warned");
                    let mut display_col = 0usize;
                    for (i, field) in fields.iter().enumerate() {
                        let pad = pads
                            .iter()
                            .find(|pad| {
                                pad.line == line_num as u32
                                    && pad.col == utf16_len(&line[..field.end]) as u32
                            })
                            .map_or(0, |pad| pad.spaces);
                        display_col += display_width(&line[field.start..field.end]) + pad;

                        if i + 1 < fields.len() {
                            if delimiter_columns.len() <= i {
                                delimiter_columns.resize(i + 1, None);
                            }
                            if let Some(previous) = delimiter_columns[i] {
                                prop_assert_eq!(
                                    previous,
                                    display_col,
                                    "delimiter column {} differs on line {}",
                                    i,
                                    line_num
                                );
                            } else {
                                delimiter_columns[i] = Some(display_col);
                            }
                            display_col += display_width(",");
                        }
                    }
                }
            }
        }
    }
}
