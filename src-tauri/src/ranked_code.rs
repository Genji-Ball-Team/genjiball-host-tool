//! Fills the game's `RANKS - generated` rule with the rank tags, as GenjiBall-CE
//! `docs/rank-tags.md` (on `v1.3.3R`) defines it. Pure: the release and the leaderboard are
//! fetched elsewhere.
//!
//! The region's top players are an entry each, so the tag over them is their place and rating
//! (`#1 | 2143`). Before them come the rank tiers, so everyone else in a tier gets its name.

use crate::config;
use crate::server::{Leaderboard, RankTiers};

/// One entry of `rankTags`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tier {
    /// The tag over the player.
    pub label: String,
    /// RGBA, 0–255.
    pub color: [u8; 4],
    /// The entry's line in the game's top 10 list. Empty for a rank tier: the game doesn't list it.
    pub guide: String,
    /// Display names, raw: not escaped for the Workshop yet.
    pub names: Vec<String>,
}

/// The game's `rankTags`, as data.
#[derive(Debug, Clone, PartialEq)]
pub struct RankTags {
    /// `rankTags[0]`, the line under the top 10 list's header.
    pub header: String,
    /// The rank tiers, lowest first, then the top players, the best last.
    pub tiers: Vec<Tier>,
}

/// The tag's colour for a player below the lowest tier.
const UNTIERED_COLOR: [u8; 4] = [255, 255, 255, 255];

/// What `top_tags` built.
#[derive(Debug, Clone, PartialEq)]
pub struct TopTags {
    pub tags: RankTags,
    /// Top players tagged with their place and rating.
    pub top: usize,
    /// Top players left out: the Workshop can't show their names.
    pub skipped: usize,
}

/// `rankTags`: the server's rank tiers, lowest first, with no line in the game's list, then the
/// leaderboard's top `config::TOP_TAGGED` players, an entry each, the best last, tagged with
/// their place and rating. A top player is left out of their tier, so everyone else in a tier
/// gets its name (the game would take the last entry with their name anyway). The list shows the top players
/// best first, under a line with `date`. Top players whose name the Workshop can't show are left
/// out, and the next one takes their spot; their place stays as the site shows it.
pub fn top_tags(tiers: &RankTiers, leaderboard: &Leaderboard, date: &str) -> TopTags {
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

    let header = if top.is_empty() {
        "No ranked players yet".to_string()
    } else {
        format!("Top {} on {date}", top.len())
    };
    let count = top.len();
    let mut entries: Vec<Tier> = tiers
        .tiers
        .iter()
        .map(|tier| Tier {
            label: tier.label.clone(),
            color: tier.color,
            guide: String::new(),
            names: tier
                .names
                .iter()
                .filter(|name| !top.iter().any(|t| t.names.contains(name)))
                .cloned()
                .collect(),
        })
        .collect();
    entries.extend(top);
    TopTags {
        tags: RankTags {
            header,
            tiers: entries,
        },
        top: count,
        skipped,
    }
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
    use crate::server::{LeaderboardTier, RankTier, Ranked};

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

    fn no_tiers() -> RankTiers {
        RankTiers {
            header: String::new(),
            updated_at: String::new(),
            region: Some("eu".into()),
            tiers: vec![],
        }
    }

    fn board(players: Vec<Ranked>) -> Leaderboard {
        Leaderboard {
            region: Some("eu".into()),
            players,
        }
    }

    #[test]
    fn tags_the_top_players_with_place_and_rating() {
        let TopTags { tags, skipped, .. } = top_tags(
            &no_tiers(),
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
    fn tags_only_the_top_ten_the_best_last() {
        let players = (1..=15)
            .map(|rank| ranked(rank, &format!("P{rank}"), 2000.0 - f64::from(rank), None))
            .collect();
        let TopTags { tags, top, .. } = top_tags(&no_tiers(), &board(players), "2026-10-05");
        assert_eq!(top, config::TOP_TAGGED);
        assert_eq!(tags.tiers.len(), config::TOP_TAGGED);
        // The best last: the game looks names up from the end.
        assert_eq!(tags.tiers[9].names, vec!["P1".to_string()]);
        assert_eq!(tags.tiers[0].names, vec!["P10".to_string()]);
        assert_eq!(tags.header, "Top 10 on 2026-10-05");
        assert_eq!(tags.tiers[9].guide, "#1 P1 - 1999");
        assert_eq!(tags.tiers[0].guide, "#10 P10 - 1990");
    }

    #[test]
    fn skips_names_the_workshop_cant_show_and_keeps_site_places() {
        let TopTags { tags, skipped, .. } = top_tags(
            &no_tiers(),
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
        let TopTags { tags, skipped, .. } = top_tags(&no_tiers(), &board(vec![]), "2026-10-05");
        assert_eq!((tags.tiers.len(), skipped), (0, 0));
        assert_eq!(tags.header, "No ranked players yet");
        assert_eq!(
            rank_tags_action(&tags).unwrap().0,
            r#"Set Global Variable(rankTags, Array(Custom String("No ranked players yet")));"#
        );
    }

    #[test]
    fn puts_the_tiers_before_the_top_players_with_no_list_line() {
        let tiers = RankTiers {
            tiers: vec![
                RankTier {
                    label: "Apprentice".into(),
                    color: [205, 127, 50, 255],
                    guide: "Apprentice - 1300".into(),
                    names: vec!["Hana".into()],
                },
                RankTier {
                    label: "Champion".into(),
                    color: [150, 0, 0, 255],
                    guide: "Champion - 2500".into(),
                    names: vec!["Kenzo".into(), "Genji".into()],
                },
            ],
            ..no_tiers()
        };
        let built = top_tags(
            &tiers,
            &board(vec![ranked(1, "Kenzo", 2600.0, Some([150, 0, 0]))]),
            "2026-10-05",
        );
        assert_eq!((built.top, built.skipped), (1, 0));
        let entries = &built.tags.tiers;
        assert_eq!(
            entries.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(),
            vec!["Apprentice", "Champion", "#1 | 2600"]
        );
        // The tiers aren't in the list, and the top player isn't in their tier.
        assert_eq!(entries[0].guide, "");
        assert_eq!(entries[1].guide, "");
        assert_eq!(entries[1].names, vec!["Genji".to_string()]);
        assert_eq!(entries[2].guide, "#1 Kenzo - 2600");
        let (action, names, _) = rank_tags_action(&built.tags).unwrap();
        assert_eq!(names, 3);
        assert!(
            action.contains(r#"Array(Custom String("Apprentice"), Custom Color(205, 127, 50, 255), Custom String(""), Custom String("Hana"))"#),
            "{action}"
        );
    }
}
