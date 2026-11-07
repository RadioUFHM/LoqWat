//! Text formatting applied to every transcript before it is inserted.
//!
//! 1. Spoken punctuation: "comma", "colon", "semicolon", "question mark",
//!    "exclamation point/mark" become the mark. Whisper often writes the word
//!    *and* adds its own mark ("Hello, comma, world"); its mark is dropped.
//!    "period" is also an everyday word ("lookback period"), so it only becomes
//!    "." where a sentence ends: at the end of the dictation, where whisper put
//!    a sentence break after it, or before a capitalized word.
//! 2. Numbers (user's style rule, 2026-11-07): one through nine are spelled
//!    out, 10 and up are digits with thousands separators. Digits are kept for
//!    money, percentages, decimals, times, dates and labels ("Rule 3"); 1st–9th
//!    become first–ninth; 4-digit years 1900–2099 and ID-like numbers
//!    ("Form 8300") get no commas.
//!
//! Only whole tokens are touched; anything that doesn't match a rule is left
//! exactly as whisper wrote it.

const LEAD: &[char] = &['"', '\'', '(', '“', '‘'];
const TRAIL: &[char] = &['.', ',', ';', ':', '!', '?', '"', '\'', ')', '”', '’'];
const MARKS: &[char] = &['.', ',', ';', ':', '!', '?'];
const SENTENCE_END: &[char] = &['.', '?', '!'];

const SMALL: [&str; 9] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
];
const ORDINALS: [&str; 9] = [
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
];

const MONTHS: &[&str] = &[
    "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep",
    "sept", "oct", "nov", "dec",
];

/// A number right after one of these is a label or identifier: keep its digits
/// and don't add thousands separators.
const LABELS: &[&str] = &[
    // references
    "rule", "rules", "section", "sections", "page", "pages", "tier", "tiers", "chapter",
    "step", "steps", "level", "phase", "version", "item", "items", "figure", "fig", "table",
    "part", "article", "appendix", "exhibit", "paragraph", "clause", "schedule", "volume", "vol",
    "question", "milestone", "category", "class", "grade", "option", "stage", "round", "slide",
    "module", "lesson", "priority", "severity", "sev", "type", "group", "team", "zone",
    "region", "district", "season", "episode", "week", "day", "quarter", "line", "number",
    "no", "#",
    // identifiers
    "form", "account", "acct", "case", "id", "ticket", "invoice", "zip", "reference", "ref",
    "routing", "code", "extension", "ext", "box", "suite", "unit", "room", "floor", "route",
    "highway", "flight", "model", "serial", "order", "claim", "policy", "file", "control",
    "check", "cheque", "loan", "member", "customer", "transaction", "alert",
];

/// A number right before one of these is money, a percentage or a time: keep the digit.
const KEEP_DIGIT_BEFORE: &[&str] = &[
    "percent", "percentage", "pct", "dollar", "dollars", "cent", "cents", "thousand",
    "million", "billion", "trillion", "a.m", "p.m", "am", "pm",
];

pub fn format_transcript(text: &str, vocabulary: &[String]) -> String {
    let words: Vec<String> = text.split_whitespace().map(String::from).collect();
    let words = spoken_punctuation(&words, vocabulary);
    format_numbers(&words).join(" ")
}

/// Split a token into (leading quotes/brackets, core, trailing punctuation).
fn split(tok: &str) -> (&str, &str, &str) {
    let core_start = tok.len() - tok.trim_start_matches(LEAD).len();
    let rest = &tok[core_start..];
    let core_len = rest.trim_end_matches(TRAIL).len();
    (&tok[..core_start], &rest[..core_len], &rest[core_len..])
}

fn lower_core(tok: &str) -> String {
    split(tok).1.to_lowercase()
}

fn is_i_form(core: &str) -> bool {
    core == "I" || core.starts_with("I'") || core.starts_with("I’")
}

// ---------------------------------------------------------------- punctuation

/// Spoken punctuation command starting at `i`: (mark, tokens used).
fn command_at(words: &[String], i: usize) -> Option<(char, usize)> {
    let first = lower_core(words.get(i)?);
    let second = words.get(i + 1).map(|t| lower_core(t)).unwrap_or_default();
    match (first.as_str(), second.as_str()) {
        ("question", "mark") => Some(('?', 2)),
        ("exclamation", "point" | "mark") => Some(('!', 2)),
        ("comma", _) => Some((',', 1)),
        ("colon", _) => Some((':', 1)),
        ("semicolon" | "semi-colon", _) => Some((';', 1)),
        ("period", _) => Some(('.', 1)),
        _ => None,
    }
}

fn starts_like_sentence(tok: &str) -> bool {
    let core = split(tok).1;
    core.chars().next().is_some_and(char::is_uppercase) && !is_i_form(core)
}

enum NextCase {
    Keep,
    Upper,
    Lower,
}

fn spoken_punctuation(words: &[String], vocabulary: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut next_case = NextCase::Keep;
    let mut i = 0;
    while i < words.len() {
        if let Some((mark, n)) = command_at(words, i) {
            let after = i + n;
            let whisper_ended = split(&words[after - 1]).2.contains(SENTENCE_END);
            let is_command = mark != '.'
                || whisper_ended
                || after == words.len()
                || starts_like_sentence(&words[after]);
            if is_command {
                attach(&mut out, mark);
                next_case = if SENTENCE_END.contains(&mark) {
                    NextCase::Upper
                } else if whisper_ended {
                    // Whisper broke the sentence at the spoken word ("comma. World");
                    // the user only asked for a comma.
                    NextCase::Lower
                } else {
                    NextCase::Keep
                };
                i = after;
                continue;
            }
        }
        let mut word = words[i].clone();
        match next_case {
            NextCase::Upper => recase_first(&mut word, true),
            NextCase::Lower if can_lowercase(&word, vocabulary) => recase_first(&mut word, false),
            _ => {}
        }
        next_case = NextCase::Keep;
        out.push(word);
        i += 1;
    }
    out
}

/// Put `mark` on the previous word, replacing any mark whisper added there.
fn attach(out: &mut Vec<String>, mark: char) {
    let Some(prev) = out.last_mut() else {
        out.push(mark.to_string());
        return;
    };
    let kept = prev.trim_end_matches(MARKS).len();
    // Keep an abbreviation's own period: "U.S." + comma → "U.S.,".
    let keep_dot = prev[..kept].contains('.') && prev[kept..].starts_with('.');
    prev.truncate(kept);
    if keep_dot {
        prev.push('.');
    }
    if !(keep_dot && mark == '.') {
        prev.push(mark);
    }
}

fn recase_first(word: &mut String, upper: bool) {
    let lead = word.len() - word.trim_start_matches(LEAD).len();
    if let Some(c) = word[lead..].chars().next() {
        let r: String = if upper {
            c.to_uppercase().collect()
        } else {
            c.to_lowercase().collect()
        };
        word.replace_range(lead..lead + c.len_utf8(), &r);
    }
}

/// Only undo whisper's sentence-start capital on a plain Titlecase word, never
/// on "I", acronyms, or vocabulary terms (likely proper nouns).
fn can_lowercase(word: &str, vocabulary: &[String]) -> bool {
    let core = split(word).1;
    let mut chars = core.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_uppercase() || chars.any(char::is_uppercase) || is_i_form(core) {
        return false;
    }
    !vocabulary
        .iter()
        .any(|term| term.split_whitespace().any(|v| v == core))
}

// -------------------------------------------------------------------- numbers

fn format_numbers(words: &[String]) -> Vec<String> {
    let lower: Vec<String> = words.iter().map(|w| lower_core(w)).collect();
    let at = |k: usize| lower.get(k).map(String::as_str).unwrap_or("");
    words
        .iter()
        .enumerate()
        .map(|(i, tok)| {
            let (lead, core, trail) = split(tok);
            let prev = if i > 0 { at(i - 1) } else { "" };
            let raw_prev = if i > 0 { words[i - 1].as_str() } else { "" };
            let labelled = LABELS.contains(&prev) || raw_prev.ends_with('#');
            let dated =
                MONTHS.contains(&prev) || (at(i + 1) == "of" && MONTHS.contains(&at(i + 2)));
            let keep_small = labelled || dated || KEEP_DIGIT_BEFORE.contains(&at(i + 1));
            let Some(mut new_core) = convert_number(core, keep_small, labelled) else {
                return tok.clone();
            };
            let sentence_start = i == 0 || {
                let (_, prev_core, prev_trail) = split(raw_prev);
                prev_trail.contains(SENTENCE_END) && !prev_core.contains('.')
            };
            if sentence_start {
                recase_first(&mut new_core, true);
            }
            format!("{lead}{new_core}{trail}")
        })
        .collect()
}

fn small_value(digits: &str) -> Option<usize> {
    match digits.as_bytes() {
        [d @ b'1'..=b'9'] => Some((d - b'0') as usize),
        _ => None,
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// New text for a number token, or None to leave it alone.
fn convert_number(core: &str, keep_small: bool, id_like: bool) -> Option<String> {
    // Ordinals: 1st–9th.
    let lc = core.to_ascii_lowercase();
    for suffix in ["st", "nd", "rd", "th"] {
        if let Some(n) = lc.strip_suffix(suffix).and_then(small_value) {
            return (!keep_small).then(|| ORDINALS[n - 1].to_string());
        }
    }
    // Compounds: "3-day" → "three-day".
    if let Some((num, rest)) = core.split_once('-') {
        let n = small_value(num)?;
        let wordy = rest.chars().next().is_some_and(char::is_alphabetic);
        return (wordy && !keep_small).then(|| format!("{}-{rest}", SMALL[n - 1]));
    }
    // Plain numbers, money, decimals.
    let (dollar, body) = match core.strip_prefix('$') {
        Some(b) => ("$", b),
        None => ("", core),
    };
    let (int, frac) = match body.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (body, None),
    };
    if !all_digits(int) || frac.is_some_and(|f| !all_digits(f)) {
        return None;
    }
    if dollar.is_empty() && frac.is_none() {
        if let Some(n) = small_value(int) {
            return (!keep_small).then(|| SMALL[n - 1].to_string());
        }
        if id_like {
            return None;
        }
        if int.len() == 4 && (1900..=2099).contains(&int.parse::<u32>().ok()?) {
            return None;
        }
    }
    if int.len() < 4 || int.starts_with('0') {
        return None;
    }
    let frac = frac.map(|f| format!(".{f}")).unwrap_or_default();
    Some(format!("{dollar}{}{frac}", group_thousands(int)))
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::format_transcript;

    fn f(s: &str) -> String {
        format_transcript(s, &["FinCEN".to_string(), "FCI Tradecraft".to_string()])
    }

    #[test]
    fn spoken_comma_replaces_whisper_mark() {
        assert_eq!(f("Hello, comma, world."), "Hello, world.");
        assert_eq!(f("Hello comma world"), "Hello, world");
        assert_eq!(f("Hello comma. World is big."), "Hello, world is big.");
    }

    #[test]
    fn sentence_marks_and_capitals() {
        assert_eq!(f("Is that right question mark"), "Is that right?");
        assert_eq!(
            f("Stop exclamation point that is it period."),
            "Stop! That is it."
        );
        assert_eq!(f("Note colon file today."), "Note: file today.");
        assert_eq!(f("One semicolon two."), "One; two.");
    }

    #[test]
    fn period_only_at_sentence_end() {
        assert_eq!(
            f("The lookback period ends Friday period."),
            "The lookback period ends Friday."
        );
        assert_eq!(f("Done period New topic."), "Done. New topic.");
        assert_eq!(f("Done period."), "Done.");
        // Saying "period period" keeps the word and adds the mark.
        assert_eq!(f("The lookback period period."), "The lookback period.");
    }

    #[test]
    fn proper_nouns_keep_capitals_after_comma() {
        assert_eq!(f("Thanks comma. FinCEN agreed."), "Thanks, FinCEN agreed.");
        assert_eq!(f("Yes comma. I agree."), "Yes, I agree.");
    }

    #[test]
    fn abbreviation_keeps_its_period() {
        assert_eq!(f("In the U.S. comma banks"), "In the U.S., banks");
    }

    #[test]
    fn small_numbers_spelled_out() {
        assert_eq!(f("We filed 3 SARs and 14 CTRs."), "We filed three SARs and 14 CTRs.");
        assert_eq!(f("3 analysts reviewed it."), "Three analysts reviewed it.");
        assert_eq!(f("Done. 2 more to go."), "Done. Two more to go.");
        assert_eq!(f("a 3-day window, a 30-day lookback"), "a three-day window, a 30-day lookback");
        assert_eq!(f("the 1st and 3rd alerts, the 10th"), "the first and third alerts, the 10th");
    }

    #[test]
    fn digits_kept_where_chosen() {
        assert_eq!(f("$5 and 5% and 5 percent"), "$5 and 5% and 5 percent");
        assert_eq!(f("$4.7 million, 3.5 hours at 9:30 on 4/12"), "$4.7 million, 3.5 hours at 9:30 on 4/12");
        assert_eq!(f("Rule 3, Section 5, page 2, Tier 1, #4"), "Rule 3, Section 5, page 2, Tier 1, #4");
        assert_eq!(f("on May 5 at 5 p.m."), "on May 5 at 5 p.m.");
        assert_eq!(f("May 1st, the 2nd of June"), "May 1st, the 2nd of June");
        assert_eq!(f("0 alerts"), "0 alerts");
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(f("1200 alerts and 45000 accounts."), "1,200 alerts and 45,000 accounts.");
        assert_eq!(f("$1200.50 and 1234567"), "$1,200.50 and 1,234,567");
        assert_eq!(f("already 1,200"), "already 1,200");
    }

    #[test]
    fn years_and_ids_have_no_commas() {
        assert_eq!(f("In 2026 and 1999."), "In 2026 and 1999.");
        assert_eq!(f("Form 8300, case 123456"), "Form 8300, case 123456");
        assert_eq!(f("0123 stays"), "0123 stays");
    }

    #[test]
    fn plain_text_untouched() {
        let s = "FinCEN filed the report; nothing else changed.";
        assert_eq!(f(s), s);
        assert_eq!(f(""), "");
    }
}
