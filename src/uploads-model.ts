/**
 * Uploads, what the window says about them: the upload status and each upload's answer, in
 * words. Pure: no Tauri, no DOM.
 */
import type { Tone } from "./dom";

/** Mirrors `Host` in src-tauri/src/server.rs. */
export interface Host {
  id: number;
  name: string;
  trust: "trusted" | "untrusted";
  /** The home region, `null` for none. */
  region: string | null;
}

/** Mirrors `UploadStatus` in src-tauri/src/uploader.rs. */
export interface UploadStatus {
  serverUrl: string;
  logFolder: string | null;
  chosenRegion: string | null;
  /** What uploads go as: the region picked, else the home region. `null` while not known. */
  region: string | null;
  problem: Problem | null;
  waiting: number;
  retrying: string | null;
  history: HistoryPage;
  historyRevision: number;
  host: Host | null;
  afk: AfkStatus;
  dryRun: boolean;
}

/** Mirrors `AfkStatus` in src-tauri/src/afk.rs: what `set_afk` returns. */
export interface AfkStatus {
  on: boolean;
  /** The match a round was last skipped in for AFK, and those rounds. */
  latest: { matchKey: string; rounds: number[] } | null;
  /** Why `afk.json` couldn't be read or written. */
  error: string | null;
}

/** Mirrors `Problem` in src-tauri/src/uploader.rs. */
export type Problem =
  | { kind: "settings" }
  | { kind: "noFolder" }
  | { kind: "folderUnreadable"; message: string }
  | { kind: "noToken" }
  | { kind: "tokenRejected"; revoked: boolean }
  | { kind: "noRegion" }
  | { kind: "local"; message: string };

/** Mirrors `Page` in src-tauri/src/history.rs: what `get_upload_history` returns. */
export interface HistoryPage {
  entries: HistoryEntry[];
  page: number;
  pageSize: number;
  total: number;
}

/** Mirrors `Entry` in src-tauri/src/history.rs. */
export interface HistoryEntry {
  file: string;
  at: string | null;
  players: string[];
  answer: Answer | null;
  queued: QueueState | null;
  /** Its tourney matches (none: only ranked ones). Mirrors `SentTourney` in src-tauri/src/uploads.rs. */
  tourneys: { lobbyKey: string; ended: boolean }[];
}

/** Mirrors `Answer` in src-tauri/src/uploads.rs. */
export type Answer = { kind: "answered"; result: "stored" | "unchanged" | "duplicate"; region: string | null; matches: UploadedMatch[] } | { kind: "refused"; error: string; message: string };

/** Mirrors `QueueState` in src-tauri/src/watcher.rs. */
export type QueueState = { kind: "playing" } | { kind: "due" } | { kind: "failed"; error: string };

/** Mirrors `UploadedMatch` in src-tauri/src/server.rs. */
export interface UploadedMatch {
  matchKey: string | null;
  matchId: number | null;
  lineCount: number;
  region: string | null;
  action: "insert" | "replace" | "repoint" | "skip";
  status: "accepted" | "review" | "rejected" | "void";
  rejection: { code: string; message: string } | null;
  reviewReasons: string[];
}

export function describeProblem(problem: Problem): string {
  switch (problem.kind) {
    case "settings":
      return "Paused until the settings file is fixed (see above).";
    case "noFolder":
      return "Waiting for the Workshop log folder.";
    case "folderUnreadable":
      return `Can't read the log folder: ${problem.message}`;
    case "noToken":
      return "Paused: there's no host token for this server.";
    case "tokenRejected":
      return problem.revoked ? "Paused: this token was revoked. Ask an admin for a new one." : "Paused: the server doesn't know this token.";
    case "noRegion":
      return "Paused: you have no home region yet. Pick the region you host in under Settings → Game.";
    case "local":
      return problem.message;
  }
}

/** Where uploads stand, for the Uploads view's status line. */
export function describeUploads(status: Pick<UploadStatus, "problem" | "dryRun" | "waiting">): string {
  if (status.problem) return describeProblem(status.problem);
  if (status.dryRun) return "Dry run: ranked logs are picked as usual but not uploaded. The debug panel (Settings → Advanced) lists them.";
  if (status.waiting === 1) return "Watching. 1 ranked log to upload.";
  if (status.waiting) return `Watching. ${status.waiting} ranked logs to upload.`;
  return "Watching for ranked matches.";
}

/** Why a match waits for an admin. An untrusted host's every match does: the host line says so. */
const reviewReasons: Record<string, string | null> = {
  duplicate_name: "two players with the same name",
  untrusted_host: null,
  // A tourney match the server couldn't link to its lobby (genjiball-ranked docs/api.md, "Tourney matches").
  tourney_unknown_lobby: "the tourney lobby isn't known",
  tourney_cancelled: "the tourney was cancelled",
  tourney_wrong_host: "you aren't the lobby's host",
  tourney_wrong_region: "uploaded as another region than the tourney's",
  tourney_lobby_taken: "the lobby already has a match",
  tourney_round_limit: "the round limit isn't the lobby's",
};

export function describeMatch(match: UploadedMatch): string {
  switch (match.status) {
    case "accepted":
      return "accepted";
    case "review": {
      const reasons = match.reviewReasons
        .map((r) => (Object.hasOwn(reviewReasons, r) ? reviewReasons[r] : r))
        .filter((r): r is string => typeof r === "string");
      return reasons.length ? `waiting for an admin: ${reasons.join(", ")}` : "waiting for an admin";
    }
    case "rejected":
      return `rejected: ${match.rejection?.message || match.rejection?.code || "no reason given"}`;
    case "void":
      return "voided by an admin";
  }
}

export function describeAnswer(answer: Answer): { text: string; tone: Tone } {
  if (answer.kind === "refused") return { text: answer.message, tone: "bad" };
  if (answer.result === "duplicate") return { text: "Already uploaded", tone: "muted" };
  if (!answer.matches.length) return { text: "No match in it", tone: "muted" };
  const text = answer.matches.map(describeMatch).join("; ");
  const tone = answer.matches.every((m) => m.status === "accepted") ? "good" : answer.matches.some((m) => m.status === "rejected") ? "bad" : "muted";
  return { text: text.charAt(0).toUpperCase() + text.slice(1), tone };
}

/** The regions an upload's matches are stored in, as their ids in capitals (`EU`). */
export function answerRegions(answer: Answer): string {
  if (answer.kind !== "answered") return "";
  const regions = new Set(answer.matches.map((m) => m.region).filter((r): r is string => r !== null));
  if (!regions.size && answer.region) regions.add(answer.region);
  return [...regions].map((r) => r.toUpperCase()).join(", ");
}

export function describeQueued(queued: QueueState): { text: string; tone: Tone } {
  switch (queued.kind) {
    case "playing":
      return { text: "Being played: uploaded when the match ends or the log stops growing", tone: "muted" };
    case "due":
      return { text: "Waiting to upload", tone: "muted" };
    case "failed":
      return { text: `Upload failed: ${queued.error}. Retrying automatically.`, tone: "bad" };
  }
}

/** Only accepted and voided matches are on the site (`server::is_public`). */
export function onSite(match: UploadedMatch): match is UploadedMatch & { matchId: number } {
  return match.matchId !== null && (match.status === "accepted" || match.status === "void");
}
