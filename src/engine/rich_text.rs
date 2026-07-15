//! Parser for the Unreal Engine rich-text markup that ships inside some game
//! string tables (skill/ability names and their descriptions).
//!
//! The game renders those strings through an `URichTextBlock`, which consumes
//! decorator tags and shows only the wrapped text. The markup looks like:
//!
//! ```text
//! <Title>变轨技能：轮转打击</>
//! 进行至多<NumGreen>5</>段的连续攻击
//! 命中后获得<hot textstyle="TextEXPL" param="nouns&HathorAnger">「闪送之力」</>
//! ```
//!
//! i.e. self-describing open tags (optionally with attributes), the generic UE
//! close `</>`, and occasionally a named close `</Title>`. Our UI draws these
//! names as plain `egui` labels, so nothing strips the tags for us the way the
//! game's rich-text widget would.
//!
//! Most rows carry clean names already (e.g. `度恶`), but a handful bake the
//! markup straight into the name field — `GA_Hathor_Skill2` →
//! `<Title>变轨技能：轮转打击</>`, `GA_Mint_Passive_3` →
//! `<Title>普通攻击：薄荷味旋风</>` — which would otherwise leak into the
//! combat-detail list verbatim. [`strip_tags`] reduces such a value to the plain
//! text the player actually sees.

/// A single lexical unit of a rich-text string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token<'a> {
    /// Literal text between tags (the part the player reads).
    Text(&'a str),
    /// An opening decorator tag, e.g. `Title` or `hot` — attributes are dropped.
    Open(&'a str),
    /// A closing tag: the generic UE `</>` (`None`) or a named `</Title>`.
    Close(Option<&'a str>),
}

/// Splits a rich-text string into [`Token`]s without allocating. A `<…>` run is
/// only treated as a tag when its body starts with an ASCII letter (open tag) or
/// `/` (close tag); any other `<` — a stray less-than in real text, or a `<` with
/// no matching `>` — is emitted as literal [`Token::Text`] so nothing is lost.
pub fn tokenize(input: &str) -> Vec<Token<'_>> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    let mut text_start = 0;

    while let Some(offset) = bytes[cursor..].iter().position(|&byte| byte == b'<') {
        let open = cursor + offset;
        // A tag body runs up to the next '>'. Attribute values never contain '>'
        // (the game escapes them), so the first '>' always closes the tag.
        let Some(rel_close) = bytes[open + 1..].iter().position(|&byte| byte == b'>') else {
            // No closing '>': everything from here on is literal text.
            break;
        };
        let close = open + 1 + rel_close;
        let body = &input[open + 1..close];
        let Some(token) = classify_tag(body) else {
            // A '<' that isn't a well-formed tag (e.g. "a < b"): keep scanning
            // after it so the '<' stays part of the surrounding text run.
            cursor = open + 1;
            continue;
        };
        if open > text_start {
            tokens.push(Token::Text(&input[text_start..open]));
        }
        tokens.push(token);
        cursor = close + 1;
        text_start = cursor;
    }

    if text_start < input.len() {
        tokens.push(Token::Text(&input[text_start..]));
    }
    tokens
}

/// Classifies the body of a `<…>` run (the text between the angle brackets).
/// Returns `None` when it isn't a decorator tag, so the caller keeps the `<`
/// as literal text.
fn classify_tag(body: &str) -> Option<Token<'_>> {
    if let Some(name) = body.strip_prefix('/') {
        // `</>` is the generic UE close; `</Title>` names the tag it closes.
        let name = name.trim();
        return Some(Token::Close((!name.is_empty()).then_some(name)));
    }
    // An open tag starts with a letter; the name ends at the first whitespace,
    // with any attributes (`textstyle="…" param="…"`) following it.
    let first = body.chars().next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let name = body.split_whitespace().next().unwrap_or(body);
    Some(Token::Open(name))
}

/// Removes UE rich-text decorator tags, keeping only the wrapped text, and trims
/// the result. Handles open tags with attributes, the generic close `</>`, and
/// named closes; preserves internal whitespace (so multi-line descriptions keep
/// their shape) and any non-tag `<`/`>`.
pub fn strip_tags(input: &str) -> String {
    // Fast path: the vast majority of names have no markup at all.
    if !input.contains('<') {
        return input.trim().to_owned();
    }
    let mut output = String::with_capacity(input.len());
    for token in tokenize(input) {
        if let Token::Text(text) = token {
            output.push_str(text);
        }
    }
    let trimmed = output.trim();
    if trimmed.len() == output.len() {
        output
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_title_wrapper_from_names() {
        assert_eq!(
            strip_tags("<Title>变轨技能：轮转打击</>"),
            "变轨技能：轮转打击"
        );
        assert_eq!(
            strip_tags("<Title>普通攻击：薄荷味旋风</>"),
            "普通攻击：薄荷味旋风"
        );
        assert_eq!(
            strip_tags("<Title>Redirect Skill: Cyclone Strike</>"),
            "Redirect Skill: Cyclone Strike"
        );
    }

    #[test]
    fn passes_through_clean_names_untouched() {
        assert_eq!(strip_tags("度恶"), "度恶");
        assert_eq!(strip_tags("极轨终结：沸血赤红"), "极轨终结：沸血赤红");
        assert_eq!(strip_tags(""), "");
        assert_eq!(strip_tags("   "), "");
    }

    #[test]
    fn strips_inline_and_attributed_tags_within_text() {
        assert_eq!(
            strip_tags("进行至多<NumGreen>5</>段的连续攻击"),
            "进行至多5段的连续攻击"
        );
        assert_eq!(
            strip_tags(
                "命中后获得<hot textstyle=\"TextEXPL\" param=\"nouns&HathorAnger\">「闪送之力」</>"
            ),
            "命中后获得「闪送之力」"
        );
    }

    #[test]
    fn keeps_non_tag_angle_brackets() {
        assert_eq!(strip_tags("a < b > c"), "a < b > c");
        assert_eq!(strip_tags("HP < 50%"), "HP < 50%");
        // A '<' with no closing '>' is literal.
        assert_eq!(strip_tags("value <= 3"), "value <= 3");
    }

    #[test]
    fn empty_wrapper_reduces_to_empty() {
        assert_eq!(strip_tags("<Title></>"), "");
        assert_eq!(strip_tags(" <Italic></> "), "");
    }

    #[test]
    fn tokenizes_open_text_and_close() {
        assert_eq!(
            tokenize("<Title>abc</>"),
            vec![Token::Open("Title"), Token::Text("abc"), Token::Close(None)]
        );
        assert_eq!(
            tokenize("x<hot a=\"1\">y</hot>z"),
            vec![
                Token::Text("x"),
                Token::Open("hot"),
                Token::Text("y"),
                Token::Close(Some("hot")),
                Token::Text("z"),
            ]
        );
    }
}
