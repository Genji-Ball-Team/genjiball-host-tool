/**
 * What the parser makes of a Workshop log file. Field names follow GenjiBall-CE `docs/ranked-log.md`.
 * Times are the raw `time` field (Total Time Elapsed, seconds), not relative to `MATCH_START`.
 */

export interface ParseResult {
  /** Every match in the file, in file order, starting at each `GBR` line. */
  matches: ParsedMatch[];
  /** The file has `KILL` lines but no `GBR` line: a v1.3.2 log, for the legacy parser. */
  legacy: boolean;
}

export interface ParsedMatch {
  format: number;
  gameVersion: string;
  /** 12 digits, kept as text. Same host + same key = the same match, possibly in several files. */
  matchKey: string;
  /** 1-based line of the `GBR` line in the file. */
  startLine: number;
  /** Lines of ours in this match, from `GBR` on. Of several copies of one match, keep the longest. */
  lineCount: number;

  /** Set when the match must not count. It's still parsed (for debugging), but never rated. An unknown format isn't parsed at all. */
  rejection: Rejection | null;
  /** `UNRANKED` reasons, in the order they were logged. Any reason rejects the match. */
  unranked: string[];
  /** Reasons an admin must look at the match before it counts (`duplicate_name`). */
  review: ReviewReason[];

  /** `null` until `MATCH_START`. */
  settings: MatchSettings | null;
  /** `time` of `MATCH_START`. */
  startTime: number | null;
  /** `MATCH_END` result (`TIME`), or `null` when the file ended first. */
  endResult: string | null;
  endTime: number | null;

  players: ParsedPlayer[];
  /** Rounds with a `ROUND_END`. A round cut off by the end of the match or file is dropped. */
  rounds: ParsedRound[];
  kills: KillEvent[];
  deflects: DeflectEvent[];

  /** Lines that were ours but didn't make sense. They are skipped, and listed here. */
  problems: Problem[];
}

export interface MatchSettings {
  map: string;
  preset: string;
  feel: boolean;
  addOns: string[];
}

export interface ParsedPlayer {
  /** Per-match id from `JOIN`. */
  id: number;
  name: string;
  joinTime: number;
  /** `time` of the `LEAVE`, or `null` if they stayed to the end. */
  leaveTime: number | null;
  /** `JOIN` `host` is `1`: the lobby host (a host who rejoins has a new id, also marked). Always false in logs without the field. */
  host: boolean;
}

export type RoundResult = "WIN" | "NONE" | "ABORT";

export interface ParsedRound {
  number: number;
  startTime: number;
  endTime: number;
  result: RoundResult;
  /** Ids listed in `ROUND_START`. */
  playerIds: number[];
  /** In the order they went out. */
  elims: ElimEvent[];
  /** Listed players who left during the round. They are dropped from the finishing order. */
  leftIds: number[];
  /**
   * Host ids dropped from the round because the host was AFK (`X-Host-Afk`, src/upload/hostAfk.ts),
   * like leavers. Always empty from the parser: the log doesn't say.
   */
  afkIds: number[];
  /** Winner id for a `WIN` round. */
  winnerId: number | null;
  /**
   * For a `WIN` round that isn't broken: winner first, then the eliminated from last out to first,
   * without the players who left (or the host, in a round they were AFK in). `null` for `NONE`, `ABORT` and broken rounds.
   */
  finishingOrder: number[] | null;
  /** Why the round can't be rated even though it ended (a listed player with no `ELIM`, `LEAVE` or win). */
  broken: string[];
}

export interface ElimEvent {
  time: number;
  id: number;
  /** `null` when there's no killer or it was the player themself. */
  killerId: number | null;
  place: number;
}

export interface KillEvent {
  time: number;
  attackerName: string | null;
  victimName: string;
  /** `null` when there's no attacker or the victim killed themself. Count kills only when set. */
  attackerId: number | null;
  /** `null` for a player who has no id yet (killed on joining, before spawning). */
  victimId: number | null;
  /** Round in progress when it happened, or `null` between rounds. */
  round: number | null;
}

export interface DeflectEvent {
  time: number;
  round: number;
  id: number;
  speed: number;
  targetId: number | null;
}

export interface Rejection {
  code: "unranked" | "unknown_format";
  message: string;
}

export type ReviewReason = "duplicate_name";

export interface Problem {
  /** 1-based line in the file. */
  line: number;
  message: string;
}
