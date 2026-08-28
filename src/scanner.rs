//! One scanner over PL/pgSQL body text, answering whether a byte is inside a
//! string or a comment or is live code.
//!
//! Every transform shares it. Separate scanners disagreed about the `''`
//! escape.

use alloc::{string::String, vec, vec::Vec};

/// What the scanner is inside at a given offset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Region {
    /// Live code, where a keyword is a keyword.
    Code,
    /// Anything quoted or commented, where it is not.
    Quoted,
}

/// Walks `text` once, reporting the region each byte belongs to.
///
/// Delimiters count as [`Region::Quoted`], so a keyword scan can skip every
/// quoted byte.
pub struct Scanner<'a> {
    text: &'a str,
}

impl<'a> Scanner<'a> {
    /// Creates a scanner over `text`.
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }

    /// The region of every byte offset in the text, as a parallel map.
    #[must_use]
    pub fn regions(&self) -> Vec<Region> {
        let mut regions = vec![Region::Code; self.text.len()];
        let mut index = 0;

        // MUTANT: equivalent: `<=` reaches an empty rest, which next_span
        // classifies as one code character and the loop then exits anyway.
        while index < self.text.len() {
            let span = match next_span(&self.text[index..]) {
                Span::Verbatim(len) => len,
                Span::Dollar(dollar) => dollar.span,
                Span::Code(character) => {
                    index += character.len_utf8();
                    continue;
                }
            };

            for region in regions.iter_mut().skip(index).take(span) {
                *region = Region::Quoted;
            }
            index += span;
        }

        regions
    }

    /// True when a dollar-quoted literal opens and never closes, swallowing the
    /// rest of the text.
    #[must_use]
    pub fn has_unterminated_dollar_quote(&self) -> bool {
        let mut index = 0;

        // MUTANT: equivalent: `<=` reaches an empty rest, which next_span
        // classifies as one code character and the loop then exits anyway.
        while index < self.text.len() {
            match next_span(&self.text[index..]) {
                Span::Verbatim(len) => index += len,
                Span::Dollar(dollar) if !dollar.closed => return true,
                Span::Dollar(dollar) => index += dollar.span,
                Span::Code(character) => index += character.len_utf8(),
            }
        }

        false
    }

    /// The offset of the first `needle` byte that is live code.
    #[must_use]
    pub fn find_in_code(&self, needle: u8) -> Option<usize> {
        let regions = self.regions();
        self.text
            .bytes()
            .zip(regions)
            .position(|(byte, region)| byte == needle && region == Region::Code)
    }

    /// The offset of the first occurrence of `needle` that is live code.
    #[must_use]
    pub fn find_str_in_code(&self, needle: &str) -> Option<usize> {
        let regions = self.regions();
        let haystack = self.text.as_bytes();
        if needle.is_empty() || needle.len() > haystack.len() {
            return None;
        }
        (0..=haystack.len() - needle.len()).find(|&start| {
            regions[start] == Region::Code && haystack[start..].starts_with(needle.as_bytes())
        })
    }

    /// The offset of `keyword` as a whole word in live code, searched from
    /// `from`, comparing case-insensitively.
    ///
    /// Both sides need a word boundary, which is what stops `myelsif` from
    /// matching `elsif`.
    #[must_use]
    pub fn find_keyword(&self, keyword: &str, from: usize) -> Option<usize> {
        let haystack = self.text.as_bytes();
        let needle = keyword.as_bytes();
        if needle.is_empty() || needle.len() > haystack.len() {
            return None;
        }
        let regions = self.regions();

        (from..=haystack.len() - needle.len()).find(|&start| {
            regions.get(start).copied() != Some(Region::Quoted)
                && haystack[start..start + needle.len()].eq_ignore_ascii_case(needle)
                && !is_word_byte(start.checked_sub(1).map(|before| haystack[before]))
                && !is_word_byte(haystack.get(start + needle.len()).copied())
        })
    }

    /// The next word of live code at or after `from`, skipping whitespace and
    /// anything quoted or commented.
    #[must_use]
    pub fn next_word_in_code(&self, from: usize) -> Option<&'a str> {
        let regions = self.regions();
        let bytes = self.text.as_bytes();
        let start = (from..bytes.len())
            .find(|&index| regions[index] == Region::Code && !bytes[index].is_ascii_whitespace())?;
        let end = (start..bytes.len())
            .find(|&index| regions[index] != Region::Code || !is_word_byte(Some(bytes[index])))
            .unwrap_or(bytes.len());
        (end > start).then(|| &self.text[start..end])
    }

    /// Split on every `separator` byte that is live code.
    #[must_use]
    pub fn split_in_code(&self, separator: u8) -> Vec<&'a str> {
        let regions = self.regions();
        let mut pieces = Vec::new();
        let mut start = 0;
        for (index, (byte, region)) in self.text.bytes().zip(regions).enumerate() {
            if byte == separator && region == Region::Code {
                pieces.push(&self.text[start..index]);
                start = index + 1;
            }
        }
        pieces.push(&self.text[start..]);
        pieces
    }

    /// Re-emits every dollar-quoted literal as a single-quoted one, doubling
    /// any single quote inside it.
    ///
    /// `$tag$...$tag$` has no SQLite counterpart, and doubling is the only
    /// escape SQLite has. Other quoted spans are copied byte for byte, so a `$`
    /// inside an ordinary string is not mistaken for a delimiter.
    #[must_use]
    pub fn requote_dollar_literals(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut index = 0;

        while index < self.text.len() {
            let rest = &self.text[index..];
            match next_span(rest) {
                Span::Verbatim(len) => {
                    out.push_str(&rest[..len]);
                    index += len;
                }
                Span::Dollar(dollar) => {
                    out.push('\'');
                    out.push_str(&dollar.inner.replace('\'', "''"));
                    out.push('\'');
                    index += dollar.span;
                }
                Span::Code(character) => {
                    out.push(character);
                    index += character.len_utf8();
                }
            }
        }

        out
    }
}

/// True for a byte that can appear inside an identifier.
fn is_word_byte(byte: Option<u8>) -> bool {
    byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$')
}

/// Length of the single-quoted string at the front of `rest`, both quotes
/// included.
///
/// A doubled quote is an escape and stays inside. Backslash escapes are
/// honored too, for `E'...'` strings.
fn single_quoted_len(rest: &str) -> usize {
    let bytes = rest.as_bytes();
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            // MUTANT: equivalent: closing here instead lets the second quote
            // open a fresh span over the same bytes, so the region map matches.
            b'\'' if bytes.get(index + 1) == Some(&b'\'') => index += 2,
            b'\'' => return index + 1,
            _ => index += 1,
        }
    }
    rest.len()
}

/// Length of the double-quoted identifier starting at the front of `rest`.
fn double_quoted_len(rest: &str) -> usize {
    let bytes = rest.as_bytes();
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            // MUTANT: equivalent: closing here instead lets the second quote
            // open a fresh span over the same bytes, so the region map matches.
            b'"' if bytes.get(index + 1) == Some(&b'"') => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    rest.len()
}

/// A dollar-quoted literal at the front of the text.
struct DollarQuoted<'a> {
    /// Byte length of the literal, both delimiters included.
    span: usize,
    /// The text between the delimiters.
    inner: &'a str,
    /// False when the closing delimiter is absent.
    closed: bool,
}

/// One lexical span at the front of `rest`.
enum Span<'a> {
    /// A comment or a quoted string, of this byte length, copied as it stands.
    Verbatim(usize),
    /// A dollar-quoted literal, which has no SQLite spelling.
    Dollar(DollarQuoted<'a>),
    /// One character of live code.
    Code(char),
}

/// Classifies the span starting at the front of `rest`, which must not be
/// empty.
///
/// Every walk over the text goes through here, so the region map, the
/// requoting, and the unterminated-literal check cannot drift apart.
fn next_span(rest: &str) -> Span<'_> {
    if let Some(after) = rest.strip_prefix("--") {
        Span::Verbatim(after.find('\n').map_or(rest.len(), |end| end + 3))
    } else if rest.starts_with("/*") {
        Span::Verbatim(block_comment_len(rest))
    } else if rest.starts_with('\'') {
        Span::Verbatim(single_quoted_len(rest))
    } else if rest.starts_with('"') {
        Span::Verbatim(double_quoted_len(rest))
    } else if let Some(dollar) = dollar_quoted(rest) {
        Span::Dollar(dollar)
    } else {
        // By character, not by byte: a multi-byte character would not be a
        // slice boundary.
        Span::Code(rest.chars().next().unwrap_or('\0'))
    }
}

/// The dollar-quoted string starting at the front of `rest`, or `None` when
/// the `$` does not open one.
///
/// The tag is everything between the opening `$` and the next `$`, and it must
/// be a valid identifier or empty, so `$1` is a placeholder rather than a tag.
/// An unterminated literal runs to the end of the text.
fn dollar_quoted(rest: &str) -> Option<DollarQuoted<'_>> {
    let after_first = rest.strip_prefix('$')?;
    let tag_end = after_first.find('$')?;
    let tag = &after_first[..tag_end];
    if !tag.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    if tag.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }

    let opener_len = tag_end + 2;
    let delimiter = &rest[..opener_len];
    Some(match rest[opener_len..].find(delimiter) {
        Some(end) => DollarQuoted {
            span: opener_len + end + delimiter.len(),
            inner: &rest[opener_len..opener_len + end],
            closed: true,
        },
        None => DollarQuoted {
            span: rest.len(),
            inner: &rest[opener_len..],
            closed: false,
        },
    })
}

/// Length of the nested block comment starting at the front of `rest`.
fn block_comment_len(rest: &str) -> usize {
    let bytes = rest.as_bytes();
    let mut depth = 1usize;
    let mut i = 2; // past the opening /*
    // MUTANT: equivalent: `<=` reads one index past the end, where both `get`
    // calls yield None and the fallthrough arm exits the loop.
    while i < bytes.len() {
        match (bytes.get(i).copied(), bytes.get(i + 1).copied()) {
            (Some(b'/'), Some(b'*')) => {
                depth += 1;
                i += 2;
            }
            (Some(b'*'), Some(b'/')) => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return i;
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    rest.len()
}

#[cfg(test)]
mod tests {
    use super::{Region, Scanner};
    use alloc::{vec, vec::Vec};
    use proptest::prelude::*;

    fn regions_of(text: &str) -> Vec<Region> {
        Scanner::new(text).regions()
    }

    #[test]
    fn a_double_quoted_identifier_ends_at_its_closing_quote() {
        let text = "x \"a\" ; y";
        let regions = regions_of(text);
        assert_eq!(regions[2], Region::Quoted, "the opening quote");
        assert_eq!(regions[3], Region::Quoted, "the identifier body");
        assert_eq!(regions[4], Region::Quoted, "the closing quote");
        assert_eq!(regions[5], Region::Code, "the space after it");
        assert_eq!(Scanner::new(text).find_in_code(b';'), Some(6));
    }

    #[test]
    fn a_doubled_double_quote_does_not_close_the_identifier() {
        let text = "\"abc\"\"d\" ; x";
        assert_eq!(Scanner::new(text).find_in_code(b';'), Some(9));
    }

    /// A doubled quote right after the opening one closes an empty identifier
    /// rather than escaping, so the quote that follows opens a new span.
    #[test]
    fn a_doubled_quote_at_the_start_closes_an_empty_identifier() {
        assert_eq!(Scanner::new("\"\"a\" ; x").find_in_code(b';'), None);
    }

    #[test]
    fn a_backslash_escapes_the_next_quote() {
        assert_eq!(Scanner::new("'abc\\'d';").find_in_code(b';'), Some(8));
    }

    #[test]
    fn an_empty_string_closes_at_its_second_quote() {
        assert_eq!(Scanner::new("'';").find_in_code(b';'), Some(2));
    }

    #[test]
    fn a_doubled_quote_late_in_a_string_stays_inside() {
        assert_eq!(Scanner::new("'abc''d';").find_in_code(b';'), Some(8));
    }

    #[test]
    fn an_underscore_is_allowed_in_a_dollar_tag() {
        // The semicolon sits after the literal, so rejecting `_` as a tag
        // character shifts it inside a bogus literal opened further along.
        assert_eq!(Scanner::new("$_t$x$_t$;").find_in_code(b';'), Some(9));
    }

    #[test]
    fn a_keyword_touching_an_underscore_is_not_a_match() {
        assert_eq!(Scanner::new("_ELSIF x").find_keyword("ELSIF", 0), None);
    }

    /// A needle exactly as long as the haystack still matches.
    #[test]
    fn a_full_length_needle_is_found() {
        assert_eq!(Scanner::new(":=").find_str_in_code(":="), Some(0));
        assert_eq!(Scanner::new("ELSIF").find_keyword("ELSIF", 0), Some(0));
    }

    #[test]
    fn a_non_word_byte_yields_no_next_word() {
        assert_eq!(Scanner::new("; x").next_word_in_code(0), None);
    }

    #[test]
    fn a_closed_literal_before_an_open_one_is_still_reported() {
        assert!(Scanner::new("$a$x$a$ $b$").has_unterminated_dollar_quote());
    }

    #[test]
    fn a_doubled_quote_does_not_close_the_string() {
        let text = "a 'it''s' b";
        let regions = regions_of(text);
        assert_eq!(regions[2], Region::Quoted, "the opening quote");
        assert_eq!(regions[8], Region::Quoted, "the closing quote");
        assert_eq!(regions[10], Region::Code, "the b after it");
    }

    #[test]
    fn a_dollar_quote_spans_its_tag() {
        let text = "x $tag$ ; $tag$ y";
        assert_eq!(Scanner::new(text).find_in_code(b';'), None);
        assert_eq!(regions_of(text)[16], Region::Code);
    }

    #[test]
    fn an_unterminated_dollar_quote_requotes_to_the_end() {
        assert_eq!(Scanner::new("$$").requote_dollar_literals(), "''");
        assert_eq!(Scanner::new("$tag$abc").requote_dollar_literals(), "'abc'");
        assert_eq!(
            Scanner::new("$na\u{ef}ve$x").requote_dollar_literals(),
            "'x'"
        );
    }

    #[test]
    fn an_unterminated_dollar_quote_is_reported() {
        assert!(Scanner::new("$$").has_unterminated_dollar_quote());
        assert!(Scanner::new("a := $tag$abc;").has_unterminated_dollar_quote());
        assert!(!Scanner::new("a := $tag$abc$tag$;").has_unterminated_dollar_quote());
        assert!(!Scanner::new("x").has_unterminated_dollar_quote());
    }

    /// A `$$` that is commented out or quoted opens nothing.
    #[test]
    fn a_dollar_quote_inside_another_span_is_not_unterminated() {
        assert!(!Scanner::new("-- $$\nx").has_unterminated_dollar_quote());
        assert!(!Scanner::new("/* $$ */ x").has_unterminated_dollar_quote());
        assert!(!Scanner::new("'$$'").has_unterminated_dollar_quote());
    }

    #[test]
    fn a_placeholder_is_not_a_dollar_quote() {
        let text = "$1 ; $2";
        assert_eq!(Scanner::new(text).find_in_code(b';'), Some(3));
    }

    #[test]
    fn a_keyword_needs_a_boundary_on_both_sides() {
        assert_eq!(Scanner::new("myelsif x").find_keyword("elsif", 0), None);
        assert_eq!(Scanner::new("elsifx x").find_keyword("elsif", 0), None);
        assert_eq!(Scanner::new("a ELSIF b").find_keyword("elsif", 0), Some(2));
    }

    #[test]
    fn a_keyword_inside_a_comment_is_not_found() {
        assert_eq!(Scanner::new("-- elsif\nx").find_keyword("elsif", 0), None);
        assert_eq!(Scanner::new("/* elsif */ x").find_keyword("elsif", 0), None);
    }

    /// A haystack shorter than the keyword has no match and must not index
    /// past its end.
    #[test]
    fn the_next_word_skips_whitespace_and_comments() {
        let scanner = Scanner::new("EXCEPTION -- note\n  WHEN x");
        assert_eq!(scanner.next_word_in_code(9), Some("WHEN"));
        assert_eq!(Scanner::new("EXCEPTION").next_word_in_code(9), None);
    }

    #[test]
    fn a_substring_inside_a_literal_is_not_found() {
        assert_eq!(Scanner::new("a := 1").find_str_in_code(":="), Some(2));
        assert_eq!(Scanner::new("'x := y'").find_str_in_code(":="), None);
    }

    #[test]
    fn a_short_text_has_no_keyword() {
        assert_eq!(Scanner::new("x, y").find_keyword("DEFAULT", 0), None);
        assert_eq!(Scanner::new("").find_keyword("BEGIN", 0), None);
    }

    #[test]
    fn splitting_ignores_separators_inside_quotes() {
        let pieces = Scanner::new("a := 'x;y'; b := 2").split_in_code(b';');
        assert_eq!(pieces, vec!["a := 'x;y'", " b := 2"]);
    }

    #[test]
    fn unicode_in_live_code_does_not_panic() {
        let text = "\u{e9} SELECT";
        let regions = regions_of(text);
        assert_eq!(regions.len(), text.len());
        assert_eq!(regions[0], Region::Code);
        assert_eq!(regions[1], Region::Code);
        assert_eq!(regions[2], Region::Code);
    }

    #[test]
    fn unicode_inside_a_comment_does_not_panic() {
        let text = "-- caf\u{e9}\nSELECT 1";
        let regions = regions_of(text);
        assert_eq!(regions.len(), text.len());
        let select_byte = text.find("SELECT").unwrap();
        assert_eq!(regions[select_byte], Region::Code);
    }

    #[test]
    fn unicode_in_dollar_quoted_string_does_not_panic() {
        let text = "$tag$caf\u{e9}$tag$ x";
        let regions = regions_of(text);
        assert_eq!(regions.len(), text.len());
        let x_byte = text.rfind('x').unwrap();
        assert_eq!(regions[x_byte], Region::Code);
    }

    #[test]
    fn nested_block_comment_is_fully_quoted() {
        let text = "/* a /* b */ c */ x";
        let regions = regions_of(text);
        assert_eq!(regions[10], Region::Quoted, "inner * must be Quoted");
        assert_eq!(regions[11], Region::Quoted, "inner / must be Quoted");
        assert_eq!(
            regions[12],
            Region::Quoted,
            "space after inner */ must be Quoted"
        );
        assert_eq!(regions[15], Region::Quoted, "outer * must be Quoted");
        assert_eq!(regions[16], Region::Quoted, "outer / must be Quoted");
        assert_eq!(
            regions[17],
            Region::Code,
            "space after outer comment must be Code"
        );
    }

    #[test]
    fn doubly_nested_block_comment_is_fully_quoted() {
        let text = "/* /* /* x */ */ */ y";
        let regions = regions_of(text);
        assert_eq!(regions[17], Region::Quoted, "outermost * must be Quoted");
        assert_eq!(regions[18], Region::Quoted, "outermost / must be Quoted");
        assert_eq!(
            regions[19],
            Region::Code,
            "space after outer comment must be Code"
        );
    }

    #[test]
    fn non_nested_block_comment_still_works_after_fix() {
        let text = "/* simple */ x";
        let regions = regions_of(text);
        let x = text.rfind('x').unwrap();
        assert_eq!(regions[x - 1], Region::Code);
        assert_eq!(regions[x - 2], Region::Quoted);
        assert_eq!(regions[x], Region::Code);
    }

    proptest! {
        #[test]
        fn regions_length_equals_byte_count(s in ".*") {
            let regions = Scanner::new(&s).regions();
            prop_assert_eq!(regions.len(), s.len());
        }
    }

    proptest! {
        #[test]
        fn find_in_code_agrees_with_regions(s in ".*", byte in any::<u8>()) {
            let scanner = Scanner::new(&s);
            let regions = scanner.regions();
            let found = scanner.find_in_code(byte);
            let manual = s
                .bytes()
                .zip(regions.iter())
                .position(|(b, r)| b == byte && *r == Region::Code);
            prop_assert_eq!(found, manual);
        }
    }

    proptest! {
        /// Requoting is total: no text makes it panic.
        #[test]
        fn requote_accepts_any_text(s in ".*") {
            let _ = Scanner::new(&s).requote_dollar_literals();
        }
    }
}
