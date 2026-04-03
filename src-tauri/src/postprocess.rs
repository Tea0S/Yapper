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
            ("new line", "\n"),
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

pub fn apply_spoken_punctuation(text: &str) -> String {
    let mut s = text.to_string();
    for (re, rep) in spoken_punct_rules() {
        s = re.replace_all(&s, *rep).into_owned();
    }
    s = normalize_after_spoken_punct(&s);
    repair_asr_punctuation(&s)
}

/// Tighten typical ASR spacing around punctuation after phrase replacement.
fn normalize_after_spoken_punct(s: &str) -> String {
    // Sentence end: keep one space after so the next word isn't glued (e.g. "Hello. how").
    let Ok(re_sent) = Regex::new(r"(\S)\s+([.!?])\s*") else {
        return s.to_string();
    };
    let mut out = re_sent.replace_all(s, "$1$2 ").into_owned();
    let Ok(re_comma) = Regex::new(r"(\S)\s+,\s*") else {
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
    let Ok(re_quote_open) = Regex::new(r#""\s+"#) else {
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
    re_spaces.replace_all(&out, " ").into_owned()
}

/// Fix collisions when spoken punctuation and Whisper both insert marks (e.g. `leave,", Kane`, `.. .`).
pub(crate) fn repair_asr_punctuation(s: &str) -> String {
    let mut out = s.to_string();
    // leave,", Kane said → leave," Kane said (comma belongs inside the closing quote for attribution)
    if let Ok(re) = Regex::new(r#"([A-Za-z']+),\s*"\s*,\s+([A-Z][a-z]*)"#) {
        out = re.replace_all(&out, "${1},\" ${2}").into_owned();
    }
    // Same after ? or ! before dialogue tag
    if let Ok(re) = Regex::new(r#"([?!]),\s*"\s*,\s+([A-Z][a-z]*)"#) {
        out = re.replace_all(&out, "${1},\" ${2}").into_owned();
    }
    // Collapse comma runs first (e.g. "Hi",, she) before quote-tightening below.
    if let Ok(re) = Regex::new(r",\s*,+") {
        out = re.replace_all(&out, ",").into_owned();
    }
    // Double comma right after a closing quote (if any survived): "foo",, bar → "foo", bar
    if let Ok(re) = Regex::new(r#""\s*,\s*,"#) {
        out = re.replace_all(&out, "\u{22}, ").into_owned();
    }
    // Stuttered periods / spaced dots (Whisper + spoken "period"): vampire.. . → vampire.
    if let Ok(re) = Regex::new(r"\.(?:\s*\.)+") {
        out = re.replace_all(&out, ".").into_owned();
    }
    // Optional space between two periods that survived: . . → .
    if let Ok(re) = Regex::new(r"\.\s+\.") {
        out = re.replace_all(&out, ".").into_owned();
    }
    // Period glued to comma (often duplicate sentence boundary): ., → .
    if let Ok(re) = Regex::new(r"\.,\s*") {
        out = re.replace_all(&out, ". ").into_owned();
    }
    // Comma then period with only space: , . → .
    if let Ok(re) = Regex::new(r",\s+\.") {
        out = re.replace_all(&out, ".").into_owned();
    }
    out
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
        let o = apply_spoken_punctuation("First line new paragraph Second");
        assert!(o.contains("\n\n"));
        assert!(o.contains("Second"));
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
    fn repair_double_comma_after_quote() {
        assert_eq!(
            repair_asr_punctuation(r#""Hi",, she said"#),
            r#""Hi", she said"#
        );
    }
}
