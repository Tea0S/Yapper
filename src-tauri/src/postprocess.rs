use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

/// Dictation-style spoken punctuation → real characters (runs before dictionary / corrections).
fn spoken_punct_rules() -> &'static [(Regex, &'static str)] {
    static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        let mut pairs: Vec<(&'static str, &'static str)> = vec![
            ("exclamation point", "!"),
            ("exclamation mark", "!"),
            ("question mark", "?"),
            ("inverted question mark", "¿"),
            ("inverted exclamation mark", "¡"),
            ("open quotes", "\""),
            ("close quotes", "\""),
            ("open quote", "\""),
            ("close quote", "\""),
            ("begin quote", "\""),
            ("end quote", "\""),
            ("new paragraph", "\n\n"),
            ("new line", "\n\n"),
            // Whisper often prints this as one word or hyphenated.
            ("line break", "\n\n"),
            ("new-line", "\n\n"),
            ("newline", "\n\n"),
            ("full stop", "."),
            ("semicolon", ";"),
            ("ellipsis", "…"),
            ("dot dot dot", "…"),
            ("em dash", "—"),
            ("en dash", "–"),
            ("open parenthesis", "("),
            ("close parenthesis", ")"),
            ("open paren", "("),
            ("close paren", ")"),
            ("left parenthesis", "("),
            ("right parenthesis", ")"),
            ("open bracket", "["),
            ("close bracket", "]"),
            ("left bracket", "["),
            ("right bracket", "]"),
            ("open brace", "{"),
            ("close brace", "}"),
            ("left brace", "{"),
            ("right brace", "}"),
            ("ampersand", "&"),
            ("at sign", "@"),
            ("hash tag", "#"),
            ("hashtag", "#"),
            ("hash sign", "#"),
            ("percent sign", "%"),
            ("dollar sign", "$"),
            ("plus sign", "+"),
            ("equal sign", "="),
            ("equals sign", "="),
            ("slash", "/"),
            ("forward slash", "/"),
            ("backslash", "\\"),
            ("asterisk", "*"),
            ("underscore", "_"),
            ("pipe", "|"),
            ("tilde", "~"),
            ("caret", "^"),
            ("period", "."),
            ("comma", ","),
            ("colon", ":"),
            ("hyphen", "-"),
        ];
        pairs.sort_by_key(|(phrase, _)| std::cmp::Reverse(phrase.len()));
        pairs
            .into_iter()
            .filter_map(|(phrase, rep)| {
                let pat = format!(r"(?i)\b{}\b", regex::escape(phrase));
                Regex::new(&pat).ok().map(|re| (re, rep))
            })
            .collect()
    });
    RULES.as_slice()
}

/// Private-use characters interpreted by `paste.rs` as key clicks (not pasted as text).
/// Keep codepoints in sync with `paste::KEY_SENTINEL_*`.
pub(crate) const KEY_SENTINEL_ENTER: &str = "\u{E090}";
pub(crate) const KEY_SENTINEL_CAPS_LOCK: &str = "\u{E091}";
pub(crate) const KEY_SENTINEL_TAB: &str = "\u{E092}";
pub(crate) const KEY_SENTINEL_ESCAPE: &str = "\u{E093}";
pub(crate) const KEY_SENTINEL_BACKSPACE: &str = "\u{E094}";

/// Spoken key commands → sentinels (`\s+` so ASR spacing variants still match).
fn spoken_key_rules() -> &'static [(Regex, &'static str)] {
    static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        let mut pairs: Vec<(&'static str, &'static str)> = vec![
            (r"(?i)\bpress\s+caps\s+lock\b", KEY_SENTINEL_CAPS_LOCK),
            (r"(?i)\bhit\s+caps\s+lock\b", KEY_SENTINEL_CAPS_LOCK),
            (r"(?i)\bpress\s+capslock\b", KEY_SENTINEL_CAPS_LOCK),
            (r"(?i)\bhit\s+capslock\b", KEY_SENTINEL_CAPS_LOCK),
            (r"(?i)\bpress\s+backspace\b", KEY_SENTINEL_BACKSPACE),
            (r"(?i)\bhit\s+backspace\b", KEY_SENTINEL_BACKSPACE),
            (r"(?i)\bpress\s+escape\b", KEY_SENTINEL_ESCAPE),
            (r"(?i)\bhit\s+escape\b", KEY_SENTINEL_ESCAPE),
            (r"(?i)\bpress\s+esc\b", KEY_SENTINEL_ESCAPE),
            (r"(?i)\bhit\s+esc\b", KEY_SENTINEL_ESCAPE),
            (r"(?i)\bpress\s+return\b", KEY_SENTINEL_ENTER),
            (r"(?i)\bhit\s+return\b", KEY_SENTINEL_ENTER),
            (r"(?i)\bpress\s+enter\b", KEY_SENTINEL_ENTER),
            (r"(?i)\bhit\s+enter\b", KEY_SENTINEL_ENTER),
            (r"(?i)\bpress\s+tab\b", KEY_SENTINEL_TAB),
            (r"(?i)\bhit\s+tab\b", KEY_SENTINEL_TAB),
        ];
        pairs.sort_by_key(|(pat, _)| std::cmp::Reverse(pat.len()));
        pairs
            .into_iter()
            .filter_map(|(pat, rep)| Regex::new(pat).ok().map(|re| (re, rep)))
            .collect()
    });
    RULES.as_slice()
}

pub fn apply_spoken_punctuation(text: &str) -> String {
    let mut s = text.to_string();
    for (re, rep) in spoken_punct_rules() {
        s = re.replace_all(&s, *rep).into_owned();
    }
    s = normalize_after_spoken_punct(&s);
    for (re, rep) in spoken_key_rules() {
        s = re.replace_all(&s, *rep).into_owned();
    }
    repair_asr_punctuation(&s)
}

/// Collapse `,,`, `, ,`, etc. (Whisper comma + spoken "comma", or ASR stutter).
fn collapse_comma_runs(s: &str) -> String {
    let Ok(re) = Regex::new(r",\s*(?:,\s*)+") else {
        return s.to_string();
    };
    let mut out = s.to_string();
    for _ in 0..16 {
        let n = re.replace_all(&out, ", ").into_owned();
        if n == out {
            break;
        }
        out = n;
    }
    out
}

/// Tighten typical ASR spacing around punctuation after phrase replacement.
fn normalize_after_spoken_punct(s: &str) -> String {
    let mut out = collapse_comma_runs(s);
    // Sentence end: keep one space after so the next word isn't glued (e.g. "Hello. how").
    // Do not use `\S` here: `"` counts as non-whitespace, so `" . word"` would wrongly become
    // `"." word"` after open-quote / close-quote commands (same for comma before attribution).
    let Ok(re_sent) = Regex::new(r#"([^\s"])\s+([.!?])\s*"#) else {
        return out;
    };
    out = re_sent.replace_all(&out, "$1$2 ").into_owned();
    // Do not anchor on comma: `, ,` would match `(\S)\s+,\s*` with \S = comma and corrupt the text.
    let Ok(re_comma) = Regex::new(r"([^\s,])\s+,\s*") else {
        return out;
    };
    out = re_comma.replace_all(&out, "$1, ").into_owned();
    let Ok(re_semi) = Regex::new(r"(\S)\s+;\s*") else {
        return out;
    };
    out = re_semi.replace_all(&out, "$1; ").into_owned();
    let Ok(re_colon) = Regex::new(r"(\S)\s+:\s*") else {
        return out;
    };
    out = re_colon.replace_all(&out, "$1: ").into_owned();
    // Tighten `" word"` → `"word"`, but do not eat the space before spoken `.` / `!` / `?`
    // (otherwise `" . Oh"` becomes `". Oh"`).
    let Ok(re_quote_open) = Regex::new(r#""\s+(?![.!?])"#) else {
        return out;
    };
    out = re_quote_open.replace_all(&out, "\"").into_owned();
    let Ok(re_quote_close) = Regex::new(r#"\s+""#) else {
        return out;
    };
    out = re_quote_close.replace_all(&out, "\"").into_owned();
    let Ok(re_spaces) = Regex::new(r"[ \t\f\v]{2,}") else {
        return out;
    };
    out = re_spaces.replace_all(&out, " ").into_owned();
    collapse_comma_runs(&out)
}

/// Fix collisions when spoken punctuation and Whisper both insert marks (e.g. `leave,", Kane`, `.. .`).
pub(crate) fn repair_asr_punctuation(s: &str) -> String {
    let mut out = collapse_comma_runs(s);
    // Whisper sometimes prepends sentence punctuation before a spoken punctuation token
    // at utterance start: `. (` -> `(`, `. ,` -> `,`.
    // Quote is special: `. " Oh"` must become `"Oh …"` not `" Oh …"` (stray space after `"`).
    if let Ok(re) = Regex::new(r#"(?m)^\s*[.?!;:]+\s+" ?"#) {
        out = re.replace_all(&out, "\"").into_owned();
    }
    if let Ok(re) = Regex::new(r#"(?m)^\s*[.?!;:]+\s+(,|\(|\[|\{|¿|¡|\.|!|\?|;|:)"#) {
        out = re.replace_all(&out, "${1}").into_owned();
    }
    // leave,", Kane said → leave," Kane said (comma belongs inside the closing quote for attribution)
    if let Ok(re) = Regex::new(r#"([A-Za-z']+),\s*"\s*,\s+([A-Z][a-z]*)"#) {
        out = re.replace_all(&out, "${1},\" ${2}").into_owned();
    }
    // Same after ? or ! before dialogue tag
    if let Ok(re) = Regex::new(r#"([?!]),\s*"\s*,\s+([A-Z][a-z]*)"#) {
        out = re.replace_all(&out, "${1},\" ${2}").into_owned();
    }
    // Stuttered periods / spaced dots (Whisper + spoken "period"): vampire.. . → vampire.
    if let Ok(re) = Regex::new(r"\.(?:\s*\.)+") {
        out = re.replace_all(&out, ".").into_owned();
    }
    // Optional space between two periods that survived: . . → .
    if let Ok(re) = Regex::new(r"\.\s+\.") {
        out = re.replace_all(&out, ".").into_owned();
    }
    // Whisper often ends questions with ? while spoken "question mark" adds another; normalize
    // also pulls `? ?` → `??`. Same pattern for exclamation.
    if let Ok(re) = Regex::new(r"\?(?:\s*\?)+") {
        out = re.replace_all(&out, "?").into_owned();
    }
    if let Ok(re) = Regex::new(r"\!(?:\s*\!)+") {
        out = re.replace_all(&out, "!").into_owned();
    }
    // Period glued to comma (often duplicate sentence boundary): ., → .
    if let Ok(re) = Regex::new(r"\.,\s*") {
        out = re.replace_all(&out, ". ").into_owned();
    }
    // Comma then period with only space: , . → .
    if let Ok(re) = Regex::new(r",\s+\.") {
        out = re.replace_all(&out, ".").into_owned();
    }
    // Whisper outputs closing dialogue as …," then a duplicate spoken "period" becomes space-dot
    // before the tag: `…to," . Vivian` → `…to," Vivian`. Same when the quote ends with a period
    // inside: `"Stop." . She` → `"Stop." She`.
    if let Ok(re) = Regex::new(r#"(?i)(,|\.)\s*"\s+\.\s+([A-Za-z])"#) {
        out = re.replace_all(&out, "${1}\" ${2}").into_owned();
    }
    // Pause after spoken "close quotes": ASR often puts comma/period *outside* the closing mark.
    // `Hi" , she` → `Hi," she`. `\D` before `"` skips `6" ,` (inches).
    if let Ok(re) = Regex::new(r#"(\D)"\s+,\s+"#) {
        out = re.replace_all(&out, "${1},\" ").into_owned();
    }
    if let Ok(re) = Regex::new(r#"(\D)"\s+\.\s+([A-Za-z])"#) {
        out = re.replace_all(&out, "${1}\" ${2}").into_owned();
    }
    collapse_comma_runs(&out)
}

#[derive(Debug, Deserialize)]
struct ToneFile {
    name: String,
    rules: Vec<ToneRule>,
}

#[derive(Debug, Deserialize)]
struct ToneRule {
    pattern: String,
    replace: String,
}

pub fn apply_corrections(text: &str, pairs: &[(String, String, i64)]) -> String {
    let mut out = text.to_string();
    for (from, to, _) in pairs {
        if from.is_empty() {
            continue;
        }
        out = out.replace(from, to);
    }
    out
}

pub fn apply_dictionary(
    text: &str,
    entries: &[(String, String, String, i64)],
) -> String {
    let mut out = text.to_string();
    for (term, replacement, scope, _) in entries {
        if term.is_empty() {
            continue;
        }
        match scope.as_str() {
            "phrase" => {
                out = out.replace(term, replacement);
            }
            _ => {
                let pat = format!(r"(?i)\b{}\b", regex::escape(term));
                if let Ok(re) = Regex::new(&pat) {
                    out = re.replace_all(&out, replacement.as_str()).into_owned();
                }
            }
        }
    }
    out
}

pub fn apply_tone(text: &str, preset: &str, tone_dir: &std::path::Path) -> String {
    let path = tone_dir.join(format!("{preset}.yaml"));
    if !path.exists() {
        return text.to_string();
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return text.to_string(),
    };
    let tone: ToneFile = match serde_yaml::from_str(&raw) {
        Ok(t) => t,
        Err(_) => return text.to_string(),
    };
    let mut out = text.to_string();
    drop(tone.name);
    for rule in tone.rules {
        if let Ok(re) = Regex::new(&rule.pattern) {
            out = re.replace_all(&out, rule.replace.as_str()).into_owned();
        }
    }
    out
}

pub fn pipeline(
    text: &str,
    corrections: &[(String, String, i64)],
    dictionary: &[(String, String, String, i64)],
    tone_preset: &str,
    tone_dir: &std::path::Path,
) -> String {
    let mut s = text.to_string();
    s = apply_spoken_punctuation(&s);
    s = apply_corrections(&s, corrections);
    s = apply_dictionary(&s, dictionary);
    s = apply_tone(&s, tone_preset, tone_dir);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn corrections_order() {
        let pairs = vec![
            ("foo bar".to_string(), "baz".to_string(), 0),
            ("foo".to_string(), "x".to_string(), 0),
        ];
        let out = apply_corrections("foo bar", &pairs);
        assert_eq!(out, "baz");
    }

    #[test]
    fn dictionary_word_boundary() {
        let e = vec![("cat".to_string(), "dog".to_string(), "word".to_string(), 0)];
        assert_eq!(apply_dictionary("The cat sat.", &e), "The dog sat.");
        assert_eq!(apply_dictionary("The catfish.", &e), "The catfish.");
    }

    #[test]
    fn tone_from_temp_yaml() {
        let dir = std::env::temp_dir().join("yapper_tone_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join("minimal.yaml")).unwrap();
        writeln!(
            f,
            r"name: Minimal
rules:
  - pattern: '!'
    replace: '.'
"
        )
        .unwrap();
        assert_eq!(apply_tone("Hi!", "minimal", &dir), "Hi.");
    }

    #[test]
    fn spoken_punct_period_comma_quotes() {
        assert_eq!(
            apply_spoken_punctuation("Hello period how are you comma fine"),
            "Hello. how are you, fine"
        );
        let o = apply_spoken_punctuation("He said open quotes hello close quotes");
        assert!(o.contains('"'));
        assert!(o.contains("hello"));
    }

    #[test]
    fn spoken_punct_new_paragraph() {
        for phrase in [
            "new paragraph",
            "new line",
            "line break",
            "new-line",
            "newline",
        ] {
            let o = apply_spoken_punctuation(&format!("First line {phrase} Second"));
            assert!(o.contains("\n\n"), "expected double newline for {phrase}");
            assert!(o.contains("Second"));
        }
    }

    #[test]
    fn spoken_key_commands() {
        assert!(apply_spoken_punctuation("ok press enter").contains(KEY_SENTINEL_ENTER));
        assert!(apply_spoken_punctuation("hit return now").contains(KEY_SENTINEL_ENTER));
        assert!(apply_spoken_punctuation("hit caps lock").contains(KEY_SENTINEL_CAPS_LOCK));
        assert!(apply_spoken_punctuation("press capslock").contains(KEY_SENTINEL_CAPS_LOCK));
        assert!(apply_spoken_punctuation("press tab").contains(KEY_SENTINEL_TAB));
        assert!(apply_spoken_punctuation("hit esc").contains(KEY_SENTINEL_ESCAPE));
        assert!(apply_spoken_punctuation("press backspace").contains(KEY_SENTINEL_BACKSPACE));
        let o = apply_spoken_punctuation("before press enter after");
        assert!(o.contains("before") && o.contains("after"));
        assert!(!o.contains("press"));
    }

    #[test]
    fn repair_dialogue_comma_quote() {
        assert_eq!(
            repair_asr_punctuation(r#"leave,", Kane said"#),
            r#"leave," Kane said"#
        );
    }

    #[test]
    fn repair_stutter_periods() {
        assert_eq!(repair_asr_punctuation("vampire.. . Only"), "vampire. Only");
        assert_eq!(repair_asr_punctuation("end. . Start"), "end. Start");
    }

    #[test]
    fn repair_stutter_question_exclamation() {
        assert_eq!(repair_asr_punctuation("really??"), "really?");
        assert_eq!(repair_asr_punctuation("really? ?"), "really?");
        assert_eq!(repair_asr_punctuation("no way!!"), "no way!");
        assert_eq!(repair_asr_punctuation("no way! !"), "no way!");
    }

    #[test]
    fn spoken_question_mark_after_whisper_question() {
        assert_eq!(
            apply_spoken_punctuation("really? question mark"),
            "really? "
        );
    }

    #[test]
    fn repair_double_comma_after_quote() {
        assert_eq!(
            repair_asr_punctuation(r#""Hi",, she said"#),
            r#""Hi", she said"#
        );
    }

    #[test]
    fn collapse_comma_runs_fiction_style_lists() {
        let s = "Ciri nodded furiously,, Oh, the things she wanted to do,, the promise";
        assert_eq!(
            collapse_comma_runs(s),
            "Ciri nodded furiously, Oh, the things she wanted to do, the promise"
        );
    }

    #[test]
    fn spoken_comma_after_whisper_comma_does_not_double() {
        assert_eq!(
            apply_spoken_punctuation("furiously, comma Oh the things"),
            "furiously, Oh the things"
        );
        assert_eq!(
            apply_spoken_punctuation("furiously , comma Oh"),
            "furiously, Oh"
        );
    }

    #[test]
    fn repair_pause_punctuation_after_close_quote() {
        assert_eq!(
            repair_asr_punctuation(r#"She said "Hi" , she left"#),
            r#"She said "Hi," she left"#
        );
        assert_eq!(
            repair_asr_punctuation(r#"She said "Hi" . She left"#),
            r#"She said "Hi" She left"#
        );
        assert_eq!(
            repair_asr_punctuation(r#"about 6" , wide"#),
            r#"about 6" , wide"#
        );
    }

    #[test]
    fn apply_spoken_close_quotes_then_comma_phrase() {
        let o = apply_spoken_punctuation(r#"word close quotes comma she said"#);
        assert!(
            o.contains("\"she") || o.contains(", she"),
            "unexpected output: {o:?}"
        );
        assert!(!o.contains("\" ,"), "comma should not float outside quotes: {o:?}");
    }

    #[test]
    fn repair_duplicate_period_after_closing_quote() {
        assert_eq!(
            repair_asr_punctuation(r#""So that's it," . Vivian said"#),
            r#""So that's it," Vivian said"#
        );
        assert_eq!(
            repair_asr_punctuation(r#""Stop." . She nodded"#),
            r#""Stop." She nodded"#
        );
    }

    #[test]
    fn spoken_open_quotes_drops_leading_period_artifact() {
        let o = apply_spoken_punctuation(r#"period open quotes Oh gods comma close quotes"#);
        assert!(
            !o.trim_start().starts_with(". \""),
            "unexpected leading period before opening quote: {o:?}"
        );
        assert!(
            o.contains(r#""Oh gods,"#),
            "expected quoted phrase to remain intact: {o:?}"
        );
    }

    #[test]
    fn spoken_open_paren_drops_leading_period_artifact() {
        let o = apply_spoken_punctuation("period open parenthesis test close parenthesis");
        assert!(
            !o.trim_start().starts_with(". ("),
            "unexpected leading period before open paren: {o:?}"
        );
        assert!(o.trim_start().starts_with('('), "expected utterance to start with (: {o:?}");
    }

    #[test]
    fn spoken_period_inside_quotes_does_not_glue_to_quote_mark() {
        let o = apply_spoken_punctuation("open quotes period Oh gods close quotes");
        assert!(
            !o.contains("\"."),
            "sentence tighten must not treat \" as word char before period: {o:?}"
        );
    }
}
