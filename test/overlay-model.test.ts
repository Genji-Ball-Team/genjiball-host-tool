import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { example } from "./fixture";
import type { Feed, MatchResult } from "../src/overlay/feed";
import { ago, overlayModel, readLiveLog } from "../src/overlay/model";

const tourneyExample = readFileSync(resolve("src-tauri/tests/fixtures/ranked-log-tourney-example.txt"), "utf8");
const NOW = Date.parse("2026-10-06T20:00:00Z");
const secondsAgo = (s: number) => new Date(NOW - s * 1000).toISOString();

function feed(change: Partial<Feed> = {}): Feed {
  return {
    stream: false,
    widgets: [],
    opacity: 92,
    scale: 100,
    layout: {},
    sizes: {},
    sizeRange: [0.5, 2.5, 0.1],
    editing: false,
    hotkeys: [{ action: "afk", label: "AFK on or off", keys: "Ctrl+Alt+A" }],
    pollMs: 1000,
    sizeSaveMs: 400,
    log: { file: "Log-2026-10-06-19-58-00.txt", size: 100, text: null },
    logWrittenAt: secondsAgo(2),
    logError: null,
    quietSecs: 60,
    host: { problem: null, waiting: 0, retrying: null, dryRun: false, afk: false, last: null },
    ratings: [],
    region: "eu",
    tourneys: [],
    result: null,
    session: { since: secondsAgo(3600), matches: 0, results: [] },
    logCopies: null,
    rankedCode: { builtAt: secondsAgo(600), staleSecs: 6 * 3600 },
    killFeedShown: 5,
    now: new Date(NOW).toISOString(),
    ...change,
  };
}

/** The example cut before line `n` (from 1): a match being played. */
const upTo = (text: string, n: number) => text.split("\n").slice(0, n - 1).join("\n") + "\n";

describe("a match being played", () => {
  // In round 2, after Mochi and a Ghost went out.
  const log = readLiveLog(upTo(example, 31));
  const model = overlayModel(feed(), log, NOW);

  it("says it's recording, and asks for the lobby's ratings", () => {
    expect(model.logging).toEqual({ tone: "good", title: "Recording", detail: "Round 2, 5 players" });
    expect(model.names).toEqual(["Sparrow", "Mochi", "Ghost", "Ghost", "Nova"]);
    expect(model.matchKey).toBe("482913507226");
  });

  it("lists who went out this round, and who sent the ball", () => {
    expect(model.eliminations).toEqual({
      round: 2,
      out: [
        { name: "Mochi", by: "Nova" },
        { name: "Ghost", by: null },
      ],
    });
    // A round is being played: no round result.
    expect(model.roundResult).toBeNull();
    expect(model.matchSummary).toBeNull();
  });

  it("feeds the kills newest first, with the deflects that led to them", () => {
    expect(model.killFeed!.map((k) => [k.killer, k.chain, k.victim, k.speed])).toEqual([
      [null, [], "Ghost", null],
      ["Nova", ["Nova"], "Mochi", 21],
      ["Sparrow", ["Sparrow"], "Ghost", 21],
      ["Ghost", ["Tidal", "Ghost"], "Mochi", 25],
      ["Sparrow", ["Mochi", "Sparrow"], "Ghost", 25],
    ]);
    expect(overlayModel(feed({ killFeedShown: 2 }), log, NOW).killFeed).toHaveLength(2);
  });

  it("ranks the standings as the site does", () => {
    expect(model.standings!.slice(0, 2)).toEqual([
      { place: 1, name: "Sparrow", wins: 1, kills: 2, left: false },
      { place: 2, name: "Ghost", wins: 0, kills: 1, left: false },
    ]);
  });
});

describe("the roster", () => {
  const log = readLiveLog(upTo(example, 31));

  it("rates the lobby best first, new players and doubled names flagged", () => {
    const ratings: Feed["ratings"] = [
      { name: "Sparrow", state: "found", standing: { id: 1, name: "Sparrow", rank: 3, rating: 1900, tier: { label: "Grandmaster", color: [255, 140, 0] } } },
      { name: "Nova", state: "found", standing: { id: 2, name: "Nova", rank: null, rating: 1500, tier: null } },
      { name: "Mochi", state: "unknown", standing: null },
    ];
    const roster = overlayModel(feed({ ratings }), log, NOW).roster!;
    expect(roster.players.map((p) => [p.name, p.rating, p.state, p.doubled, p.host])).toEqual([
      ["Sparrow", 1900, "found", false, true],
      ["Nova", 1500, "found", false, false],
      ["Mochi", null, "unknown", false, false],
      ["Ghost", null, "pending", true, false],
      ["Ghost", null, "pending", true, false],
    ]);
    expect(roster.average).toBe(1700);
    expect(roster.newPlayers).toBe(1);
    expect(roster.doubled).toBe(true);
  });
});

describe("after the match", () => {
  const log = readLiveLog(example);

  it("shows the last round's finishing order", () => {
    const model = overlayModel(feed(), log, NOW);
    expect(model.roundResult).toEqual({
      number: 3,
      result: "WIN",
      order: [
        { position: 1, name: "Sparrow", left: false },
        { position: 2, name: "Ghost", left: false },
        { position: 3, name: "Ghost", left: false },
        { position: 4, name: "Mochi", left: false },
        { position: 5, name: "Nova", left: false },
      ],
    });
    expect(model.logging.title).toBe("Match over");
  });

  it("sums the match up, with the server's rating changes once it rated it", () => {
    const unrated = overlayModel(feed(), log, NOW).matchSummary!;
    expect(unrated.rated).toBe(false);
    expect(unrated.rounds).toBe(3);
    expect(unrated.topSpeed).toEqual({ name: "Nova", speed: 29 });
    expect(unrated.rows[0]).toMatchObject({ place: 1, name: "Sparrow", wins: 2, kills: 5, ratingAfter: null });

    const result: MatchResult = {
      id: 12,
      playedAt: null,
      rounds: 3,
      ratedRounds: 3,
      players: [{ name: "sparrow", roundWins: 2, kills: 5, place: 1, ratingBefore: 1500, ratingAfter: 1531 }],
    };
    const rated = overlayModel(feed({ result }), log, NOW).matchSummary!;
    expect(rated.rated).toBe(true);
    expect(rated.rows[0]).toMatchObject({ ratingBefore: 1500, ratingAfter: 1531 });
  });

  it("goes idle once the log is quiet", () => {
    const model = overlayModel(feed({ logWrittenAt: secondsAgo(600) }), log, NOW);
    expect(model.logging).toEqual({ tone: "idle", title: "Lobby idle", detail: "Log quiet for 10 min" });
  });

  it("adds up the session, with the host's own rating change", () => {
    const result = (id: number, before: number, after: number): MatchResult => ({
      id,
      playedAt: null,
      rounds: 4,
      ratedRounds: 4,
      players: [
        { name: "Sparrow", roundWins: 1, kills: 1, place: 1, ratingBefore: before, ratingAfter: after },
        { name: `Other${id}`, roundWins: 0, kills: 0, place: 2, ratingBefore: 1500, ratingAfter: 1490 },
      ],
    });
    const session = overlayModel(feed({ session: { since: secondsAgo(3600), matches: 3, results: [result(1, 1500, 1520), result(2, 1520, 1510)] } }), log, NOW).session;
    expect(session).toEqual({ matches: 3, rounds: 8, players: 3, host: "Sparrow", hostChange: 10 });
    expect(overlayModel(feed(), log, NOW).session).toBeNull();
  });
});

describe("a tourney match", () => {
  const tourneys: Feed["tourneys"] = [{ lobbyKey: "073518264903", tourney: "October Cup", label: "Lobby 1/2", roundLimit: 3, needsScreenshot: false }];

  it("counts every round started toward the limit", () => {
    // In round 3, after the restarted round 2.
    const model = overlayModel(feed({ tourneys }), readLiveLog(upTo(tourneyExample, 33)), NOW);
    expect(model.tourney).toMatchObject({ name: "October Cup", label: "Lobby 1/2", round: 3, limit: 3, left: 0, ended: false });
  });

  it("asks for the verify screenshot once the final standings show", () => {
    const model = overlayModel(feed({ tourneys }), readLiveLog(tourneyExample), NOW);
    expect(model.tourney!.ended).toBe(true);
    expect(model.tourney!.standings[0]).toMatchObject({ place: 1, name: "Tidal", wins: 2 });
  });

  it("names an unknown lobby by its key", () => {
    const model = overlayModel(feed(), readLiveLog(tourneyExample), NOW);
    expect(model.tourney).toMatchObject({ name: "Tourney match", label: "Lobby 073518264903", limit: 3 });
  });
});

describe("host status", () => {
  const log = readLiveLog(upTo(example, 31));

  it("says when there's no log, or no ranked lines in it", () => {
    expect(overlayModel(feed({ log: null }), readLiveLog(null), NOW).logging.title).toBe("No Workshop log yet");
    expect(overlayModel(feed({ logError: "The Workshop log folder isn't there yet" }), readLiveLog(null), NOW).logging.tone).toBe("bad");
    expect(overlayModel(feed(), readLiveLog("[00:00:01] some inspector line\n"), NOW).logging.title).toBe("Log without ranked lines");
  });

  it("says why a match won't count", () => {
    const unranked = upTo(example, 4) + "[00:00:02] UNRANKED|2.38|BOT\n";
    expect(overlayModel(feed(), readLiveLog(unranked), NOW).logging).toMatchObject({ tone: "bad", title: "Unranked: won't count", detail: "a dummy bot is in the match" });
  });

  it("puts what holds uploads up first", () => {
    const host = (change: Partial<Feed["host"]>) => ({ ...feed().host, ...change });
    expect(overlayModel(feed({ host: host({ problem: { kind: "noToken" }, retrying: "x" }) }), log, NOW).uploads).toMatchObject({ tone: "bad", detail: "No host token" });
    expect(overlayModel(feed({ host: host({ retrying: "Couldn't reach the server" }) }), log, NOW).uploads.tone).toBe("warn");
    const answered = (status: string) =>
      host({ last: { file: "Log-a.txt", at: secondsAgo(5), queued: null, answer: { kind: "answered", result: "stored", matches: [{ matchKey: "1", status, rejection: null }] } } });
    expect(overlayModel(feed({ host: answered("accepted") }), log, NOW).uploads.title).toBe("Uploaded and counted");
    expect(overlayModel(feed({ host: answered("review") }), log, NOW).uploads.tone).toBe("warn");
  });

  it("warns once the ranked code's rank tags may be stale", () => {
    expect(overlayModel(feed(), log, NOW).rankedCode).toEqual({ tone: "good", title: "Ranked code copied 10 min ago", detail: null });
    expect(overlayModel(feed({ rankedCode: { builtAt: secondsAgo(7 * 3600), staleSecs: 6 * 3600 } }), log, NOW).rankedCode.tone).toBe("warn");
    expect(overlayModel(feed({ rankedCode: { builtAt: null, staleSecs: 1 } }), log, NOW).rankedCode.title).toBe("Ranked code not copied");
  });

  it("shows AFK with its hotkey", () => {
    expect(overlayModel(feed({ host: { ...feed().host, afk: true } }), log, NOW).afk).toEqual({ on: true, keys: "Ctrl+Alt+A" });
  });
});

it("says how long ago, shortly", () => {
  expect([ago(5), ago(125), ago(7300)]).toEqual(["5 s", "2 min", "2 h"]);
});
