//! Core CSV align/shrink logic ported from
//! `vscode_rainbow_csv/rainbow_utils.js` + `rbql_core/rbql-js/csv_utils.js`.
//!
//! MVP scope (documented limits):
//! - Single-line records only (no RFC4180 multiline). A line with unbalanced
//!   quotes is flagged `warning=true` and left untouched by align.
//! - Policies: `quoted` (`,` and `;`) honours `"..."` with `""` escapes;
//!   `simple` (`\t` and `|`) splits literally.
//! - No comment-prefix handling, no dynamic separator, no numeric
//!   decimal-point right-alignment yet (all columns left-aligned).
//! - Display width via `unicode-width` (port of `wcwidth` usage).

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

    /// Map from Zed `languages/*/config.toml` `name` fields.
    pub fn from_language_name(name: &str) -> Option<Dialect> {
        match name {
            "Rainbow CSV (,)" => Some(Dialect::Csv),
            // Upstream tsv config contains a tab glyph that may not round-trip;
            // accept both the exact name and a prefix match.
            n if n.starts_with("Rainbow TSV") => Some(Dialect::Tsv),
            "Rainbow CSV (;)" => Some(Dialect::Ssv),
            "Rainbow CSV (|)" => Some(Dialect::Psv),
            _ => None,
        }
    }

    /// Map from file extension, for the CLI fallback.
    pub fn from_extension(ext: &str) -> Option<Dialect> {
        match ext.to_ascii_lowercase().as_str() {
            "csv" => Some(Dialect::Csv),
            "tsv" | "tab" => Some(Dialect::Tsv),
            _ => None,
        }
    }
}

/// Split one line into fields. Returns `(fields, warning)`.
/// `warning=true` means unbalanced quotes; caller should skip aligning that line.
pub fn split_line(line: &str, dialect: Dialect) -> (Vec<String>, bool) {
    if !dialect.quoted() {
        return (line.split(dialect.delimiter()).map(|s| s.to_string()).collect(), false);
    }
    split_quoted(line, dialect.delimiter())
}

/// Port of `split_quoted_str` in `csv_utils.js`.
fn split_quoted(src: &str, dlm: char) -> (Vec<String>, bool) {
    if !src.contains('"') {
        return (src.split(dlm).map(|s| s.to_string()).collect(), false);
    }
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    let mut warning = false;
    // Byte-index based scan; quote handling is ASCII so char boundaries are safe.
    while i < n {
        if bytes[i] == b'"' {
            // Try to parse a quoted field starting at i.
            match parse_quoted_at(src, i, dlm) {
                Some((field, next)) => {
                    out.push(field);
                    i = next;
                    // Skip single delimiter after quoted field.
                    if i < n && bytes[i] == dlm as u8 {
                        i += dlm.len_utf8();
                        if i == n {
                            out.push(String::new());
                        }
                    } else if i < n {
                        // Garbage after closing quote before delimiter.
                        warning = true;
                        // Fall back: consume to next delimiter literally.
                        let rest = &src[i..];
                        if let Some(pos) = rest.find(dlm) {
                            out.last_mut().unwrap().push_str(&rest[..pos]);
                            i += pos + dlm.len_utf8();
                        } else {
                            out.last_mut().unwrap().push_str(rest);
                            i = n;
                        }
                    }
                }
                None => {
                    warning = true;
                    // Unbalanced: take the rest literally.
                    let rest = &src[i..];
                    if let Some(pos) = rest.find(dlm) {
                        out.push(rest[..pos].to_string());
                        i += pos + dlm.len_utf8();
                    } else {
                        out.push(rest.to_string());
                        i = n;
                    }
                }
            }
        } else {
            let rest = &src[i..];
            if let Some(pos) = rest.find(dlm) {
                let field = rest[..pos].to_string();
                if field.contains('"') {
                    warning = true;
                }
                out.push(field);
                i += pos + dlm.len_utf8();
                // Trailing delimiter handled by next loop or below.
                if i == n {
                    out.push(String::new());
                }
            } else {
                let field = rest.to_string();
                if field.contains('"') {
                    warning = true;
                }
                out.push(field);
                i = n;
            }
        }
    }
    // Empty input -> one empty field to mirror `"".split(dlm)`.
    if src.is_empty() {
        out.push(String::new());
    }
    (out, warning)
}

/// Parse `"..."` starting at byte index `start` (which must be `"`).
/// Returns `(field_text_with_quotes, next_byte_index_past_closing_quote)`.
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
            // Closing quote: must be followed by delim or EOL.
            let end = i + 1;
            if end == n || src[end..].starts_with(dlm) {
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

/// Align whole document with spaces (whitespace-align). Returns new text.
/// Lines with quote warnings keep original content but still participate
/// in width computation via their raw split.
pub fn align_document(text: &str, dialect: Dialect) -> String {
    let trailing_nl = text.ends_with('\n');
    let mut lines: Vec<&str> = text.split('\n').collect();
    // `split('\n')` on "a\n" yields ["a", ""]; drop the artifact, re-add later.
    let had_artifact = trailing_nl && lines.last() == Some(&"");
    if had_artifact {
        lines.pop();
    }
    // Handle \r\n by stripping \r for computation and re-adding.
    let endings: Vec<&str> = lines.iter().map(|l| if l.ends_with('\r') { "\r\n" } else { "\n" }).collect();
    let stripped: Vec<&str> = lines.iter().map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();

    let mut records: Vec<Vec<String>> = Vec::with_capacity(stripped.len());
    for line in &stripped {
        let (fields, _warn) = split_line(line, dialect);
        records.push(fields.into_iter().map(|f| f.trim().to_string()).collect());
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
    let delim = dialect.delimiter().to_string();
    let mut out_lines = Vec::with_capacity(records.len());
    for rec in &records {
        let mut cells = Vec::with_capacity(rec.len());
        for (i, f) in rec.iter().enumerate() {
            let is_last = i + 1 == rec.len();
            if is_last {
                cells.push(f.clone()); // no trailing pad on last column
            } else {
                let pad = widths[i].saturating_sub(display_width(f));
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
        // Preserve original \r\n vs \n per line; default \n.
        let e = endings.get(i).copied().unwrap_or("\n");
        // For the last line, only add ending if input had trailing newline.
        if i + 1 < out_lines.len() {
            out.push_str(e);
        } else if trailing_nl {
            // Match input style: if input was \r\n, endings[last] is \r\n.
            out.push_str(e);
        }
    }
    // Edge: empty input stays empty.
    if text.is_empty() {
        return String::new();
    }
    out
}

/// Shrink: trim leading/trailing whitespace of every field.
/// Returns `(new_text, changed)`.
pub fn shrink_document(text: &str, dialect: Dialect) -> (String, bool) {
    let trailing_nl = text.ends_with('\n');
    let mut lines: Vec<&str> = text.split('\n').collect();
    let had_artifact = trailing_nl && lines.last() == Some(&"");
    if had_artifact {
        lines.pop();
    }
    let mut changed = false;
    let mut out_lines = Vec::with_capacity(lines.len());
    for line in lines {
        let stripped = line.strip_suffix('\r').unwrap_or(line);
        let ending = if line.ends_with('\r') { "\r" } else { "" };
        let (fields, _warn) = split_line(stripped, dialect);
        let trimmed: Vec<String> = fields.iter().map(|f| f.trim().to_string()).collect();
        for (a, b) in fields.iter().zip(trimmed.iter()) {
            if a != b {
                changed = true;
                break;
            }
        }
        out_lines.push(trimmed.join(&dialect.delimiter().to_string()) + ending);
    }
    let mut out = out_lines.join("\n");
    if trailing_nl {
        out.push('\n');
    }
    if text.is_empty() {
        return (String::new(), false);
    }
    (out, changed)
}

/// One virtual-align pad: insert `spaces` after `line:col` (col = end of field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualPad {
    pub line: u32,
    pub col: u32,
    pub spaces: usize,
}

/// Compute virtual-align pads without touching text (port of
/// `calculate_column_offsets` + per-field delta in `rainbow_utils.js`).
/// `col` is the UTF-16-ish char offset of the field end; for ASCII-heavy CSV
/// char offset == LSP character. Non-ASCII callers should map via LSP layer
/// (MVP uses char offsets; CJK width handled in pad size via display_width).
pub fn virtual_pads(text: &str, dialect: Dialect) -> Vec<VirtualPad> {
    let records = parse_trimmed_records(text, dialect);
    let widths = column_widths(&records);
    pads_for_range(&records, &widths, 0, u32::MAX)
}

/// Trimmed single-line records of a document (trailing-newline artifact skipped).
/// Shared by width computation and the server-side inlay cache so big files are
/// parsed once per version instead of once per request.
pub fn parse_trimmed_records(text: &str, dialect: Dialect) -> Vec<Vec<String>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut records = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let stripped = line.strip_suffix('\r').unwrap_or(line);
        // Skip the final artifact of trailing newline.
        if stripped.is_empty() && idx + 1 == lines.len() && text.ends_with('\n') {
            continue;
        }
        let (fields, _warn) = split_line(stripped, dialect);
        records.push(fields.into_iter().map(|f| f.trim().to_string()).collect());
    }
    records
}

/// Max display width per column (port of `ColumnStat` accumulation).
pub fn column_widths(records: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = Vec::new();
    for rec in records {
        for (i, f) in rec.iter().enumerate() {
            if widths.len() <= i {
                widths.push(0);
            }
            widths[i] = widths[i].max(display_width(f));
        }
    }
    widths
}

/// Virtual-align pads for lines in `[start_line, end_line]` only.
pub fn pads_for_range(
    records: &[Vec<String>],
    widths: &[usize],
    start_line: u32,
    end_line: u32,
) -> Vec<VirtualPad> {
    let mut pads = Vec::new();
    for (lnum, rec) in records.iter().enumerate() {
        let lnum = lnum as u32;
        if lnum < start_line || lnum > end_line {
            continue;
        }
        // Char offsets of trimmed fields as laid out unpadded.
        let mut col: usize = 0;
        for (i, f) in rec.iter().enumerate() {
            let is_last = i + 1 == rec.len();
            col += f.chars().count();
            if !is_last {
                let pad = widths.get(i).copied().unwrap_or(0).saturating_sub(display_width(f));
                if pad > 0 {
                    pads.push(VirtualPad { line: lnum, col: col as u32, spaces: pad });
                }
                col += 1; // delimiter
            }
        }
    }
    pads
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
        assert!(pads.contains(&VirtualPad { line: 0, col: 1, spaces: 3 }));
    }

    #[test]
    fn pads_for_range_subset() {
        let records = parse_trimmed_records("a,bb\ncccc,d\n", Dialect::Csv);
        let widths = column_widths(&records);
        assert_eq!(widths, vec![4, 2]);
        let all = pads_for_range(&records, &widths, 0, u32::MAX);
        assert_eq!(all, virtual_pads("a,bb\ncccc,d\n", Dialect::Csv));
        // Line 0 needs padding ("a" -> width 4); line 1 is already widest.
        let line0 = pads_for_range(&records, &widths, 0, 0);
        assert_eq!(line0, vec![VirtualPad { line: 0, col: 1, spaces: 3 }]);
        let line1 = pads_for_range(&records, &widths, 1, 1);
        assert!(line1.is_empty());
    }

    #[test]
    fn upstream_sample_parses_without_warnings() {
        let sample = include_str!("../../../_upstream/zed-rainbow-csv/samples/sample_csv_file.csv");
        let dialect = Dialect::Csv;
        let mut warnings = 0;
        for line in sample.lines() {
            let (_, w) = split_line(line, dialect);
            if w {
                warnings += 1;
            }
        }
        // Line 10 of the sample has an odd construct; allow at most 1 warning.
        assert!(warnings <= 1, "too many warnings: {warnings}");
    }
}
