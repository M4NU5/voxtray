//! Splits selected text into sentence-sized chunks for synthesis.
//!
//! Chunks never cross a paragraph break. The first chunk is a single sentence
//! so audio starts quickly. Later sentences in a paragraph are merged up to
//! [`TARGET_CHARS`] for fewer requests and steadier intonation. Anything longer
//! than [`MAX_CHARS`] is split at the last clause mark or space that fits.
//!
//! No text is dropped or reordered: only whitespace changes.

use unicode_segmentation::UnicodeSegmentation;

/// Sentences in a paragraph are merged until a chunk would exceed this.
pub const TARGET_CHARS: usize = 220;
/// Hard upper bound on a chunk. Well under every provider's input limit.
pub const MAX_CHARS: usize = 400;

/// Words that end in a full stop without ending the sentence.
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "fig", "no", "vol", "approx",
    "inc", "ltd", "co", "mt", "gen", "col", "lt", "sgt", "capt", "rev", "dept", "est",
];

/// Split `text` into chunks ready to synthesise, in reading order.
pub fn chunk(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    for para in paragraphs(text) {
        let mut current = String::new();
        for sentence in sentences(&para) {
            for piece in split_long(&sentence, MAX_CHARS) {
                let fits = len(&current) + 1 + len(&piece) <= TARGET_CHARS;
                // The very first sentence always stands alone.
                if !current.is_empty() && (chunks.is_empty() || !fits) {
                    chunks.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&piece);
            }
        }
        if !current.is_empty() {
            chunks.push(current);
        }
    }
    chunks
}

fn len(s: &str) -> usize {
    s.chars().count()
}

/// Group lines into paragraphs, unwrapping hard line breaks (PDFs, email).
///
/// A line break is kept as a paragraph break when the next line starts a list
/// item, or when the line before it is much shorter than the widest line in
/// its block (a heading, or the end of a paragraph). Otherwise the lines are
/// joined. A line ending in a hyphen joins without a space.
fn paragraphs(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for block in text
        .split("\n")
        .collect::<Vec<_>>()
        .split(|l| l.trim().is_empty())
    {
        let lines: Vec<String> = block.iter().map(|l| collapse_ws(l)).collect();
        let widest = lines.iter().map(|l| len(l)).max().unwrap_or(0);
        let mut para = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                let prev = &lines[i - 1];
                if starts_list_item(line) || len(prev) * 10 < widest * 6 {
                    out.push(std::mem::take(&mut para));
                } else if !ends_with_hyphenation(prev) {
                    para.push(' ');
                }
            }
            para.push_str(line);
        }
        if !para.is_empty() {
            out.push(para);
        }
    }
    out
}

/// Trim and collapse every run of whitespace to a single space.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn starts_list_item(line: &str) -> bool {
    let mut chars = line.chars();
    match chars.next() {
        Some('-' | '*' | '•' | '·' | '–' | '#') => true,
        Some(c) if c.is_ascii_digit() => {
            let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
            rest.starts_with(". ") || rest.starts_with(") ")
        }
        _ => false,
    }
}

fn ends_with_hyphenation(line: &str) -> bool {
    let mut rev = line.chars().rev();
    rev.next() == Some('-') && rev.next().is_some_and(char::is_alphabetic)
}

/// UAX #29 sentences, with breaks after abbreviations and initials undone.
fn sentences(para: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in para.split_sentence_bounds() {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(prev) if ends_with_abbreviation(prev) => {
                prev.push(' ');
                prev.push_str(seg);
            }
            _ => out.push(seg.to_string()),
        }
    }
    out
}

fn ends_with_abbreviation(s: &str) -> bool {
    let Some(stem) = s.strip_suffix('.') else {
        return false;
    };
    let word = stem
        .rsplit(|c: char| c.is_whitespace() || matches!(c, '(' | '"' | '“' | '\''))
        .next()
        .unwrap_or("")
        .to_lowercase();
    if word.is_empty() {
        return false;
    }
    // Initials ("J.") and dotted forms ("U.S.", "e.g.", "Ph.D.").
    let dotted = word
        .split('.')
        .all(|p| (1..=2).contains(&len(p)) && p.chars().all(char::is_alphabetic));
    dotted && (len(&word) == 1 || word.contains('.')) || ABBREVIATIONS.contains(&word.as_str())
}

/// Split `s` into pieces of at most `max` chars, preferring clause marks.
fn split_long(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while len(rest) > max {
        let head_end = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
        let head = &rest[..head_end];
        let min = rest.char_indices().nth(max / 3).map_or(0, |(i, _)| i);
        let clause = head
            .char_indices()
            .rev()
            .find(|&(i, c)| {
                i >= min
                    && matches!(c, ',' | ';' | ':' | '—' | '–' | ')')
                    && head[i + c.len_utf8()..].starts_with(' ')
            })
            .map(|(i, c)| i + c.len_utf8());
        let space = head
            .char_indices()
            .rev()
            .find(|&(i, c)| i >= min && c == ' ')
            .map(|(i, _)| i);
        let cut = clause.or(space).unwrap_or(head_end);
        out.push(rest[..cut].trim().to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn non_ws(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn first_sentence_stands_alone_then_merges() {
        let c = chunk("One here. Two here. Three here.");
        assert_eq!(c, vec!["One here.", "Two here. Three here."]);
    }

    #[test]
    fn empty_and_blank_input() {
        assert!(chunk("").is_empty());
        assert!(chunk(" \n\t\n ").is_empty());
    }

    #[test]
    fn abbreviations_and_initials_do_not_split() {
        let c = chunk("Hi. Dr. Smith met J. K. Rowling in the U.S. Army, e.g. on Monday. Bye.");
        assert_eq!(
            c,
            vec![
                "Hi.",
                "Dr. Smith met J. K. Rowling in the U.S. Army, e.g. on Monday. Bye."
            ]
        );
    }

    #[test]
    fn decimals_urls_and_ellipses_stay_whole() {
        let c = chunk("Start. Pi is 3.14 and see example.com/a.b for more. Wait... what.");
        assert_eq!(
            c,
            vec![
                "Start.",
                "Pi is 3.14 and see example.com/a.b for more. Wait... what."
            ]
        );
    }

    #[test]
    fn pdf_hard_wraps_are_unwrapped() {
        let pdf = "The committee reviewed the proposal in\n\
                   detail and found that the costs were well-\n\
                   known to every member of the panel before\n\
                   the vote. It passed.";
        let c = chunk(pdf);
        assert_eq!(
            c,
            vec![
                "The committee reviewed the proposal in detail and found that the costs \
                 were well-known to every member of the panel before the vote.",
                "It passed."
            ]
        );
    }

    #[test]
    fn short_heading_line_is_its_own_chunk() {
        let c = chunk("Introduction\nThis study looks at sleep in sixty adults over a year.");
        assert_eq!(
            c,
            vec![
                "Introduction",
                "This study looks at sleep in sixty adults over a year."
            ]
        );
    }

    #[test]
    fn paragraphs_and_list_items_never_merge() {
        let md = "# Setup\n\nInstall it. Then run it.\n- first item\n- second item\n1. step one\r\n2) step two";
        let c = chunk(md);
        assert_eq!(
            c,
            vec![
                "# Setup",
                "Install it. Then run it.",
                "- first item",
                "- second item",
                "1. step one",
                "2) step two"
            ]
        );
    }

    #[test]
    fn long_sentence_splits_at_clause_marks() {
        let clause = "the quick brown fox jumps over the lazy dog";
        let s = format!("Start. {}.", vec![clause; 20].join(", "));
        let c = chunk(&s);
        assert!(c.len() > 2, "{c:?}");
        for piece in &c[1..c.len() - 1] {
            assert!(
                piece.ends_with(','),
                "should end at a clause mark: {piece:?}"
            );
            assert!(len(piece) <= MAX_CHARS);
        }
    }

    #[test]
    fn unbroken_text_is_hard_cut() {
        let s = "x".repeat(MAX_CHARS * 2 + 5);
        let c = chunk(&s);
        assert_eq!(
            c.iter().map(|p| len(p)).collect::<Vec<_>>(),
            [MAX_CHARS, MAX_CHARS, 5]
        );
    }

    #[test]
    fn multibyte_text_is_cut_on_char_boundaries() {
        let s = "中文".repeat(MAX_CHARS);
        let c = chunk(&s);
        assert!(c.iter().all(|p| len(p) <= MAX_CHARS));
        assert_eq!(non_ws(&c.concat()), s);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn keeps_every_character_in_order(s in "[a-zA-Z0-9 .,;:!?'\"()\\-\n\r\t•é中😀]{0,1500}") {
            let c = chunk(&s);
            prop_assert_eq!(non_ws(&c.concat()), non_ws(&s));
        }

        #[test]
        fn chunks_are_trimmed_non_empty_and_bounded(s in "[a-zA-Z ,;-]{0,3000}") {
            for piece in chunk(&s) {
                prop_assert!(!piece.is_empty());
                prop_assert_eq!(piece.trim(), piece.as_str());
                prop_assert!(len(&piece) <= MAX_CHARS, "{} chars", len(&piece));
            }
        }
    }
}
