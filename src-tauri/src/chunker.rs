//! Splits selected text into sentence-sized chunks for synthesis.
//!
//! Chunks never cross a paragraph break. The first chunk is a single sentence,
//! cut at a clause mark if it is longer than [`FIRST_CHARS`], so audio starts
//! quickly. Chunks then ramp up: each is at most [`RAMP`] times the one before,
//! so it is ready before that one finishes playing. While ramping, a sentence
//! that is too long is cut at a clause mark, never mid-phrase. Once at
//! [`TARGET_CHARS`], sentences in a paragraph are merged up to it for fewer
//! requests and steadier intonation. Anything longer than [`MAX_CHARS`] is
//! split at the last clause mark or space that fits.
//!
//! No text is dropped or reordered: only whitespace changes.

use unicode_segmentation::UnicodeSegmentation;

/// Sentences in a paragraph are merged until a chunk would exceed this.
pub const TARGET_CHARS: usize = 220;
/// A first sentence longer than this is cut at a clause mark. Kokoro takes
/// about 1 s to synthesise it on a fast CPU.
pub const FIRST_CHARS: usize = 100;
/// Each chunk is at most this many times the length of the one before. Kokoro
/// synthesises about 5 times faster than real time on a fast CPU, so 3 leaves
/// room for 1.5x speed.
pub const RAMP: usize = 3;
/// The ramp never asks for less: shorter input sounds worse and saves little.
pub const MIN_CHARS: usize = 40;
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
            for mut piece in split_long(&sentence, MAX_CHARS) {
                loop {
                    let ramp = ramp(&chunks);
                    let limit = ramp.min(TARGET_CHARS);
                    let fits = len(&current) + 1 + len(&piece) <= limit;
                    // The very first sentence always stands alone.
                    if !current.is_empty() && (chunks.is_empty() || !fits) {
                        chunks.push(std::mem::take(&mut current));
                        continue;
                    }
                    // Cut a sentence too long for this step of the ramp.
                    if current.is_empty() && len(&piece) > ramp {
                        if let Some(at) = clause_cut(&piece, limit) {
                            chunks.push(piece[..at].to_string());
                            piece = piece[at..].trim_start().to_string();
                            continue;
                        }
                    }
                    if !current.is_empty() {
                        current.push(' ');
                    }
                    current.push_str(&piece);
                    break;
                }
            }
        }
        if !current.is_empty() {
            chunks.push(current);
        }
    }
    chunks
}

/// How long the next chunk may be, given the chunks so far, before the cap
/// of [`TARGET_CHARS`]. Past [`MAX_CHARS`] the ramp is over: no sentence
/// piece is that long, so none is cut.
fn ramp(chunks: &[String]) -> usize {
    match chunks.last() {
        None => FIRST_CHARS,
        Some(prev) => (RAMP * len(prev)).max(MIN_CHARS),
    }
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
        let (head, min) = window(rest, max);
        let space = head
            .char_indices()
            .rev()
            .find(|&(i, c)| i >= min && c == ' ')
            .map(|(i, _)| i);
        let cut = clause_cut(rest, max).or(space).unwrap_or(head.len());
        out.push(rest[..cut].trim().to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// Where to cut `s` to keep at most `max` chars: just after the last clause
/// mark that fits and is followed by a space. None if there isn't one past
/// the first third.
fn clause_cut(s: &str, max: usize) -> Option<usize> {
    let (head, min) = window(s, max);
    head.char_indices()
        .rev()
        .find(|&(i, c)| {
            i >= min
                && matches!(c, ',' | ';' | ':' | '—' | '–' | ')')
                && s[i + c.len_utf8()..].starts_with(' ')
        })
        .map(|(i, c)| i + c.len_utf8())
}

/// The first `max` chars of `s`, and the byte index a third of the way in.
fn window(s: &str, max: usize) -> (&str, usize) {
    let end = s.char_indices().nth(max).map_or(s.len(), |(i, _)| i);
    let min = s.char_indices().nth(max / 3).map_or(0, |(i, _)| i);
    (&s[..end], min)
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
    fn a_long_first_sentence_is_cut_at_a_clause_mark() {
        let c = chunk(
            "When the committee finally met on Tuesday afternoon to review the draft \
             budget for the coming year, after weeks of delays caused by missing \
             figures, the mood was tense but hopeful. The chair thanked everyone.",
        );
        assert_eq!(
            c,
            vec![
                "When the committee finally met on Tuesday afternoon to review the draft \
                 budget for the coming year,",
                "after weeks of delays caused by missing figures, the mood was tense but \
                 hopeful. The chair thanked everyone."
            ]
        );
    }

    #[test]
    fn chunks_ramp_up_from_a_short_start() {
        let clause = "and the fox ran on";
        let s = format!("Go. Then {}.", [clause; 20].join(", "));
        let lens: Vec<usize> = chunk(&s).iter().map(|c| len(c)).collect();
        assert_eq!(lens[0], 3);
        for pair in lens.windows(2) {
            let allowed = (RAMP * pair[0]).clamp(MIN_CHARS, TARGET_CHARS);
            assert!(pair[1] <= allowed, "{lens:?}");
        }
        assert!(lens.iter().any(|&n| n > 150), "reaches full size: {lens:?}");
    }

    #[test]
    fn the_ramp_holds_after_reaching_full_size() {
        let first = "This first sentence is long enough that three times it is over the target.";
        let rest = ["the quick brown fox jumps over the lazy dog"; 7].join(", ");
        let lens: Vec<usize> = chunk(&format!("{first} And {rest}."))
            .iter()
            .map(|c| len(c))
            .collect();
        assert_eq!(lens[0], len(first));
        assert!(lens[1] <= TARGET_CHARS, "{lens:?}");
    }

    #[test]
    fn a_clause_mark_right_at_the_limit_is_used() {
        let s = format!("{}, {}.", "a".repeat(FIRST_CHARS - 1), "b".repeat(50));
        let c = chunk(&s);
        assert_eq!(len(&c[0]), FIRST_CHARS);
        assert!(c[0].ends_with(','));
    }

    #[test]
    fn the_ramp_never_cuts_mid_phrase() {
        let long = "This study looks at how the timing of morning light changes the \
                    sleep of sixty adults followed over one whole year.";
        let c = chunk(&format!("Introduction\n{long}"));
        assert_eq!(c, vec!["Introduction", long]);
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
                // Already over the ramp's limit, so "Bye." doesn't join it.
                "Dr. Smith met J. K. Rowling in the U.S. Army, e.g. on Monday.",
                "Bye."
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
                "Pi is 3.14 and see example.com/a.b for more.",
                "Wait... what."
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
