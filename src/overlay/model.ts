import { parseLog } from "../parser/parse";
import type { ParsedMatch, ParsedRound } from "../parser/types";
import { acceptedLogFormats, unrankedReasons } from "../match-config";
import { readLog, type MatchView } from "../match-model";
import type { Feed, MatchResult, PlayerRating, Standing } from "./feed";

/**
 * What each overlay widget says (#50–#54), from the live log and the feed. Pure: no DOM, so it's
 * tested on the spec's example logs. The log is read with the server's parser, like the Match view
 * (`match-model.ts`), and the standings rank as the site does. Ratings are the server's alone.
 */

export type Tone = "good" | "warn" | "bad" | "idle";

/** A host status line (#50). */
export interface StatusLine {
  tone: Tone;
  title: string;
  detail: string | null;
}

export interface RosterPlayer {
  name: string;
  host: boolean;
  state: PlayerRating["state"];
  rating: number | null;
  rank: number | null;
  tier: Standing["tier"];
  /** Another player in the lobby has the same name: the match goes to review. */
  doubled: boolean;
}

export interface Roster {
  players: RosterPlayer[];
  /** Of the players with a rating. */
  average: number | null;
  newPlayers: number;
  doubled: boolean;
}

export interface Kill {
  /** Stable across polls, for the feed's slide-in. */
  key: string;
  killer: string | null;
  victim: string;
  /** Who touched the ball since the kill before, in order, the killer last. */
  chain: string[];
  /** The ball's speed after the last deflect. */
  speed: number | null;
}

export interface Placing {
  place: number;
  name: string;
  wins: number;
  kills: number;
  left: boolean;
}

export interface RoundResult {
  number: number;
  result: ParsedRound["result"];
  /** Finishing order of a `WIN` round, the winner first; leavers last. */
  order: { position: number | null; name: string; left: boolean }[];
}

export interface SummaryRow extends Placing {
  ratingBefore: number | null;
  ratingAfter: number | null;
}

export interface MatchSummary {
  rows: SummaryRow[];
  rounds: number;
  topSpeed: { name: string; speed: number } | null;
  /** The server has rated it: each row's rating change is known. */
  rated: boolean;
}

export interface Session {
  matches: number;
  /** Of the matches public on the site. */
  rounds: number;
  players: number;
  host: string | null;
  /** The host's own rating change over those matches, `null` while unknown. */
  hostChange: number | null;
}

export interface TourneyPanel {
  name: string;
  label: string;
  /** The round being played or last started (every `ROUND_START` counts). */
  round: number;
  limit: number | null;
  left: number | null;
  standings: Placing[];
  /** `MATCH_END ROUNDS`: the final standings show now. */
  ended: boolean;
}

export interface OverlayModel {
  logging: StatusLine;
  uploads: StatusLine;
  afk: { on: boolean; keys: string | null };
  logCopies: number | null;
  rankedCode: StatusLine;
  roster: Roster | null;
  eliminations: { round: number; out: { name: string; by: string | null }[] } | null;
  standings: Placing[] | null;
  killFeed: Kill[] | null;
  roundResult: RoundResult | null;
  matchSummary: MatchSummary | null;
  session: Session | null;
  tourney: TourneyPanel | null;
  /** What to ask the feed for next: the lobby's names and the current match. */
  names: string[];
  matchKey: string | null;
}

/** The log parsed once per text: the feed sends it again only when it grew. */
export interface ReadLog {
  match: ParsedMatch | null;
  view: MatchView | null;
}

export function readLiveLog(text: string | null): ReadLog {
  if (!text) return { match: null, view: null };
  const parsed = parseLog(text, { acceptedFormats: acceptedLogFormats });
  const views = readLog(text).matches;
  return { match: parsed.matches.at(-1) ?? null, view: views.at(-1) ?? null };
}

export function overlayModel(feed: Feed, log: ReadLog, now: number): OverlayModel {
  const { match, view } = log;
  const nameOf = names(match);
  const lobby = match?.players.filter((p) => p.leaveTime === null) ?? [];
  const live = view?.live ?? null;
  return {
    logging: logging(feed, match, view, now),
    uploads: uploads(feed),
    afk: { on: feed.host.afk, keys: feed.hotkeys.find((h) => h.action === "afk")?.keys ?? null },
    logCopies: feed.logCopies,
    rankedCode: rankedCode(feed, now),
    roster: match && lobby.length > 0 ? roster(lobby, feed.ratings) : null,
    eliminations: live && { round: live.number, out: live.out },
    standings: view && view.players.length > 0 && (view.rounds.length > 0 || live) ? view.players.map(placing) : null,
    killFeed: match && match.kills.length > 0 ? killFeed(match, nameOf, feed.killFeedShown) : null,
    roundResult: match && view && !live ? roundResult(match.rounds.at(-1), nameOf) : null,
    matchSummary: match && view && match.endResult !== null ? summary(match, view, feed.result, nameOf) : null,
    session: session(feed, match),
    tourney: match?.tourney && view ? tourney(match, view, feed) : null,
    names: lobby.map((p) => p.name),
    matchKey: match?.matchKey || null,
  };
}

function names(match: ParsedMatch | null): (id: number | null) => string | null {
  const byId = new Map(match?.players.map((p) => [p.id, p.name]));
  return (id) => (id === null ? null : (byId.get(id) ?? `#${id}`));
}

function placing(p: MatchView["players"][number]): Placing {
  return { place: p.place, name: p.name, wins: p.roundWins, kills: p.kills, left: p.left };
}

function since(at: string | null, now: number): number | null {
  const time = at ? Date.parse(at) : NaN;
  return Number.isNaN(time) ? null : Math.max(0, (now - time) / 1000);
}

/** "40 s", "12 min", "3 h". */
export function ago(secs: number): string {
  if (secs < 60) return `${Math.round(secs)} s`;
  if (secs < 3600) return `${Math.floor(secs / 60)} min`;
  return `${Math.floor(secs / 3600)} h`;
}

function logging(feed: Feed, match: ParsedMatch | null, view: MatchView | null, now: number): StatusLine {
  if (feed.logError) return { tone: "bad", title: "Not logging", detail: feed.logError };
  if (!feed.log) return { tone: "warn", title: "No Workshop log yet", detail: "Turn on the inspector log file in Overwatch's options" };
  const quiet = since(feed.logWrittenAt, now);
  const growing = quiet !== null && quiet < feed.quietSecs;
  if (!match) {
    return growing
      ? { tone: "warn", title: "Log without ranked lines", detail: "Is the lobby on the ranked code?" }
      : { tone: "idle", title: "Lobby idle", detail: "No ranked match in the newest log" };
  }
  if (match.unranked.length > 0) {
    return { tone: "bad", title: "Unranked: won't count", detail: match.unranked.map((r) => unrankedReasons[r] ?? r).join("; ") };
  }
  if (!growing) return { tone: "idle", title: "Lobby idle", detail: quiet === null ? null : `Log quiet for ${ago(quiet)}` };
  if (match.endResult !== null) return { tone: "good", title: "Match over", detail: "Uploading it" };
  const round = view?.live ? `Round ${view.live.number}` : view && view.rounds.length > 0 ? `Between rounds` : "Waiting for round 1";
  const players = match.players.filter((p) => p.leaveTime === null).length;
  return { tone: "good", title: "Recording", detail: `${round}, ${players} ${players === 1 ? "player" : "players"}` };
}

const problems: Record<string, string> = {
  settings: "The settings file can't be read",
  noFolder: "No Workshop log folder",
  folderUnreadable: "Can't read the log folder",
  noToken: "No host token",
  tokenRejected: "The server turned your token down",
  noRegion: "No region picked",
  local: "A file of the tool's failed",
};

function uploads(feed: Feed): StatusLine {
  const { host } = feed;
  if (host.problem) return { tone: "bad", title: "Uploads paused", detail: problems[host.problem.kind] ?? host.problem.kind };
  if (host.dryRun) return { tone: "warn", title: "Dry run", detail: "Nothing is uploaded" };
  if (host.retrying) return { tone: "warn", title: "Upload failed, retrying", detail: host.retrying };
  const last = host.last;
  if (!last) return { tone: "idle", title: "No uploads yet", detail: null };
  if (last.queued?.kind === "playing") return { tone: "idle", title: "Uploads when the match ends", detail: last.file };
  if (last.queued?.kind === "due") return { tone: "idle", title: "Uploading", detail: last.file };
  if (last.queued?.kind === "failed") return { tone: "warn", title: "Upload failed", detail: last.queued.error };
  const answer = last.answer;
  if (!answer) return { tone: "idle", title: "Not uploaded yet", detail: last.file };
  if (answer.kind === "refused") return { tone: "bad", title: "Upload refused", detail: answer.message };
  const latest = answer.matches.at(-1);
  if (!latest) return { tone: "good", title: "Uploaded", detail: `Nothing new (${answer.result})` };
  switch (latest.status) {
    case "accepted":
      return { tone: "good", title: "Uploaded and counted", detail: waitingNote(host.waiting) };
    case "review":
      return { tone: "warn", title: "Uploaded: in review", detail: "An admin checks it before it counts" };
    case "rejected":
      return { tone: "bad", title: "Uploaded: won't count", detail: latest.rejection?.message ?? null };
    default:
      return { tone: "idle", title: `Uploaded: ${latest.status}`, detail: null };
  }
}

function waitingNote(waiting: number): string | null {
  return waiting > 0 ? `${waiting} more ${waiting === 1 ? "log" : "logs"} waiting` : null;
}

function rankedCode(feed: Feed, now: number): StatusLine {
  const age = since(feed.rankedCode.builtAt, now);
  if (age === null) return { tone: "warn", title: "Ranked code not copied", detail: "Copy it on Home before you host" };
  if (age > feed.rankedCode.staleSecs) return { tone: "warn", title: `Ranked code copied ${ago(age)} ago`, detail: "Its rank tags may be stale" };
  return { tone: "good", title: `Ranked code copied ${ago(age)} ago`, detail: null };
}

function roster(lobby: ParsedMatch["players"], ratings: PlayerRating[]): Roster {
  const key = (name: string) => name.toLowerCase();
  const counts = new Map<string, number>();
  for (const p of lobby) counts.set(key(p.name), (counts.get(key(p.name)) ?? 0) + 1);
  const players: RosterPlayer[] = lobby.map((p) => {
    const rating = ratings.find((r) => key(r.name) === key(p.name));
    const standing = rating?.standing ?? null;
    return {
      name: p.name,
      host: p.host,
      state: rating?.state ?? "pending",
      rating: standing?.rating ?? null,
      rank: standing?.rank ?? null,
      tier: standing?.tier ?? null,
      doubled: (counts.get(key(p.name)) ?? 0) > 1,
    };
  });
  // Best first; unrated last, in join order.
  players.sort((a, b) => (b.rating ?? -Infinity) - (a.rating ?? -Infinity));
  const rated = players.flatMap((p) => (p.rating === null ? [] : [p.rating]));
  return {
    players,
    average: rated.length > 0 ? rated.reduce((a, b) => a + b, 0) / rated.length : null,
    newPlayers: players.filter((p) => p.state === "unknown").length,
    doubled: players.some((p) => p.doubled),
  };
}

function killFeed(match: ParsedMatch, nameOf: (id: number | null) => string | null, shown: number): Kill[] {
  const kills: Kill[] = [];
  match.kills.forEach((kill, i) => {
    // The deflects since the kill before it in the same round led to this one.
    const before = match.kills.slice(0, i).filter((k) => k.round === kill.round).at(-1);
    const deflects =
      kill.round === null ? [] : match.deflects.filter((d) => d.round === kill.round && d.time <= kill.time && (!before || d.time > before.time));
    const chain: string[] = [];
    for (const d of deflects) {
      const name = nameOf(d.id) ?? "?";
      if (chain.at(-1) !== name) chain.push(name);
    }
    const killer = kill.attackerId === null ? null : (nameOf(kill.attackerId) ?? kill.attackerName);
    kills.push({
      key: `${i}:${kill.time}`,
      killer,
      victim: kill.victimId === null ? kill.victimName : (nameOf(kill.victimId) ?? kill.victimName),
      chain: chain.slice(-3),
      speed: deflects.at(-1)?.speed ?? null,
    });
  });
  return kills.slice(-shown).reverse();
}

function roundResult(round: ParsedRound | undefined, nameOf: (id: number | null) => string | null): RoundResult | null {
  if (!round) return null;
  const order: RoundResult["order"] = (round.finishingOrder ?? []).map((id, i) => ({ position: i + 1, name: nameOf(id) ?? "?", left: false }));
  for (const id of round.leftIds) order.push({ position: null, name: nameOf(id) ?? "?", left: true });
  return { number: round.number, result: round.result, order };
}

function summary(match: ParsedMatch, view: MatchView, result: MatchResult | null, nameOf: (id: number | null) => string | null): MatchSummary {
  const key = (name: string) => name.toLowerCase();
  const rows = view.players.map((p) => {
    const rated = result?.players.find((r) => key(r.name) === key(p.name));
    return { ...placing(p), ratingBefore: rated?.ratingBefore ?? null, ratingAfter: rated?.ratingAfter ?? null };
  });
  const top = match.deflects.reduce<(typeof match.deflects)[number] | null>((best, d) => (!best || d.speed > best.speed ? d : best), null);
  return {
    rows,
    rounds: match.rounds.length,
    topSpeed: top && { name: nameOf(top.id) ?? "?", speed: top.speed },
    rated: result !== null && rows.some((r) => r.ratingAfter !== null),
  };
}

function session(feed: Feed, match: ParsedMatch | null): Session | null {
  const { matches, results } = feed.session;
  if (matches === 0 && results.length === 0) return null;
  const host = match?.players.find((p) => p.host)?.name ?? null;
  const key = (name: string) => name.toLowerCase();
  const players = new Set(results.flatMap((r) => r.players.map((p) => key(p.name))));
  let hostChange: number | null = null;
  if (host) {
    for (const r of results) {
      const me = r.players.find((p) => key(p.name) === key(host));
      if (me?.ratingBefore != null && me.ratingAfter != null) hostChange = (hostChange ?? 0) + me.ratingAfter - me.ratingBefore;
    }
  }
  return { matches, rounds: results.reduce((n, r) => n + r.rounds, 0), players: players.size, host, hostChange };
}

function tourney(match: ParsedMatch, view: MatchView, feed: Feed): TourneyPanel {
  const info = match.tourney!;
  const lobby = feed.tourneys.find((t) => t.lobbyKey === info.lobbyKey);
  const round = view.live?.number ?? match.rounds.at(-1)?.number ?? 0;
  const limit = info.roundLimit ?? lobby?.roundLimit ?? null;
  return {
    name: lobby?.tourney ?? "Tourney match",
    label: lobby?.label ?? `Lobby ${info.lobbyKey}`,
    round,
    limit,
    left: limit === null ? null : Math.max(0, limit - round),
    standings: view.players.map(placing),
    ended: match.endResult === "ROUNDS",
  };
}
