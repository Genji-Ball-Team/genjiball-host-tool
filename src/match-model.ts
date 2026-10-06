import { parseLog, stripPrefix } from "./parser/parse";
import type { ParsedMatch, ParsedPlayer, ParsedRound, RoundResult } from "./parser/types";
import { acceptedLogFormats, mapNames, minMatchPlayers, minRatedRoundPlayers, unrankedReasons } from "./match-config";

/**
 * A log's matches as the site's match page shows them, read with the server's own parser
 * (`src/parser/`, a copy of genjiball-ranked's). Pure: no DOM, so it's tested on the spec's example
 * log. The stats follow genjiball-ranked `src/site/handler.ts` `matchView`, and what the server will
 * make of the match `src/upload/plan.ts` `judge`. Ratings are the server's alone: none here.
 */
export interface LogView {
  matches: MatchView[];
  /** A v1.3.2 log (`KILL` lines, no `GBR`): never ranked. */
  legacy: boolean;
}

export interface MatchView {
  matchKey: string;
  gameVersion: string;
  /** As the site names it; `null` before `MATCH_START`. */
  map: string | null;
  preset: string | null;
  addOns: string[];
  /** `MATCH_END` was read: the match is over. */
  ended: boolean;
  /** By overall place, as the site ranks them: most rounds won, then most kills; leavers last among a shared place. */
  players: PlayerView[];
  /** Finished rounds, in order. */
  rounds: RoundView[];
  /** The round being played, in the last match of a file that hasn't ended. */
  live: LiveRound | null;
  /** Why the server will reject the match. Empty: it counts (maybe after a review). */
  rejected: Reason[];
  /** Why an admin must look at it before it counts. */
  review: Reason[];
  /** Lines the parser skipped, and why. */
  problems: { line: number; message: string }[];
}

export interface Reason {
  /** The server's code: `unranked`, `too_few_players`, `duplicate_name`… */
  code: string;
  message: string;
}

/**
 * One row of the match: a player's stays in it. A player who left and came back (a new id) is one
 * row; two players with the same name at once (sent to review) are two.
 */
export interface PlayerView {
  name: string;
  /** Their log ids, one per stay. */
  ids: number[];
  /** The lobby host (`JOIN` `host`). */
  host: boolean;
  /** Left and didn't come back. */
  left: boolean;
  /** Overall: 1 + players with more round wins, or as many and more kills. */
  place: number;
  /** `WIN` rounds won, rated or not. */
  roundWins: number;
  /** `KILL` lines with them as attacker, not counting themself. */
  kills: number;
  deflects: number;
  /** Most `WIN` rounds won in a row, of those they played to the end. */
  longestStreak: number;
}

export interface RoundView {
  number: number;
  result: RoundResult;
  /** A finishing order of at least `minRatedRoundPlayers`. */
  rated: boolean;
  /** Why an ended round can't be rated. */
  broken: string[];
  /** One per row of `players`, in the same order; `null` when they weren't in the round. */
  cells: (RoundCell | null)[];
}

export interface RoundCell {
  /** Place in the finishing order of a rated round, else `null`. */
  position: number | null;
  won: boolean;
  /** Left during the round: last, out of the finishing order. */
  left: boolean;
  kills: number;
  deflects: number;
}

export interface LiveRound {
  number: number;
  /** Still in, in the order `ROUND_START` lists them. */
  alive: string[];
  /** Out so far, first out first, and who sent the ball (`null`: no one, or themself). */
  out: { name: string; by: string | null }[];
  left: string[];
}

export function readLog(text: string): LogView {
  const parsed = parseLog(text, { acceptedFormats: acceptedLogFormats });
  const last = parsed.matches.length - 1;
  return {
    matches: parsed.matches.map((match, i) => matchView(match, i === last ? openRound(text, match) : null)),
    legacy: parsed.legacy,
  };
}

function matchView(match: ParsedMatch, live: OpenRound | null): MatchView {
  const rows = playerRows(match.players);
  const rowOf = new Map<number, Row>();
  for (const row of rows) for (const id of row.ids) rowOf.set(id, row);
  const nameOf = (id: number | null) => (id === null ? null : (rowOf.get(id)?.name ?? null));

  for (const kill of match.kills) {
    if (kill.attackerId !== null) countFor(rowOf, kill.attackerId, (row) => row.kills++);
  }
  for (const deflect of match.deflects) countFor(rowOf, deflect.id, (row) => row.deflects++);

  // Each row's stats, then the overall order, then each round's cells in that order.
  const streaks = new Map<Row, number>();
  for (const round of match.rounds) {
    if (round.result !== "WIN") continue;
    for (const id of round.playerIds) {
      const row = rowOf.get(id);
      if (!row || round.leftIds.includes(id)) continue;
      const won = id === round.winnerId;
      if (won) row.roundWins++;
      const streak = won ? (streaks.get(row) ?? 0) + 1 : 0;
      streaks.set(row, streak);
      row.longestStreak = Math.max(row.longestStreak, streak);
    }
  }
  const players = rows
    .map((row) => ({ ...row, place: 1 + rows.filter((o) => o.roundWins > row.roundWins || (o.roundWins === row.roundWins && o.kills > row.kills)).length }))
    // Leavers last among a shared place.
    .sort((a, b) => a.place - b.place || Number(a.left) - Number(b.left));

  return {
    matchKey: match.matchKey,
    gameVersion: match.gameVersion,
    map: match.settings ? (mapNames[match.settings.map] ?? match.settings.map) : null,
    preset: match.settings?.preset ?? null,
    addOns: match.settings?.addOns ?? [],
    ended: match.endResult !== null,
    players,
    rounds: match.rounds.map((round) => roundView(round, players, match)),
    live: live && {
      number: live.number,
      alive: live.playerIds.filter((id) => !live.elims.some((e) => e.id === id) && !live.leftIds.includes(id)).map((id) => nameOf(id) ?? `#${id}`),
      out: live.elims.map((e) => ({ name: nameOf(e.id) ?? `#${e.id}`, by: nameOf(e.killerId) })),
      left: live.leftIds.map((id) => nameOf(id) ?? `#${id}`),
    },
    rejected: rejections(match),
    review: reviews(match),
    problems: match.problems,
  };
}

type Row = Omit<PlayerView, "place">;

/** Rows in join order: a stay joins the row of a player with the same name who has left. */
function playerRows(players: ParsedPlayer[]): Row[] {
  const rows: Row[] = [];
  const stays = new Map<number, ParsedPlayer>(players.map((p) => [p.id, p]));
  for (const player of players) {
    const back = rows.find((row) => nameKey(row.name) === nameKey(player.name) && row.ids.every((id) => stays.get(id)?.leaveTime !== null));
    if (back) {
      back.ids.push(player.id);
      back.host ||= player.host;
      back.left = player.leaveTime !== null;
      continue;
    }
    rows.push({ name: player.name, ids: [player.id], host: player.host, left: player.leaveTime !== null, roundWins: 0, kills: 0, deflects: 0, longestStreak: 0 });
  }
  return rows;
}

/** Names map to players ignoring case, as on the server (`src/upload/plan.ts` `nameKey`). */
function nameKey(name: string): string {
  return name.toLowerCase();
}

function countFor(rowOf: Map<number, Row>, id: number, add: (row: Row) => void): void {
  const row = rowOf.get(id);
  if (row) add(row);
}

function isRated(order: readonly number[] | null): order is number[] {
  return order !== null && order.length >= minRatedRoundPlayers;
}

function roundView(round: ParsedRound, players: PlayerView[], match: ParsedMatch): RoundView {
  const rated = isRated(round.finishingOrder);
  return {
    number: round.number,
    result: round.result,
    rated,
    broken: round.broken,
    cells: players.map((player) => {
      const id = round.playerIds.find((i) => player.ids.includes(i));
      if (id === undefined) return null;
      const position = rated ? round.finishingOrder!.indexOf(id) + 1 : 0;
      return {
        position: position || null,
        won: round.result === "WIN" && round.winnerId === id,
        left: round.leftIds.includes(id),
        kills: match.kills.filter((k) => k.round === round.number && k.attackerId === id).length,
        deflects: match.deflects.filter((d) => d.round === round.number && d.id === id).length,
      };
    }),
  };
}

/** What the server will reject the match for: every reason, not only the first it reports. */
function rejections(match: ParsedMatch): Reason[] {
  const reasons: Reason[] = [];
  if (!match.matchKey) reasons.push({ code: "no_match_key", message: "The log has no match key" });
  if (match.rejection?.code === "unknown_format") reasons.push(match.rejection);
  for (const reason of match.unranked) {
    reasons.push({ code: "unranked", message: `Unranked: ${unrankedReasons[reason] ?? reason}` });
  }
  const rated = new Set<number>();
  for (const round of match.rounds) if (isRated(round.finishingOrder)) round.finishingOrder.forEach((id) => rated.add(id));
  if (rated.size < minMatchPlayers) {
    const message = rated.size === 0 ? "No rated round" : `${rated.size} players in rated rounds, the minimum is ${minMatchPlayers}`;
    reasons.push({ code: "too_few_players", message });
  }
  return reasons;
}

/** Why an admin will review it. An untrusted host's every match is too: the window says so by the host's name. */
function reviews(match: ParsedMatch): Reason[] {
  if (!match.review.includes("duplicate_name")) return [];
  // The names two players had at once, found as the parser does: a `JOIN` with the name of a player
  // who hadn't left. The file is read to the end, so "hadn't left" is a later (or no) `LEAVE`.
  const names = new Set<string>();
  match.players.forEach((p, i) => {
    if (match.players.slice(0, i).some((o) => o.name === p.name && (o.leaveTime === null || o.leaveTime > p.joinTime))) names.add(p.name);
  });
  const list = [...names].map((n) => `"${n}"`).join(", ");
  return [{ code: "duplicate_name", message: `Two players named ${list || "the same"} at once: an admin checks the match before it counts` }];
}

interface OpenRound {
  number: number;
  playerIds: number[];
  elims: { id: number; killerId: number | null }[];
  leftIds: number[];
}

/**
 * The round in progress at the end of the file, which the parser drops (only finished rounds count).
 * Reads the match's lines after its `GBR` the way the parser does; `null` once the match has ended,
 * or when the server can't read its format (it has no rounds at all then).
 */
function openRound(text: string, match: ParsedMatch): OpenRound | null {
  if (match.endResult !== null || match.rejection?.code === "unknown_format") return null;
  let round: OpenRound | null = null;
  const lines = text.replace(/^\uFEFF/, "").split(/\r?\n/).slice(match.startLine);
  for (const line of lines) {
    const [type, , ...f] = stripPrefix(line).split("|");
    if (type === "ROUND_START") {
      const number = whole(f[0]);
      const ids = (f[1] ?? "").split(",").filter(Boolean).map(whole);
      round = number === null || ids.some((id) => id === null) ? null : { number, playerIds: ids as number[], elims: [], leftIds: [] };
    } else if (type === "ROUND_END") {
      round = null;
    } else if (type === "ELIM" && round && whole(f[0]) === round.number) {
      const id = whole(f[1]);
      if (id !== null && round.playerIds.includes(id)) round.elims.push({ id, killerId: f[2] ? whole(f[2]) : null });
    } else if (type === "LEAVE" && round) {
      const id = whole(f[0]);
      if (id !== null && round.playerIds.includes(id) && !round.elims.some((e) => e.id === id)) round.leftIds.push(id);
    }
  }
  return round;
}

function whole(field: string | undefined): number | null {
  return field !== undefined && /^\d+$/.test(field) ? Number(field) : null;
}
