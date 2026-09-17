//! Measuring and cutting a row's styled pairs: `(Style, String)` runs, as the
//! highlighter hands them over and as a row is drawn.
//!
//! Word boundaries, display width and breaking an over-long token are
//! `textwrap`'s and `unicode-width`'s job (design rule 5). What lives here is
//! the part no crate can hold for us: cutting STYLED runs at the columns those
//! answers fall on, so a break or a cut carries every style across untouched.
//!
//! This file began as tuicr's `text_utils.rs` (see CREDITS.md). Nothing of
//! tuicr's is left in it: its one cutter reserved three columns for `...`
//! while every other cut in the reviewer spends one on `…`, and rewriting it
//! over `take_columns` made it ours.
use ratatui::{style::Style, text::Span};
use textwrap::WordSeparator;
use textwrap::core::Word;
use textwrap::wrap_algorithms::wrap_first_fit;
use unicode_width::UnicodeWidthStr;

/// The pairs cut or padded to exactly `width` columns, for a row whose
/// content must end where the pane does.
///
/// A row wider than the pane keeps its first `width - 1` columns and ends in
/// `…`; a narrower one is padded out in `base_style`, so a selection's
/// background reaches the edge. One glyph and one column, the same as
/// `elide_head` and `truncate_width` in `app/text.rs`. A wide character the
/// cut falls inside becomes a space, so the row is exactly `width` wide
/// either way.
pub fn truncate_or_pad_spans(
    spans: &[(Style, String)],
    width: usize,
    base_style: Style,
) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(|(_, text)| text.width()).sum();
    if total > width {
        let mut out: Vec<Span<'static>> = take_columns(spans, width.saturating_sub(1))
            .into_iter()
            .map(|(style, text)| Span::styled(text, style))
            .collect();
        if width > 0 {
            out.push(Span::styled("…", base_style));
        }
        return out;
    }
    let mut out: Vec<Span<'static>> = spans
        .iter()
        .map(|(style, text)| Span::styled(text.clone(), *style))
        .collect();
    if total < width {
        out.push(Span::styled(" ".repeat(width - total), base_style));
    }
    out
}

/// Break styled pairs into the screen lines they occupy at `width`.
///
/// Always returns at least one line, and returns exactly one when the pairs
/// already fit — so a caller that wraps and one that does not have the same
/// shape.
///
/// Word boundaries, display width and breaking an over-long token are
/// `textwrap`'s job, not ours (working rule 5). It answers in words over the
/// row's plain text; this cuts the STYLED pairs at the byte offsets those
/// words fall on, so a break carries every style across untouched.
pub fn wrap_pairs(pairs: &[(Style, String)], width: usize) -> Vec<Vec<(Style, String)>> {
    let plain: String = pairs.iter().map(|(_, t)| t.as_str()).collect();
    if width == 0 || plain.width() <= width {
        return vec![pairs.to_vec()];
    }
    let words: Vec<Word> = WordSeparator::new()
        .find_words(&plain)
        // A token wider than the pane has no break point of its own, and a
        // line the reader cannot see the end of is the bug being fixed. Only
        // such a token, though: `break_apart` returns NOTHING for a word that
        // is pure whitespace, and a line's indent is exactly that word.
        .flat_map(|w| {
            if w.word.width() > width {
                w.break_apart(width).collect::<Vec<_>>()
            } else {
                vec![w]
            }
        })
        .collect();

    let mut cut = Vec::new();
    let mut at = 0;
    for line in wrap_first_fit(&words, &[width as f64]) {
        let start = at;
        for w in line {
            at += w.word.len() + w.whitespace.len();
        }
        // The whitespace a line breaks ON is not drawn: it would pad the row
        // with spaces that carry the line's own background past its last
        // character.
        let end = at - line.last().map_or(0, |w| w.whitespace.len());
        cut.push(slice_pairs(pairs, start, end));
    }
    if cut.is_empty() {
        cut.push(pairs.to_vec());
    }
    cut
}

/// The pairs with their first `cols` DISPLAY columns dropped, styles kept.
///
/// What a horizontal shift needs, and why `slice_pairs` next door cannot do
/// it: that one cuts at BYTE offsets, because `textwrap` answers in them. A
/// shift is a question about columns on a screen.
///
/// A cut that lands inside a wide character emits one space in its place, so
/// the text after it still starts in the column the caller asked for. Dropping
/// the whole character instead would slide every following line by one, and a
/// pane that shifts by a different amount per row is worse than one that
/// shifts too far.
pub fn drop_columns(pairs: &[(Style, String)], cols: usize) -> Vec<(Style, String)> {
    if cols == 0 {
        return pairs.to_vec();
    }
    let mut out: Vec<(Style, String)> = Vec::new();
    // Columns still to be dropped. Once it reaches zero everything is kept.
    let mut left = cols;
    for (style, text) in pairs {
        if left == 0 {
            out.push((*style, text.clone()));
            continue;
        }
        let w = text.width();
        if w <= left {
            left -= w;
            continue;
        }
        let mut kept = String::new();
        for c in text.chars() {
            if left == 0 {
                kept.push(c);
                continue;
            }
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if cw <= left {
                left -= cw;
            } else {
                // The cut fell inside this character. It cannot be drawn in
                // part, so a space holds its remaining columns.
                kept.push_str(&" ".repeat(cw - left));
                left = 0;
            }
        }
        if !kept.is_empty() {
            out.push((*style, kept));
        }
    }
    out
}

/// The first `cols` display columns of `pairs`, styles kept.
///
/// The mirror of [`drop_columns`], and written because the search box needs
/// both: it drops what has scrolled off the left of the query and keeps what
/// fits before the pill on the right. Same rule at the far edge — a character
/// the cut falls inside cannot be drawn in part, so a space holds its columns.
pub fn take_columns(pairs: &[(Style, String)], cols: usize) -> Vec<(Style, String)> {
    let mut out: Vec<(Style, String)> = Vec::new();
    let mut left = cols;
    for (style, text) in pairs {
        if left == 0 {
            break;
        }
        let w = text.width();
        if w <= left {
            left -= w;
            out.push((*style, text.clone()));
            continue;
        }
        let mut kept = String::new();
        for c in text.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if cw > left {
                kept.push_str(&" ".repeat(left));
                left = 0;
                break;
            }
            kept.push(c);
            left -= cw;
        }
        if !kept.is_empty() {
            out.push((*style, kept));
        }
    }
    out
}

/// The pairs covering `start..end` of the concatenated text, styles kept.
pub fn slice_pairs(pairs: &[(Style, String)], start: usize, end: usize) -> Vec<(Style, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (style, text) in pairs {
        let span = at..at + text.len();
        at = span.end;
        let lo = span.start.max(start);
        let hi = span.end.min(end);
        if lo < hi {
            out.push((*style, text[lo - span.start..hi - span.start].to_string()));
        }
    }
    out
}

pub fn split_pairs_at_ranges(
    pairs: &[(Style, String)],
    ranges: Vec<(usize, usize)>,
    highlight: Style,
) -> Vec<(Style, String)> {
    let mut out: Vec<(Style, String)> = Vec::new();
    let mut ranges = ranges.into_iter().peekable();
    let mut span_start = 0;
    for (style, text) in pairs {
        if text.is_empty() {
            out.push((*style, String::new()));
            continue;
        }
        let span_end = span_start + text.len();
        let mut cursor = span_start;
        while cursor < span_end {
            while ranges.peek().is_some_and(|&(_, end)| end <= cursor) {
                ranges.next();
            }
            match ranges.peek().copied() {
                Some((start, end)) if start < span_end => {
                    if start > cursor {
                        out.push((
                            *style,
                            text[cursor - span_start..start - span_start].to_string(),
                        ));
                        cursor = start;
                    }
                    let segment_end = end.min(span_end);
                    out.push((
                        style.patch(highlight),
                        text[cursor - span_start..segment_end - span_start].to_string(),
                    ));
                    cursor = segment_end;
                }
                _ => {
                    out.push((*style, text[cursor - span_start..].to_string()));
                    cursor = span_end;
                }
            }
        }
        span_start = span_end;
    }
    out
}

// A slice holding one `Range` is exactly what `highlight_ranges` takes; the
// lint is about `vec![0..n]` written where a list of numbers was meant.
#[allow(clippy::single_range_in_vec_init)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_pad_highlighted_spans_to_exact_width() {
        // given - highlighted spans from the syntax highlighter (which strips
        // the trailing \n that syntect includes). Short content gets padded
        // by truncate_or_pad_spans; the result must have exactly `width`
        // characters so the side-by-side separator stays aligned.
        let highlighter = crate::vendor::syntax::SyntaxHighlighter::default();
        let lines = vec!["let x = 1;".to_string()];
        let highlighted = highlighter
            .highlight_ranges(std::path::Path::new("test.rs"), &lines, &[0..1])
            .unwrap();
        let spans = &highlighted.spans[&0];

        let width = 80;

        // when
        let result = truncate_or_pad_spans(spans, width, Style::default());

        // then - total char count must equal the target width so each
        // side-by-side column is the same size
        let total_chars: usize = result.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(
            total_chars, width,
            "padded spans should have exactly {width} chars, got {total_chars}"
        );
    }

    #[test]
    fn a_cut_row_ends_in_one_ellipsis_at_exactly_the_width() {
        let st = |n: u8| Style::default().fg(ratatui::style::Color::Indexed(n));
        let pairs = vec![(st(1), "let ".to_string()), (st(2), "x = 1;".to_string())];
        let out = truncate_or_pad_spans(&pairs, 6, Style::default());
        let plain: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(plain, "let x…");
        assert_eq!(plain.width(), 6);
        assert_eq!(out[0].style, st(1), "every style survives the cut");
        assert_eq!(out[1].style, st(2));

        // A wide character the cut falls inside becomes a space, so the row
        // is still exactly as wide as asked, ellipsis included.
        let pairs = vec![(st(1), "ok\u{3042}".to_string())];
        let out = truncate_or_pad_spans(&pairs, 3, Style::default());
        let plain: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(plain, "ok…");
        let out = truncate_or_pad_spans(&pairs, 2, Style::default());
        let plain: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(plain, "o…");
        let pairs = vec![(st(1), "\u{3042}\u{3044}\u{3046}".to_string())];
        let out = truncate_or_pad_spans(&pairs, 4, Style::default());
        let plain: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(plain, "\u{3042} …");
        assert_eq!(plain.width(), 4);
    }

    fn text(line: &[(Style, String)]) -> String {
        line.iter().map(|(_, t)| t.as_str()).collect()
    }

    #[test]
    fn should_return_one_line_when_the_pairs_already_fit() {
        // given
        let pairs = vec![(Style::default(), "fits".to_string())];

        // when
        let lines = wrap_pairs(&pairs, 10);

        // then
        assert_eq!(lines.len(), 1);
        assert_eq!(text(&lines[0]), "fits");
    }

    #[test]
    fn should_break_at_a_word_boundary_and_drop_the_space_it_broke_on() {
        // given
        let pairs = vec![(Style::default(), "one two three".to_string())];

        // when
        let lines = wrap_pairs(&pairs, 7);

        // then
        let out: Vec<String> = lines.iter().map(|l| text(l)).collect();
        assert_eq!(out, vec!["one two", "three"]);
    }

    #[test]
    fn should_carry_every_style_across_a_break() {
        // given - two differently styled runs, broken mid-way
        let red = Style::default().fg(ratatui::style::Color::Red);
        let blue = Style::default().fg(ratatui::style::Color::Blue);
        let pairs = vec![(red, "aaa bbb ".to_string()), (blue, "ccc ddd".to_string())];

        // when
        let lines = wrap_pairs(&pairs, 7);

        // then - the styles survive, and each line keeps only its own runs
        let out: Vec<String> = lines.iter().map(|l| text(l)).collect();
        assert_eq!(out, vec!["aaa bbb", "ccc ddd"]);
        assert_eq!(lines[0][0].0, red);
        assert_eq!(lines[1][0].0, blue);
    }

    #[test]
    fn should_break_a_token_wider_than_the_pane() {
        // given - one word with no break point of its own
        let pairs = vec![(Style::default(), "abcdefghij".to_string())];

        // when
        let lines = wrap_pairs(&pairs, 4);

        // then
        let out: Vec<String> = lines.iter().map(|l| text(l)).collect();
        assert_eq!(out, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn should_keep_leading_indentation_on_the_first_line_only() {
        // given - a code line, indented
        let pairs = vec![(Style::default(), "    let x = compute(a, b);".to_string())];

        // when
        let lines = wrap_pairs(&pairs, 14);

        // then - the indent is text like any other, and never repeats
        let out: Vec<String> = lines.iter().map(|l| text(l)).collect();
        assert_eq!(out[0], "    let x =");
        assert!(!out[1].starts_with(' '), "got {:?}", out[1]);
    }

    #[test]
    fn should_return_one_empty_line_for_no_content() {
        // given
        let pairs: Vec<(Style, String)> = Vec::new();

        // when
        let lines = wrap_pairs(&pairs, 10);

        // then
        assert_eq!(lines.len(), 1);
        assert_eq!(text(&lines[0]), "");
    }

    #[test]
    fn dropping_columns_cuts_from_the_left_and_keeps_every_style() {
        let st = |n: u8| Style::default().fg(ratatui::style::Color::Indexed(n));
        let pairs = vec![(st(1), "let ".to_string()), (st(2), "x = 1;".to_string())];

        // Nothing to drop.
        assert_eq!(drop_columns(&pairs, 0), pairs);

        // Inside the first pair: the rest of it survives, with its own style.
        assert_eq!(
            drop_columns(&pairs, 2),
            vec![(st(1), "t ".to_string()), (st(2), "x = 1;".to_string())]
        );

        // Past the first pair: it goes entirely, the second is cut.
        assert_eq!(drop_columns(&pairs, 6), vec![(st(2), "= 1;".to_string())]);

        // Past everything.
        assert!(drop_columns(&pairs, 40).is_empty());
    }

    /// A cut inside a wide character must still leave the text starting where
    /// the caller asked, or every row shifts by a different amount.
    #[test]
    fn dropping_columns_pads_a_wide_character_it_cuts_through() {
        let st = Style::default();
        let pairs = vec![(st, "\u{3042}\u{3044}ok".to_string())];
        // Two columns each: dropping one lands inside the first.
        let out = drop_columns(&pairs, 1);
        let plain: String = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(plain, " \u{3044}ok");
        assert_eq!(plain.width(), 5, "the row must lose exactly one column");

        // A clean boundary drops the character outright.
        let out = drop_columns(&pairs, 2);
        let plain: String = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(plain, "\u{3044}ok");
    }

    #[test]
    fn taking_columns_cuts_from_the_right_and_keeps_every_style() {
        let st = |n: u8| Style::default().fg(ratatui::style::Color::Indexed(n));
        let pairs = vec![(st(1), "let ".to_string()), (st(2), "x = 1;".to_string())];

        assert!(take_columns(&pairs, 0).is_empty());

        // Inside the first pair, and the second is never reached.
        assert_eq!(take_columns(&pairs, 2), vec![(st(1), "le".to_string())]);

        // Into the second, which keeps its own style.
        assert_eq!(
            take_columns(&pairs, 6),
            vec![(st(1), "let ".to_string()), (st(2), "x ".to_string())]
        );

        // More than there is.
        assert_eq!(take_columns(&pairs, 40), pairs);
    }

    /// The far edge follows the same rule as the near one: a character the cut
    /// falls inside cannot be drawn in part, so a space holds its columns and
    /// the width the caller asked for is the width they get.
    #[test]
    fn taking_columns_pads_a_wide_character_it_cuts_through() {
        let st = Style::default();
        let pairs = vec![(st, "ok\u{3042}\u{3044}".to_string())];
        let out = take_columns(&pairs, 3);
        let plain: String = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(plain, "ok ");
        assert_eq!(plain.width(), 3, "exactly the columns asked for");

        let out = take_columns(&pairs, 4);
        let plain: String = out.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(plain, "ok\u{3042}");
    }
}
