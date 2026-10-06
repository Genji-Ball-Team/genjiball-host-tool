/**
 * Home (#34): what a host checks while hosting, worked out from the state the window already has.
 * Pure: no Tauri, no DOM. `home-view.ts` draws it.
 */
import type { Pane, View } from "./views";

/** What a problem's button does. */
export type Fix =
  | { kind: "pane"; pane: Pane; label: string }
  | { kind: "view"; view: View; label: string }
  | { kind: "changeToken"; label: string }
  | { kind: "checkToken"; label: string }
  | { kind: "dryRunOff"; label: string };

export interface Problem {
  text: string;
  fix: Fix | null;
}

export type Tone = "good" | "warn" | "bad";

/** The parts of the upload status Home reads. Mirrors part of `UploadStatus` in src-tauri/src/uploader.rs. */
export interface UploadSummary {
  problem: { kind: string; message?: string; revoked?: boolean } | null;
  waiting: number;
  retrying: string | null;
}

export interface HomeInput {
  hasToken: boolean;
  /** The log folder in use, `null` when there's no Documents folder. */
  logFolder: { exists: boolean } | null;
  dryRun: boolean;
  settingsError: boolean;
  /** `null` until the first upload status for these settings. */
  upload: UploadSummary | null;
  /** The last token check's result, `null` before one. */
  tokenCheck: "ok" | "unknown" | "revoked" | "unreachable" | null;
  /** Why the last token check couldn't reach the server. */
  unreachable: string | null;
  /** Tourney lobbies waiting for the host's verify screenshot. */
  screenshotsDue: number;
}

/** The status line at the top of Home, and its colour. */
export function headline(input: HomeInput, serverName: string): { text: string; detail: string; tone: Tone } {
  const { upload } = input;
  if (input.settingsError) return { text: "Uploads are paused", detail: "The settings file can't be read: see above.", tone: "bad" };
  if (upload?.problem) return { text: "Uploads are paused", detail: "Fix what's listed below and they go on by themselves.", tone: "bad" };
  if (input.dryRun) return { text: "Dry run", detail: "Ranked logs are picked as usual, but nothing is uploaded.", tone: "warn" };
  if (!upload) return { text: `Uploading to ${serverName}`, detail: "Starting…", tone: "good" };
  const waiting = upload.waiting === 1 ? "1 ranked log to upload." : upload.waiting ? `${upload.waiting} ranked logs to upload.` : "Watching for ranked matches.";
  return { text: `Uploading to ${serverName}`, detail: waiting, tone: upload.retrying ? "warn" : "good" };
}

/** What stops or holds up uploads, each with the button to the place that fixes it. */
export function problems(input: HomeInput): Problem[] {
  const found: Problem[] = [];
  const kind = input.upload?.problem?.kind;

  if (kind === "tokenRejected" || input.tokenCheck === "unknown" || input.tokenCheck === "revoked") {
    const revoked = input.upload?.problem?.revoked ?? input.tokenCheck === "revoked";
    found.push({
      text: revoked ? "Your host token was revoked. Ask an admin for a new one." : "The server doesn't know your host token.",
      fix: { kind: "changeToken", label: "Change token" },
    });
  } else if (input.tokenCheck === "unreachable") {
    found.push({ text: `Couldn't check your token: ${input.unreachable ?? "the server didn't answer"}`, fix: { kind: "checkToken", label: "Check again" } });
  }

  if (!input.logFolder || !input.logFolder.exists || kind === "noFolder") {
    found.push({
      text: input.logFolder ? "The Workshop log folder isn't there yet. Turn on log files in the game's settings, or choose the folder." : "No Workshop log folder.",
      fix: { kind: "pane", pane: "game", label: "Choose folder" },
    });
  } else if (kind === "folderUnreadable") {
    found.push({ text: `Can't read the log folder: ${input.upload?.problem?.message ?? ""}`, fix: { kind: "pane", pane: "game", label: "Log folder" } });
  }

  if (kind === "noRegion") found.push({ text: "You have no home region yet. Pick the region you host in.", fix: { kind: "pane", pane: "game", label: "Pick region" } });
  if (kind === "local") found.push({ text: input.upload?.problem?.message ?? "Uploads are paused.", fix: { kind: "view", view: "uploads", label: "Uploads" } });
  if (input.dryRun) found.push({ text: "Dry run is on: nothing is uploaded.", fix: { kind: "dryRunOff", label: "Turn it off" } });
  if (input.upload?.retrying) found.push({ text: `An upload failed and is retried automatically: ${input.upload.retrying}`, fix: { kind: "view", view: "uploads", label: "Uploads" } });
  if (input.screenshotsDue) {
    found.push({
      text: input.screenshotsDue === 1 ? "A tourney lobby needs your verify screenshot." : `${input.screenshotsDue} tourney lobbies need your verify screenshot.`,
      fix: { kind: "view", view: "tourneys", label: "Tourneys" },
    });
  }
  return found;
}

/** How long ago `iso` was, in words a host reads at a glance (`12 s ago`, `3 min ago`). */
export function ago(iso: string, now: number): string {
  const secs = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
  if (secs < 60) return `${secs} s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} h ago`;
  return new Date(iso).toLocaleDateString();
}
