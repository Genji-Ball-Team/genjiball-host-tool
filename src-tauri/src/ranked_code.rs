//! Fills the game's `RANKS - generated` rule with the rank tags, as GenjiBall-CE
//! `docs/rank-tags.md` (on `v1.3.3R`) defines it. Pure: the release and the leaderboard are
//! fetched elsewhere.
//!
//! The tags are the region's top players, not the server's rank tiers: each one is a "tier" of
//! their own, so the tag over them is their place and rating (`#1 | 2143`), and everyone else gets
//! none. The game needs no change for it.

use crate::config;
use crate::server::Leaderboard;

/// One tier of `rankTags`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tier {
    /// The tag over the player.
    pub label: String,
    /// RGBA, 0–255.
    pub color: [u8; 4],
    /// A line in the game's guide.
    pub guide: String,
    /// Display names, raw: not escaped for the Workshop yet.
    pub names: Vec<String>,
}

/// The game's `rankTags`, as data.
#[derive(Debug, Clone, PartialEq)]
pub struct RankTags {
    /// `rankTags[0]`, a line in the game's guide.
    pub header: String,
    /// Lowest tier first.
    pub tiers: Vec<Tier>,
}

/// The tag's colour for a player below the lowest tier.
const UNTIERED_COLOR: [u8; 4] = [255, 255, 255, 255];

/// Where the game's guide (`Rank tags - tier guide`, GenjiBall-CE `workshop/genjiball.txt`) puts
/// its lines: HUD sort orders, lower first. Its own "Live leaderboard" line is at -40 and made
/// first, `rankTags[0]` at -39 and made next, then each tier at `-30 - index`, highest first.
const GUIDE_HEADER_SORT: i64 = -39;
const GUIDE_TIER_SORT: i64 = -30;

/// The guide's lines from the top, after its own "Live leaderboard": `None` for `rankTags[0]`,
/// `Some(i)` for tier `i` (1 is the lowest). Lines with the same sort order are shown in the order
/// the game made them, so past 8 tiers a tier's line goes above `rankTags[0]`.
fn guide_order(tiers: usize) -> Vec<Option<usize>> {
    // (sort order, when it's made, line)
    let mut lines = vec![(GUIDE_HEADER_SORT, 0, None)];
    for i in 1..=tiers {
        lines.push((GUIDE_TIER_SORT - i as i64, 1 + tiers - i, Some(i)));
    }
    lines.sort();
    lines.into_iter().map(|(_, _, line)| line).collect()
}

/// `rankTags` for the leaderboard's top `config::TOP_TAGGED` players: a tier each, the best
/// highest, tagged with their place and rating. The guide lists them best first, under a line
/// with `date`. Players whose name the Workshop can't show are left out (counted, the second
/// value), and the next one takes their spot; their place stays as the site shows it.
pub fn top_tags(leaderboard: &Leaderboard, date: &str) -> (RankTags, usize) {
    let mut skipped = 0;
    let mut top = Vec::new();
    for player in &leaderboard.players {
        if top.len() == config::TOP_TAGGED {
            break;
        }
        let rating = player.rating.round();
        let guide = format!("#{} {} - {rating}", player.rank, player.name);
        if !fits(&player.name) || !fits(&guide) {
            skipped += 1;
            continue;
        }
        let color = player.tier.as_ref().map_or(UNTIERED_COLOR, |t| {
            [t.color[0], t.color[1], t.color[2], 255]
        });
        top.push(Tier {
            label: format!("#{} | {rating}", player.rank),
            color,
            guide,
            names: vec![player.name.clone()],
        });
    }
    top.reverse();

    let title = if top.is_empty() {
        "No ranked players yet".to_string()
    } else {
        format!("Top {} on {date}", top.len())
    };
    // The guide's lines are each tier's own: put the texts in them in the order they show.
    let mut texts = vec![title];
    texts.extend(top.iter().rev().map(|t| t.guide.clone()));
    let mut header = String::new();
    for (line, text) in guide_order(top.len()).into_iter().zip(texts) {
        match line {
            None => header = text,
            Some(i) => top[i - 1].guide = text,
        }
    }
    (RankTags { header, tiers: top }, skipped)
}

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
                "The rank tags have a line the Workshop can't show: {text}"
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
    use crate::server::{LeaderboardTier, Ranked};

    /// The real rule and its neighbours, from GenjiBall-CE `workshop/genjiball.txt` on `v1.3.3R`.
    const EXAMPLE: &str = include_str!("../tests/fixtures/ranks-rule-example.txt");

    fn tags() -> RankTags {
        RankTags {
            header: "Ranks updated 2026-10-03".into(),
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

    fn ranked(rank: u32, name: &str, rating: f64, color: Option<[u8; 3]>) -> Ranked {
        Ranked {
            rank,
            name: name.into(),
            rating,
            tier: color.map(|color| LeaderboardTier { color }),
        }
    }

    fn board(players: Vec<Ranked>) -> Leaderboard {
        Leaderboard {
            region: Some("eu".into()),
            players,
        }
    }

    /// The guide's texts from the top, as the game shows them (`guide_order`).
    fn guide(tags: &RankTags) -> Vec<String> {
        guide_order(tags.tiers.len())
            .into_iter()
            .map(|line| match line {
                None => tags.header.clone(),
                Some(i) => tags.tiers[i - 1].guide.clone(),
            })
            .collect()
    }

    #[test]
    fn the_guide_follows_the_games_sort_order() {
        // Up to 8 tiers: rankTags[0], then the highest tier down.
        assert_eq!(guide_order(3), vec![None, Some(3), Some(2), Some(1)]);
        // 10: tier 10 shares -40 with the game's header and tier 9 -39 with rankTags[0], each
        // made after it.
        assert_eq!(
            guide_order(10),
            [vec![Some(10), None], (1..=9).rev().map(Some).collect()].concat()
        );
    }

    #[test]
    fn tags_the_top_players_with_place_and_rating() {
        let (tags, skipped) = top_tags(
            &board(vec![
                ranked(1, "Kenzo", 2143.0, Some([150, 0, 0])),
                ranked(2, "Hana", 1010.4, None),
            ]),
            "2026-10-05",
        );
        assert_eq!(skipped, 0);
        assert_eq!(
            tags,
            RankTags {
                header: "Top 2 on 2026-10-05".into(),
                tiers: vec![
                    Tier {
                        label: "#2 | 1010".into(),
                        color: [255, 255, 255, 255],
                        guide: "#2 Hana - 1010".into(),
                        names: vec!["Hana".into()],
                    },
                    Tier {
                        label: "#1 | 2143".into(),
                        color: [150, 0, 0, 255],
                        guide: "#1 Kenzo - 2143".into(),
                        names: vec!["Kenzo".into()],
                    },
                ],
            }
        );
        let (action, names, _) = rank_tags_action(&tags).unwrap();
        assert_eq!(names, 2);
        assert!(
            action.contains(r##"Array(Custom String("#1 | 2143"), Custom Color(150, 0, 0, 255)"##),
            "{action}"
        );
    }

    #[test]
    fn tags_only_the_top_ten_and_lists_them_best_first() {
        let players = (1..=15)
            .map(|rank| ranked(rank, &format!("P{rank}"), 2000.0 - f64::from(rank), None))
            .collect();
        let (tags, _) = top_tags(&board(players), "2026-10-05");
        assert_eq!(tags.tiers.len(), config::TOP_TAGGED);
        // Lowest first: the best player is the highest tier.
        assert_eq!(tags.tiers[9].names, vec!["P1".to_string()]);
        assert_eq!(tags.tiers[0].names, vec!["P10".to_string()]);
        let mut expected = vec!["Top 10 on 2026-10-05".to_string()];
        expected.extend((1..=10).map(|r| format!("#{r} P{r} - {}", 2000 - r)));
        assert_eq!(guide(&tags), expected);
    }

    #[test]
    fn skips_names_the_workshop_cant_show_and_keeps_site_places() {
        let (tags, skipped) = top_tags(
            &board(vec![
                ranked(1, "{0}", 2100.0, None),
                ranked(2, "Kenzo", 2000.0, None),
            ]),
            "2026-10-05",
        );
        assert_eq!(skipped, 1);
        assert_eq!(tags.tiers.len(), 1);
        assert_eq!(tags.tiers[0].label, "#2 | 2000");
    }

    #[test]
    fn an_empty_leaderboard_tags_nobody() {
        let (tags, skipped) = top_tags(&board(vec![]), "2026-10-05");
        assert_eq!((tags.tiers.len(), skipped), (0, 0));
        assert_eq!(tags.header, "No ranked players yet");
        assert_eq!(
            rank_tags_action(&tags).unwrap().0,
            r#"Set Global Variable(rankTags, Array(Custom String("No ranked players yet")));"#
        );
    }
}
