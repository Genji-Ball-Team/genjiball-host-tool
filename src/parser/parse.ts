import type {
  DeflectEvent,
  ElimEvent,
  KillEvent,
  ParsedMatch,
  ParsedPlayer,
  ParsedRound,
  ParseResult,
  RoundResult,
} from "./types";

/**
 * Parses a Workshop log file into matches, rounds and events, following GenjiBall-CE
 * `docs/ranked-log.md`. Pure: no D1, no config import, so it's easy to test.
 *
 * - Lines are read top to bottom and ordered by file position, never by `time`.
 * - Each `GBR` line starts a match. Lines before the first `GBR`, after `MATCH_END`, and lines that
 *   aren't ours are ignored.
 * - A match in a format version not in `acceptedFormats` is rejected and its lines are skipped.
 * - Format 2 adds tourney matches: a `TOURNEY` line after `GBR` and `MATCH_END` `ROUNDS`. A format 1
 *   match is read as before, `TOURNEY` lines included (skipped): it's a ranked match.
 */
export function parseLog(text: string, options: ParseOptions): ParseResult {
  const matches: ParsedMatch[] = [];
  let parser: MatchParser | null = null;
  let sawGbr = false;
  let sawKill = false;

  const lines = text.replace(/^\uFEFF/, "").split(/\r?\n/);
  for (const [index, raw] of lines.entries()) {
    const lineNumber = index + 1;
    const fields = stripPrefix(raw).split("|");
    const type = fields[0] ?? "";

    if (type === "GBR") {
      parser?.finish();
      sawGbr = true;
      parser = new MatchParser(fields, lineNumber, options);
      matches.push(parser.match);
      continue;
    }
    if (type === "KILL") sawKill = true;
    if (eventTypes.has(type)) parser?.line(type, fields, lineNumber);
  }
  parser?.finish();

  return { matches, legacy: !sawGbr && sawKill };
}

export interface ParseOptions {
  /** Format versions (`GBR` `format`) this parser may read. From config `acceptedLogFormats`. */
  acceptedFormats: readonly number[];
}

/** The first format version with tourney matches (`TOURNEY`, `MATCH_END` `ROUNDS`). */
const tourneyFormat = 2;

const eventTypes = new Set([
  "TOURNEY",
  "MATCH_START",
  "JOIN",
  "LEAVE",
  "ROUND_START",
  "ELIM",
  "ROUND_END",
  "KILL",
  "DEFLECT",
  "UNRANKED",
  "MATCH_END",
]);

const roundResults = new Set<string>(["WIN", "NONE", "ABORT"]);

/** Removes the Workshop's `[hh:mm:ss] ` prefix. */
export function stripPrefix(line: string): string {
  return line.replace(/^\[\d+:\d{2}:\d{2}\] ?/, "").trimEnd();
}

interface OpenRound {
  number: number;
  startTime: number;
  playerIds: number[];
  elims: ElimEvent[];
  leftIds: number[];
}

class MatchParser {
  readonly match: ParsedMatch;
  private readonly players = new Map<number, ParsedPlayer>();
  private round: OpenRound | null = null;
  /** Lines after `MATCH_END` or in an unreadable format aren't part of the match. */
  private closed = false;
  /** File line being read, for problems found later than their line (a round never closed). */
  private lineNumber: number;
  /** Number of the last `ROUND_START`, ended or not: a tourney match ends after round `roundLimit`. */
  private lastRound: number | null = null;

  constructor(fields: string[], lineNumber: number, options: ParseOptions) {
    const formatField = fields[2] ?? "";
    const format = int(formatField);
    this.lineNumber = lineNumber;
    this.match = {
      format: format ?? 0,
      gameVersion: fields[3] ?? "",
      matchKey: fields[4] ?? "",
      startLine: lineNumber,
      lineCount: 1,
      rejection: null,
      unranked: [],
      review: [],
      tourney: null,
      settings: null,
      startTime: null,
      endResult: null,
      endTime: null,
      players: [],
      rounds: [],
      kills: [],
      deflects: [],
      problems: [],
    };
    if (format === null || !options.acceptedFormats.includes(format)) {
      this.match.rejection = {
        code: "unknown_format",
        message: `Log format "${formatField}" isn't supported (accepted: ${options.acceptedFormats.join(", ")})`,
      };
      this.closed = true;
      return;
    }
    if (!this.match.matchKey) this.problem(lineNumber, "GBR has no matchKey");
  }

  line(type: string, fields: string[], lineNumber: number): void {
    if (this.closed) return;
    // Format 1 has no tourney matches: its parser skipped the line, as an unknown one.
    if (type === "TOURNEY" && this.match.format < tourneyFormat) return;
    this.lineNumber = lineNumber;
    this.match.lineCount++;
    const time = num(fields[1]);
    if (time === null) {
      this.problem(lineNumber, `${type} has no valid time`);
      return;
    }
    const error = this.event(type, time, fields.slice(2));
    if (error) this.problem(lineNumber, `${type}: ${error}`);
  }

  /** Applies one event. Returns why the line was skipped, if it was. */
  private event(type: string, time: number, f: string[]): string | void {
    switch (type) {
      case "TOURNEY": {
        if (this.match.tourney) return "second TOURNEY in the match";
        const lobbyKey = f[0] ?? "";
        const limit = int(f[1]);
        // Read even when something's off: a match with a TOURNEY line is never a ranked one.
        this.match.tourney = { lobbyKey, roundLimit: limit !== null && limit >= 1 ? limit : null };
        if (this.match.settings) this.problem(this.lineNumber, "TOURNEY: after MATCH_START (read anyway)");
        if (!/^\d+$/.test(lobbyKey)) this.problem(this.lineNumber, `TOURNEY: lobbyKey "${lobbyKey}" isn't digits`);
        if (this.match.tourney.roundLimit === null) this.problem(this.lineNumber, `TOURNEY: roundLimit "${f[1] ?? ""}" isn't a whole number from 1`);
        return;
      }

      case "MATCH_START": {
        if (this.match.settings) return "second MATCH_START in the match";
        this.match.settings = {
          map: f[0] ?? "",
          preset: f[1] ?? "",
          feel: f[2] === "1",
          addOns: list(f[3]),
        };
        this.match.startTime = time;
        return;
      }

      case "JOIN": {
        const id = int(f[0]);
        const name = f[1] ?? "";
        if (id === null) return "bad player id";
        if (this.players.has(id)) return `player id ${id} joined twice`;
        const present = [...this.players.values()].some((p) => p.leaveTime === null && p.name === name);
        if (present && !this.match.review.includes("duplicate_name")) this.match.review.push("duplicate_name");
        // `host` was added at the end of the line in format 1: older logs don't have it (no host known).
        const player: ParsedPlayer = { id, name, joinTime: time, leaveTime: null, host: f[2] === "1" };
        this.players.set(id, player);
        this.match.players.push(player);
        return;
      }

      case "LEAVE": {
        const id = int(f[0]);
        const player = id === null ? undefined : this.players.get(id);
        if (id === null || !player) return "unknown player";
        if (player.leaveTime !== null) return `player ${id} left twice`;
        player.leaveTime = time;
        const round = this.round;
        if (round?.playerIds.includes(id) && !round.elims.some((e) => e.id === id)) round.leftIds.push(id);
        return;
      }

      case "ROUND_START": {
        const number = int(f[0]);
        const ids = list(f[1]).map(int);
        if (number === null) return "bad round number";
        if (ids.some((id) => id === null)) return "bad player id list";
        if (this.round) this.dropRound(`round ${this.round.number} has no ROUND_END before round ${number}`);
        const playerIds = ids as number[];
        const unknown = playerIds.filter((id) => !this.players.has(id));
        if (unknown.length) return `round ${number} lists players that never joined: ${unknown.join(", ")}`;
        this.round = { number, startTime: time, playerIds, elims: [], leftIds: [] };
        this.lastRound = number;
        return;
      }

      case "ELIM": {
        const round = this.currentRound(f[0]);
        if (typeof round === "string") return round;
        const id = int(f[1]);
        const place = int(f[3]);
        if (id === null || place === null) return "bad player id or place";
        if (!round.playerIds.includes(id)) return `player ${id} isn't in round ${round.number}`;
        if (round.elims.some((e) => e.id === id)) return `player ${id} eliminated twice`;
        if (round.leftIds.includes(id)) return `player ${id} eliminated after leaving`;
        round.elims.push({ time, id, killerId: optionalInt(f[2]), place });
        return;
      }

      case "ROUND_END": {
        const round = this.currentRound(f[0]);
        if (typeof round === "string") return round;
        const result = f[2] ?? "";
        if (!roundResults.has(result)) return `unknown result "${result}"`;
        this.round = null;
        this.match.rounds.push(closeRound(round, time, result as RoundResult, optionalInt(f[1])));
        return;
      }

      case "KILL": {
        const victimName = f[1];
        if (victimName === undefined) return "missing victim";
        const kill: KillEvent = {
          time,
          attackerName: f[0] || null,
          victimName,
          attackerId: optionalInt(f[2]),
          victimId: optionalInt(f[3]),
          round: this.round?.number ?? null,
        };
        this.match.kills.push(kill);
        return;
      }

      case "DEFLECT": {
        const round = int(f[0]);
        const id = int(f[1]);
        const speed = num(f[2]);
        if (round === null || id === null || speed === null) return "bad round, player id or speed";
        const deflect: DeflectEvent = { time, round, id, speed, targetId: optionalInt(f[3]) };
        this.match.deflects.push(deflect);
        return;
      }

      case "UNRANKED": {
        const reason = f[0] ?? "";
        if (!this.match.unranked.includes(reason)) this.match.unranked.push(reason);
        this.match.rejection ??= { code: "unranked", message: `Unranked match: ${reason}` };
        return;
      }

      case "MATCH_END": {
        if (this.round) this.dropRound(`round ${this.round.number} has no ROUND_END before MATCH_END`);
        const result = f[0] ?? "";
        this.match.endResult = result;
        this.match.endTime = time;
        this.closed = true;
        if (this.match.format >= tourneyFormat) return this.checkEnd(result);
        return;
      }
    }
  }

  /**
   * A format 2 `MATCH_END` that doesn't fit the match: `TIME` ends a ranked match, `ROUNDS` a tourney
   * match after round `roundLimit`. Only reported: the match ended either way.
   */
  private checkEnd(result: string): string | void {
    const tourney = this.match.tourney;
    const expected = tourney ? "ROUNDS" : "TIME";
    if (result !== expected) return `"${result}" ends a ${tourney ? "tourney" : "ranked"} match, expected ${expected}`;
    if (tourney && tourney.roundLimit !== null && this.lastRound !== tourney.roundLimit) {
      return `ROUNDS after round ${this.lastRound ?? "(none)"}, the roundLimit is ${tourney.roundLimit}`;
    }
  }

  /** The open round, if `field` is its number; otherwise the reason it isn't. */
  private currentRound(field: string | undefined): OpenRound | string {
    const number = int(field);
    if (number === null) return "bad round number";
    if (!this.round) return `round ${number} isn't in progress`;
    if (this.round.number !== number) return `round ${number} isn't the round in progress (${this.round.number})`;
    return this.round;
  }

  private dropRound(message: string): void {
    this.round = null;
    this.problem(this.lineNumber, message);
  }

  private problem(line: number, message: string): void {
    this.match.problems.push({ line, message });
  }

  /** End of the match or the file. An open round is dropped: only finished rounds count. */
  finish(): void {
    if (this.round && !this.closed) this.dropRound(`round ${this.round.number} has no ROUND_END (file ended)`);
    this.round = null;
    this.closed = true;
  }
}

function closeRound(round: OpenRound, endTime: number, result: RoundResult, winnerId: number | null): ParsedRound {
  const broken: string[] = [];
  let finishingOrder: number[] | null = null;

  if (result === "WIN") {
    if (winnerId === null || !round.playerIds.includes(winnerId)) {
      broken.push(`winner ${winnerId ?? "(none)"} isn't in the round`);
    } else if (round.elims.some((e) => e.id === winnerId) || round.leftIds.includes(winnerId)) {
      broken.push(`winner ${winnerId} was eliminated or left`);
    }
    const accounted = new Set([...round.elims.map((e) => e.id), ...round.leftIds, winnerId]);
    for (const id of round.playerIds) {
      if (!accounted.has(id)) broken.push(`player ${id} has no ELIM, LEAVE or win`);
    }
    if (!broken.length && winnerId !== null) {
      finishingOrder = [winnerId, ...round.elims.map((e) => e.id).reverse()];
    }
  }

  return {
    number: round.number,
    startTime: round.startTime,
    endTime,
    result,
    playerIds: round.playerIds,
    elims: round.elims,
    leftIds: round.leftIds,
    afkIds: [],
    winnerId: result === "WIN" ? winnerId : null,
    finishingOrder,
    broken,
  };
}

/** A number with `.` or `,` as the decimal mark, or `null`. */
export function num(field: string | undefined): number | null {
  if (field === undefined || !/^-?\d+(?:[.,]\d+)?$/.test(field)) return null;
  return Number(field.replace(",", "."));
}

/** A whole number, or `null`. */
function int(field: string | undefined): number | null {
  return field !== undefined && /^\d+$/.test(field) ? Number(field) : null;
}

/** An id field that may be empty ("none"). */
function optionalInt(field: string | undefined): number | null {
  return field ? int(field) : null;
}

function list(field: string | undefined): string[] {
  return field ? field.split(",").filter((item) => item !== "") : [];
}
