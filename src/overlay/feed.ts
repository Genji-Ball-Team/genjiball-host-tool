/**
 * The overlay's feed (#49), and where it comes from: the overlay window asks Rust
 * (`get_overlay_feed`); the stream page (#55), open in OBS without Tauri, asks the tool's server on
 * this PC (`/feed`, `src-tauri/src/stream.rs`). Both get the same thing.
 */

/** Mirrors `Feed` in src-tauri/src/overlay.rs. */
export interface Feed {
  stream: boolean;
  widgets: string[];
  opacity: number;
  scale: number;
  layout: Record<string, [number, number]>;
  /** How big the host made each widget, a share of its normal size. */
  sizes: Record<string, number>;
  /** The smallest and biggest a widget can be made, and a step. */
  sizeRange: [number, number, number];
  editing: boolean;
  hotkeys: { action: string; label: string; keys: string | null }[];
  pollMs: number;
  /** `text` is `null` when the log didn't change since `known`. */
  log: { file: string; size: number; text: string | null } | null;
  logWrittenAt: string | null;
  logError: string | null;
  quietSecs: number;
  host: HostView;
  ratings: PlayerRating[];
  region: string | null;
  tourneys: TourneyView[];
  result: MatchResult | null;
  session: { since: string; matches: number; results: MatchResult[] };
  logCopies: number | null;
  rankedCode: { builtAt: string | null; staleSecs: number };
  killFeedShown: number;
  now: string;
}

/** Mirrors `HostView` in src-tauri/src/overlay.rs. */
export interface HostView {
  problem: { kind: string; message?: string; revoked?: boolean } | null;
  waiting: number;
  retrying: string | null;
  dryRun: boolean;
  afk: boolean;
  /** Mirrors `Entry` in src-tauri/src/history.rs. */
  last: {
    file: string;
    at: string | null;
    answer:
      | { kind: "answered"; result: string; matches: { matchKey: string | null; status: string; rejection: { code: string; message: string } | null }[] }
      | { kind: "refused"; error: string; message: string }
      | null;
    queued: { kind: "playing" } | { kind: "due" } | { kind: "failed"; error: string } | null;
  } | null;
}

/** Mirrors `Standing` in src-tauri/src/server.rs. */
export interface Standing {
  id: number;
  name: string;
  rank: number | null;
  rating: number | null;
  tier: { label: string; color: [number, number, number] } | null;
}

/** Mirrors `PlayerRating` in src-tauri/src/ratings.rs. */
export interface PlayerRating {
  name: string;
  state: "pending" | "found" | "unknown";
  standing: Standing | null;
}

/** Mirrors `TourneyView` in src-tauri/src/overlay.rs. */
export interface TourneyView {
  lobbyKey: string;
  tourney: string;
  label: string;
  roundLimit: number;
  needsScreenshot: boolean;
}

/** Mirrors `MatchResult` in src-tauri/src/server.rs. */
export interface MatchResult {
  id: number;
  playedAt: string | null;
  rounds: number;
  ratedRounds: number;
  players: { name: string; roundWins: number | null; kills: number | null; place: number | null; ratingBefore: number | null; ratingAfter: number | null }[];
}

/** Mirrors `FeedRequest` in src-tauri/src/overlay.rs. */
export interface FeedRequest {
  known: { file: string; size: number } | null;
  names: string[];
  matchKey: string | null;
}

export interface Source {
  read(request: FeedRequest): Promise<Feed>;
  /** The overlay window only: the stream page can't change anything. */
  saveLayout?(layout: Record<string, [number, number]>, sizes: Record<string, number>): Promise<void>;
  /** Switches one widget off: "Hide this widget". */
  hideWidget?(key: string): Promise<void>;
  setEditing?(on: boolean): Promise<void>;
  /** Calls `listener` when the settings or edit mode change, to read the feed again at once. */
  onChange?(listener: () => void): Promise<void>;
}

/** Whether this page runs in the app's overlay window, not in a browser (OBS). */
export function inTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export async function tauriSource(): Promise<Source> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  return {
    read: (request) => invoke<Feed>("get_overlay_feed", { request }),
    saveLayout: (layout, sizes) => invoke<void>("set_overlay_layout", { layout, sizes }),
    hideWidget: (key) => invoke<void>("set_overlay_widget", { key, on: false }),
    setEditing: async (on) => {
      await invoke("set_overlay_editing", { on });
    },
    onChange: async (listener) => {
      await listen("overlay-changed", listener);
    },
  };
}

/** The stream page's feed, from the server that served it (or `?feed=`, under `tauri dev`). */
export function httpSource(base: string): Source {
  return {
    read: async (request) => {
      const response = await fetch(new URL(`feed?${feedQuery(request)}`, base), { cache: "no-store" });
      if (!response.ok) throw new Error(`The host tool answered ${response.status}`);
      return (await response.json()) as Feed;
    },
  };
}

/** The request as `stream.rs` reads it. */
export function feedQuery(request: FeedRequest): string {
  const query = new URLSearchParams();
  if (request.known) {
    query.set("file", request.known.file);
    query.set("size", String(request.known.size));
  }
  for (const name of request.names) query.append("name", name);
  if (request.matchKey) query.set("matchKey", request.matchKey);
  return query.toString();
}
