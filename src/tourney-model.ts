/**
 * Tourneys (#8, #9, #10), what the window works out itself: when a lobby's code window opens, the
 * countdowns, the region warning and the verify screenshot's checks. Pure: no Tauri, no DOM.
 */

/** Mirrors `LobbyTourney` in src-tauri/src/server.rs. */
export interface LobbyTourney {
  id: number;
  name: string;
  region: string;
  /** ISO 8601. */
  startsAt: string;
  status: "scheduled" | "live" | "done" | "cancelled";
}

/** Mirrors `TourneyCodeValues` in src-tauri/src/server.rs. */
export interface TourneyCodeValues {
  lobbyKey: string;
  roundLimit: number;
  name: string;
  label: string;
}

/** Mirrors `LobbyView` in src-tauri/src/tourneys.rs (with `TourneyLobby` from server.rs flattened in). */
export interface LobbyView {
  id: number;
  label: string;
  /** The tourney's region: the lobby's match is uploaded as it. */
  region: string;
  roundLimit: number;
  tourney: LobbyTourney;
  matchId: number | null;
  /** The verify screenshot's URL on the server, `null` while there's none. */
  screenshot: string | null;
  screenshotExpired: boolean;
  verified: boolean;
  codeFrom: string | null;
  /** Only while the code window is open. */
  code: TourneyCodeValues | null;
  /** The lobby's key, from its code values now or earlier (`null`: the tool hasn't seen it). */
  lobbyKey: string | null;
  /** This tool uploaded the end of its match. */
  matchUploaded: boolean;
  needsScreenshot: boolean;
}

/** Mirrors `TourneyProblem` in src-tauri/src/tourneys.rs. */
export type TourneyProblem = { kind: "settings" } | { kind: "noToken" } | { kind: "tokenRejected"; revoked: boolean } | { kind: "failed"; message: string };

/** Mirrors `TourneysStatus` in src-tauri/src/tourneys.rs: what `get_tourneys` returns. */
export interface TourneysStatus {
  serverUrl: string;
  lobbies: LobbyView[];
  /** When the server last answered, `null` while it hasn't. */
  checkedAt: string | null;
  problem: TourneyProblem | null;
}

/** Mirrors `TourneyCode` in src-tauri/src/lib.rs: what `build_tourney_code` returns. */
export interface TourneyCode {
  code: string;
  serverUrl: string;
  lobbyId: number;
  /** Mirrors `Written` in src-tauri/src/tourney.rs: the values as they went into the rule. */
  values: TourneyCodeValues;
  region: string;
  release: string;
  top: number;
  names: number;
  skippedNames: number;
  /** The data center the code puts the lobby on, `null`: the game picks. */
  dataCenter: string | null;
}

/** Mirrors `ScreenshotFile` in src-tauri/src/tourney.rs: what `newest_screenshot` returns. */
export interface ScreenshotFile {
  path: string;
  name: string;
  /** When it was written (taken), ISO 8601. */
  takenAt: string;
  bytes: number;
}

/** Whether a lobby is over for its host, apart from its screenshot (`tourney::is_done` in Rust). */
export function isDone(lobby: LobbyView): boolean {
  return lobby.matchId !== null || lobby.tourney.status === "done" || lobby.tourney.status === "cancelled";
}

export type CodeState = { kind: "open" } | { kind: "opens"; at: Date } | { kind: "closed" };

/**
 * Whether "Copy tourney code" can work at `now`. The server decides (`code`); once `codeFrom` has
 * passed, a list read before it is out of date, so it counts as open: the build asks the server again.
 */
export function codeState(lobby: LobbyView, now: Date): CodeState {
  if (isDone(lobby)) return { kind: "closed" };
  if (lobby.code) return { kind: "open" };
  const from = lobby.codeFrom ? new Date(lobby.codeFrom) : null;
  if (from && !Number.isNaN(from.getTime()) && from.getTime() > now.getTime()) return { kind: "opens", at: from };
  return from ? { kind: "open" } : { kind: "closed" };
}

/** How long until a moment, in words: "in 2 d 3 h", "in 1 h 05 min", "in 4 min 10 s"; "now" once it's here. */
export function countdown(ms: number): string {
  if (ms <= 0) return "now";
  const s = Math.floor(ms / 1000);
  const days = Math.floor(s / 86400);
  const hours = Math.floor((s % 86400) / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  const seconds = s % 60;
  if (days) return `in ${days} d ${hours} h`;
  if (hours) return `in ${hours} h ${String(minutes).padStart(2, "0")} min`;
  if (minutes) return `in ${minutes} min ${String(seconds).padStart(2, "0")} s`;
  return `in ${seconds} s`;
}

/**
 * Whether to warn that a lobby's region isn't the one the tool uploads as (`uploadRegion`: the
 * region picked, else the home region; `null` while not known). The tool uploads the lobby's match
 * as the lobby's region anyway once it knows the lobby, but the host's other matches go as theirs.
 */
export function regionWarning(lobbyRegion: string, uploadRegion: string | null): boolean {
  return uploadRegion !== null && uploadRegion !== lobbyRegion;
}

/** The MIME type of a PNG, JPEG or WebP image from its first bytes (`tourney::image_type` in Rust). */
export function imageType(bytes: Uint8Array): string | null {
  const starts = (head: number[]) => head.every((b, i) => bytes[i] === b);
  if (starts([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return "image/png";
  if (starts([0xff, 0xd8, 0xff])) return "image/jpeg";
  const text = (from: number, to: number) => String.fromCharCode(...bytes.subarray(from, to));
  if (bytes.length >= 12 && text(0, 4) === "RIFF" && text(8, 12) === "WEBP") return "image/webp";
  return null;
}

export type ScreenshotCheck = { ok: true; type: string } | { ok: false; error: string };

/** Whether the server will take these bytes as a verify screenshot (at most `maxBytes`), as Rust checks again. */
export function checkScreenshot(bytes: Uint8Array, maxBytes: number): ScreenshotCheck {
  if (!bytes.length) return { ok: false, error: "The image is empty" };
  if (bytes.length > maxBytes) return { ok: false, error: `The image is over the server's ${Math.floor(maxBytes / (1024 * 1024))} MB limit` };
  const type = imageType(bytes);
  return type ? { ok: true, type } : { ok: false, error: "That isn't a PNG, JPEG or WebP image" };
}

/** Whether a screenshot was taken before the tourney started: probably not of this lobby's final standings. */
export function takenBeforeStart(takenAt: string, startsAt: string): boolean {
  return new Date(takenAt).getTime() < new Date(startsAt).getTime();
}

/** Where the lobby's verify screenshot stands, for its line. `null`: nothing to say yet. */
export function screenshotState(lobby: LobbyView): { text: string; tone: "good" | "bad" | "muted" } | null {
  if (lobby.verified) return { text: "Screenshot verified by an admin.", tone: "good" };
  if (lobby.screenshot) return { text: "Screenshot uploaded. An admin checks it against the match; you can replace or delete it until then.", tone: "muted" };
  if (lobby.needsScreenshot) {
    const expired = lobby.screenshotExpired ? " The one uploaded before has expired." : "";
    return { text: `Upload your screenshot of the final standings.${expired}`, tone: "bad" };
  }
  return null;
}
