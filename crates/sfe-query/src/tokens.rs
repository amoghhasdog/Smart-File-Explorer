//! Tokeniser: quoted phrases stay whole; everything else splits on
//! whitespace with surrounding punctuation stripped.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tok {
    /// Lowercased, punctuation-trimmed form used for matching.
    pub text: String,
    /// Original spelling (for phrases / semantic text).
    pub orig: String,
    pub quoted: bool,
    pub used: bool,
}

pub(crate) fn tokenize(input: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix(['"', '“']) {
            let end = after.find(['"', '”']).unwrap_or(after.len());
            let phrase = after[..end].trim();
            if !phrase.is_empty() {
                out.push(Tok {
                    text: phrase.to_lowercase(),
                    orig: phrase.to_string(),
                    quoted: true,
                    used: false,
                });
            }
            rest = after.get(end + 1..).unwrap_or("").trim_start();
            continue;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..end];
        rest = rest[end..].trim_start();
        let trimmed = word.trim_matches(|c: char| {
            matches!(
                c,
                ',' | ';' | ':' | '(' | ')' | '?' | '!' | '[' | ']' | '{' | '}'
            )
        });
        let trimmed = trimmed.trim_end_matches('.');
        if trimmed.is_empty() {
            continue;
        }
        out.push(Tok {
            text: trimmed.to_lowercase(),
            orig: trimmed.to_string(),
            quoted: false,
            used: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_keeps_quoted_phrases() {
        let t = tokenize(r#"Find "Machine Learning" PDFs, (please)!"#);
        let texts: Vec<_> = t.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["find", "machine learning", "pdfs", "please"]);
        assert!(t[1].quoted);
        assert_eq!(t[1].orig, "Machine Learning");
        assert_eq!(tokenize("  ").len(), 0);
        assert_eq!(tokenize("v1.2.").len(), 1);
        assert_eq!(tokenize("v1.2.")[0].text, "v1.2");
    }
}
