//! Fills the game's `RANKS - generated` rule with the rank tags, as GenjiBall-CE
//! `docs/rank-tags.md` (on `v1.3.3R`) defines it. Pure: the release and the tags are fetched
//! elsewhere.

use crate::server::RankTags;

/// The rule's first line. It's in the code exactly once.
pub const RULE_START: &str = r#"rule ("RANKS - generated") {"#;

/// The line that ends the rule: the first one after `RULE_START` that is exactly this.
const RULE_END: &str = "}";

/// The most characters a Workshop `Custom String` holds.
const CUSTOM_STRING_MAX_CHARS: usize = 128;

/// The ranked code, and what went into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Filled {
    pub code: String,
    /// Names in the rule.
    pub names: usize,
    /// Names left out: the Workshop can't show them (`{`, `}`, too long).
    pub skipped: usize,
}

/// A name the Workshop can write in a `Custom String`: `{` and `}` are placeholders there.
fn fits(text: &str) -> bool {
    !text.contains(['{', '}']) && text.chars().count() <= CUSTOM_STRING_MAX_CHARS
}

/// `Custom String("<text>")`, with `"` and `\` escaped.
fn custom_string(text: &str) -> String {
    let escaped = text.replace('\\', r"\\").replace('"', r#"\""#);
    format!(r#"Custom String("{escaped}")"#)
}

/// The rule's action: `Set Global Variable(rankTags, Array(...));`. Errors when the header, a
/// label or a guide line can't be written (the server never sends those); names that can't are
/// left out and counted.
pub fn rank_tags_action(tags: &RankTags) -> Result<(String, usize, usize), String> {
    let check = |text: &str| {
        if fits(text) {
            Ok(custom_string(text))
        } else {
            Err(format!(
                "The server's rank tags have a line the Workshop can't show: {text}"
            ))
        }
    };
    let mut parts = vec![check(&tags.header)?];
    let (mut names, mut skipped) = (0, 0);
    for tier in &tags.tiers {
        let [r, g, b, a] = tier.color;
        let mut tier_parts = vec![
            check(&tier.label)?,
            format!("Custom Color({r}, {g}, {b}, {a})"),
            check(&tier.guide)?,
        ];
        for name in &tier.names {
            if fits(name) {
                tier_parts.push(custom_string(name));
                names += 1;
            } else {
                skipped += 1;
            }
        }
        parts.push(format!("Array({})", tier_parts.join(", ")));
    }
    let action = format!(
        "Set Global Variable(rankTags, Array({}));",
        parts.join(", ")
    );
    Ok((action, names, skipped))
}

/// The whole rule, in the shape the spec gives, with `eol` between lines.
fn rule(action: &str, eol: &str) -> String {
    [
        RULE_START,
        "    event {",
        "        Ongoing - Global;",
        "    }",
        "    actions {",
        &format!("        {action}"),
        "    }",
        RULE_END,
    ]
    .join(eol)
}

/// `code` with its `RANKS - generated` rule replaced by one holding `tags`. Everything else,
/// line endings included, stays as it was.
pub fn fill(code: &str, tags: &RankTags) -> Result<Filled, String> {
    // Line starts, with each line's text without its line ending.
    let mut lines = Vec::new();
    let mut at = 0;
    for line in code.split_inclusive('\n') {
        let text = line
            .strip_suffix('\n')
            .map_or(line, |l| l.strip_suffix('\r').unwrap_or(l));
        lines.push((at, text, line.len()));
        at += line.len();
    }

    let mut starts = lines.iter().enumerate().filter(|(_, l)| l.1 == RULE_START);
    let (start_index, &(start, start_text, start_len)) = starts
        .next()
        .ok_or("The release's code has no RANKS - generated rule. Is it a ranked (R) build?")?;
    if starts.next().is_some() {
        return Err(
            "The release's code has the RANKS - generated rule twice, so the host tool can't tell which to fill".into(),
        );
    }
    let &(end, end_text, end_len) = lines[start_index + 1..]
        .iter()
        .find(|l| l.1 == RULE_END)
        .ok_or("The RANKS - generated rule in the release's code never ends")?;

    // The rule's own line endings: whatever its first line uses.
    let eol = &code[start + start_text.len()..start + start_len];
    let eol = if eol.is_empty() { "\n" } else { eol };
    // Whatever followed the closing `}` (nothing at the end of the file).
    let after = &code[end + end_text.len()..end + end_len];

    let (action, names, skipped) = rank_tags_action(tags)?;
    let mut filled = String::with_capacity(code.len() + action.len());
    filled.push_str(&code[..start]);
    filled.push_str(&rule(&action, eol));
    filled.push_str(after);
    filled.push_str(&code[end + end_len..]);
    Ok(Filled {
        code: filled,
        names,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::Tier;

    /// The real rule and its neighbours, from GenjiBall-CE `workshop/genjiball.txt` on `v1.3.3R`.
    const EXAMPLE: &str = include_str!("../tests/fixtures/ranks-rule-example.txt");

    fn tags() -> RankTags {
        RankTags {
            header: "Ranks updated 2026-10-03".into(),
            updated_at: "2026-10-03T12:00:00Z".into(),
            region: Some("eu".into()),
            tiers: vec![
                Tier {
                    label: "Apprentice".into(),
                    color: [205, 127, 50, 255],
                    guide: "Apprentice - 1300".into(),
                    names: vec!["Kenzo".into(), "唖者".into()],
                },
                Tier {
                    label: "Master".into(),
                    color: [255, 215, 0, 255],
                    guide: "Master - 1600".into(),
                    names: vec![],
                },
            ],
        }
    }

    const ACTION: &str = r#"Set Global Variable(rankTags, Array(Custom String("Ranks updated 2026-10-03"), Array(Custom String("Apprentice"), Custom Color(205, 127, 50, 255), Custom String("Apprentice - 1300"), Custom String("Kenzo"), Custom String("唖者")), Array(Custom String("Master"), Custom Color(255, 215, 0, 255), Custom String("Master - 1600"))));"#;

    #[test]
    fn builds_the_action_in_the_spec_shape() {
        assert_eq!(rank_tags_action(&tags()).unwrap(), (ACTION.into(), 2, 0));
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        let mut tags = tags();
        tags.tiers[0].names = vec![r#"a"b\c"#.into()];
        let (action, names, _) = rank_tags_action(&tags).unwrap();
        assert!(action.contains(r#"Custom String("a\"b\\c")"#), "{action}");
        assert_eq!(names, 1);
    }

    #[test]
    fn leaves_out_names_the_workshop_cant_show() {
        let mut tags = tags();
        tags.tiers[0].names = vec![
            "{0}".into(),
            "a}".into(),
            "x".repeat(CUSTOM_STRING_MAX_CHARS + 1),
            "x".repeat(CUSTOM_STRING_MAX_CHARS),
        ];
        let (action, names, skipped) = rank_tags_action(&tags).unwrap();
        assert_eq!((names, skipped), (1, 3));
        assert!(!action.contains("{0}"));
        assert!(!action.contains("x".repeat(CUSTOM_STRING_MAX_CHARS + 1).as_str()));
    }

    #[test]
    fn refuses_a_header_or_label_it_cant_write() {
        let mut bad_header = tags();
        bad_header.header = "{0}".into();
        assert!(rank_tags_action(&bad_header).is_err());
        let mut bad_label = tags();
        bad_label.tiers[1].label = "x".repeat(CUSTOM_STRING_MAX_CHARS + 1);
        assert!(rank_tags_action(&bad_label).is_err());
    }

    /// The fixture with the rule's action line replaced: what `fill` must give.
    fn expected(code: &str, eol: &str) -> String {
        let old = code
            .split(eol)
            .find(|l| l.trim_start().starts_with("Set Global Variable(rankTags"))
            .unwrap();
        code.replace(old, &format!("        {ACTION}"))
    }

    #[test]
    fn replaces_only_the_rule() {
        let filled = fill(EXAMPLE, &tags()).unwrap();
        assert_eq!(filled.code, expected(EXAMPLE, "\n"));
        assert_eq!((filled.names, filled.skipped), (2, 0));
        // The neighbours are untouched.
        assert!(filled
            .code
            .starts_with("rule (\"Ranked log - MATCH_END\") {\n"));
        assert!(filled.code.contains("rule (\"Rank tags - find tier\") {\n"));
        assert_eq!(filled.code.matches(RULE_START).count(), 1);
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let crlf = EXAMPLE.replace('\n', "\r\n");
        let filled = fill(&crlf, &tags()).unwrap();
        assert_eq!(filled.code, expected(&crlf, "\r\n"));
        assert!(!filled.code.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn handles_the_rule_at_the_end_of_the_file() {
        let start = EXAMPLE.find(RULE_START).unwrap();
        let end = EXAMPLE[start..].find("\n}\n").unwrap() + start + 2;
        let last = &EXAMPLE[..end];
        let filled = fill(last, &tags()).unwrap();
        assert!(filled.code.ends_with("    }\n}"));
        assert_eq!(filled.code[..start], last[..start]);
    }

    #[test]
    fn stops_without_exactly_one_rule() {
        let none = EXAMPLE.replace(RULE_START, r#"rule ("RANKS") {"#);
        assert!(fill(&none, &tags()).unwrap_err().contains("no RANKS"));
        // Not the marker unless the whole line is.
        let indented = EXAMPLE.replace(RULE_START, &format!(" {RULE_START}"));
        assert!(fill(&indented, &tags()).is_err());
        let twice = format!("{EXAMPLE}\n{EXAMPLE}");
        assert!(fill(&twice, &tags()).unwrap_err().contains("twice"));
        let start = EXAMPLE.find(RULE_START).unwrap();
        let unended = &EXAMPLE[..start + RULE_START.len() + 40];
        assert!(fill(unended, &tags()).unwrap_err().contains("never ends"));
    }
}
