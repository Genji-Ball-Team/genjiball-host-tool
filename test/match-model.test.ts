import { describe, expect, it } from "vitest";
import { example } from "./fixture";
import { readLog, type MatchView } from "../src/match-model";

function only(text: string): MatchView {
  const { matches } = readLog(text);
  expect(matches).toHaveLength(1);
  return matches[0]!;
}

const standings = (m: MatchView) => m.players.map((p) => [p.place, p.name, p.roundWins, p.kills, p.deflects, p.longestStreak, p.left]);

/** Each row's cell in a round as the site's grid shows it: L, the place, W for the winner, or "–". */
const column = (m: MatchView, round: number) =>
  m.rounds[round]!.cells.map((c) => (c === null ? "" : c.left ? "L" : c.won ? "W" : String(c.position ?? "–")));

describe("the example log", () => {
  const match = only(example);

  it("reads the match's settings", () => {
    expect(match).toMatchObject({ matchKey: "482913507226", gameVersion: "1.3.3R", map: "Workshop Island Night", preset: "Default", addOns: [], ended: true, live: null });
  });

  it("ranks the players as the site does: round wins, then kills, leavers last", () => {
    expect(standings(match)).toEqual([
      [1, "Sparrow", 2, 5, 6, 1, false],
      [2, "Nova", 1, 3, 3, 1, false],
      [3, "Ghost", 0, 1, 3, 0, false],
      [3, "Ghost", 0, 1, 4, 0, false],
      [5, "Mochi", 0, 0, 1, 0, false],
      [5, "Tidal", 0, 0, 1, 0, true],
    ]);
    expect(match.players[0]!.host).toBe(true);
    expect(match.players.map((p) => p.ids)).toEqual([[1], [6], [4], [5], [3], [2]]);
  });

  it("places each player in each round, the leaver marked", () => {
    expect(match.rounds.map((r) => [r.number, r.result, r.rated, r.broken])).toEqual([
      [1, "WIN", true, []],
      [2, "WIN", true, []],
      [3, "WIN", true, []],
    ]);
    // Rows: Sparrow, Nova, Ghost (4), Ghost (5), Mochi, Tidal. Nova joined after round 1.
    expect(column(match, 0)).toEqual(["W", "", "4", "2", "3", "L"]);
    expect(column(match, 1)).toEqual(["3", "W", "4", "2", "5", ""]);
    expect(column(match, 2)).toEqual(["W", "5", "2", "3", "4", ""]);
    expect(match.rounds[0]!.cells[0]).toEqual({ position: 1, won: true, left: false, kills: 2, deflects: 2 });
  });

  it("counts, with a review for the two players named Ghost", () => {
    expect(match.rejected).toEqual([]);
    expect(match.review.map((r) => r.code)).toEqual(["duplicate_name"]);
    expect(match.review[0]!.message).toContain('"Ghost"');
    expect(match.problems).toEqual([]);
  });
});

describe("a match being played", () => {
  // The example cut in round 2, after Mochi and a Ghost went out.
  const lines = example.split("\n");
  const cut = lines.findIndex((l) => l.includes("ELIM|58.40|2|4||4"));
  const match = only(lines.slice(0, cut + 1).join("\n") + "\n");

  it("shows the round in progress, which the parser leaves out", () => {
    expect(match.ended).toBe(false);
    expect(match.rounds.map((r) => r.number)).toEqual([1]);
    expect(match.live).toEqual({
      number: 2,
      alive: ["Sparrow", "Ghost", "Nova"],
      out: [
        { name: "Mochi", by: "Nova" },
        { name: "Ghost", by: null },
      ],
      left: [],
    });
  });

  it("has wins and kills so far", () => {
    expect(match.players.map((p) => [p.name, p.roundWins, p.kills])).toEqual([
      ["Sparrow", 1, 2],
      ["Ghost", 0, 1],
      ["Nova", 0, 1],
      ["Mochi", 0, 0],
      ["Ghost", 0, 0],
      ["Tidal", 0, 0],
    ]);
  });

  it("has no live round between rounds or once it has ended", () => {
    const end = lines.findIndex((l) => l.includes("ROUND_END|43.30"));
    expect(only(lines.slice(0, end + 1).join("\n")).live).toBeNull();
  });
});

describe("why a match won't count", () => {
  it("lists every UNRANKED reason, and no rated round", () => {
    const text = ["GBR|1|1|1.3.3R|1", "MATCH_START|1|other|Custom|1|", "UNRANKED|1|MAP", "UNRANKED|1|PRESET", "JOIN|2|1|Zbozo|1", "UNRANKED|2|BOT", "UNRANKED|2|SOMETHING_NEW", "MATCH_END|9|TIME", ""].join("\n");
    const match = only(text);
    expect(match.rejected).toEqual([
      { code: "unranked", message: "Unranked: the map isn't Workshop Island Night" },
      { code: "unranked", message: "Unranked: the preset isn't Default" },
      { code: "unranked", message: "Unranked: a dummy bot is in the match" },
      { code: "unranked", message: "Unranked: SOMETHING_NEW" },
      { code: "too_few_players", message: "No rated round" },
    ]);
    expect(match.map).toBe("other");
  });

  it("marks NONE and ABORT rounds as not rated", () => {
    const text = ["GBR|1|1|1.3.3R|1", "MATCH_START|1|workshop-island-night|Default|0|", "JOIN|1|1|A|1", "JOIN|1|2|B|", "ROUND_START|2|1|1,2", "ELIM|3|1|1||2", "ELIM|3|1|2||1", "ROUND_END|3|1||NONE", "ROUND_START|4|2|1,2", "ROUND_END|5|2||ABORT", ""].join("\n");
    const match = only(text);
    expect(match.rounds.map((r) => [r.result, r.rated])).toEqual([
      ["NONE", false],
      ["ABORT", false],
    ]);
    expect(match.rejected.map((r) => r.code)).toEqual(["too_few_players"]);
  });

  it("rejects a format it can't read", () => {
    expect(only("GBR|1|99|9.9.9R|1\n").rejected[0]).toMatchObject({ code: "unknown_format" });
  });

  it("knows a legacy log", () => {
    expect(readLog("KILL|1|A|B\n")).toEqual({ matches: [], legacy: true });
  });
});
