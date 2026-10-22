/**
 * Home (#34): the status a host checks while hosting, without scrolling: the connection, the log
 * being read, problems with a button to the place that fixes them, the ranked code, AFK, the live
 * lobby and the last upload. A first start (no token yet) shows a short setup instead.
 */
import { invoke } from "@tauri-apps/api/core";
import { el, errorText, node, type Tone } from "./dom";
import { ago, headline, matchInProgress, problems, type Fix, type HomeInput } from "./home-model";
import { setTitlebarState } from "./titlebar";
import { markView, onViewChange } from "./views";

/** Mirrors `LiveFile` in src-tauri/src/match_log.rs: what `get_live_file` returns. */
interface LiveFile {
  file: string;
  writtenAt: string;
}

/** What Home shows, from the rest of the window. */
export interface HomeState extends HomeInput {
  serverName: string;
  host: { name: string; untrusted: boolean } | null;
  /** The token's state, as the Account pane says it. */
  token: { text: string; tone: Tone };
  /** The region uploads go as, in words. */
  region: string;
  lastUpload: { file: string; when: string; text: string; tone: Tone } | null;
  pollSecs: number;
  /** What the live lobby saw in the live log, `null` while it's off. */
  lobbyLive: "idle" | "unranked" | "playing" | null;
  /** How long a log must stop growing before its match counts as over, in seconds. */
  quietSecs: number;
}

/** Set by `setupHome`: Home draws nothing before. */
let get: (() => HomeState) | null = null;
let fix: (fix: Fix) => void;
let liveFile: LiveFile | null = null;
let liveFileError: string | null = null;
/** The last upload shown, so a new one is marked once (`undefined` before the first). */
let shownUpload: string | undefined;

function liveLog(state: HomeState): string {
  if (!state.logFolder) return "No Documents folder found";
  if (!state.logFolder.exists) return "Folder not found";
  if (liveFileError) return liveFileError;
  if (!liveFile) return "Folder found, no log yet";
  return `${liveFile.file}, written ${ago(liveFile.writtenAt, Date.now())}`;
}

export function renderHome(): void {
  if (!get) return;
  const state = get();
  el("setup").hidden = state.hasToken;
  el("home").hidden = !state.hasToken;
  const found = problems(state);
  markView("home", state.hasToken && found.length > 0);
  if (!state.hasToken) {
    setTitlebarState("Not set up", "warn");
    const folder = state.logFolder;
    el("setup-folder-state").textContent = !folder ? "No Documents folder found: choose it." : folder.exists ? "Found." : "Not there yet: it appears once the game writes a log, or choose it.";
    el("setup-folder").classList.toggle("done", Boolean(folder?.exists));
    return;
  }

  const head = headline(state, state.serverName);
  // A match being played: the status dots pulse.
  const playing = matchInProgress(state.lobbyLive, liveFile?.writtenAt ?? null, state.quietSecs, Date.now());
  setTitlebarState(head.tone === "bad" ? "Paused" : state.dryRun ? "Dry run" : head.tone === "warn" ? "Retrying" : state.serverName, head.tone, playing);
  el("home-status").dataset.tone = head.tone;
  el("home-status").classList.toggle("playing", playing);
  el("home-headline").textContent = head.text;
  el("home-detail").textContent = playing && head.tone === "good" ? "A ranked match is being played." : head.detail;
  el("home-host").textContent = state.host?.name ?? "Not known yet";
  el("home-host-trust").hidden = !state.host?.untrusted;
  const token = el("home-token");
  token.textContent = state.token.text;
  token.className = `status ${state.token.tone}`;
  el("home-region").textContent = state.region;
  el("home-log").textContent = liveLog(state);

  el("home-problems").replaceChildren(
    ...found.map((p) => {
      const item = node("li");
      item.append(node("span", p.text));
      if (p.fix) {
        const target = p.fix;
        const button = node("button", target.label, "quiet");
        button.type = "button";
        button.addEventListener("click", () => fix(target));
        item.append(button);
      }
      return item;
    }),
  );

  const last = el("home-last");
  if (!state.lastUpload) {
    last.replaceChildren(node("span", "Nothing uploaded to this server yet.", "muted"));
  } else {
    const { file, when, text, tone } = state.lastUpload;
    last.replaceChildren(node("span", file, "path"), node("span", when, "muted"), node("span", text, tone));
  }
  const upload = state.lastUpload ? `${state.lastUpload.file}|${state.lastUpload.when}` : "";
  if (shownUpload !== undefined && upload && upload !== shownUpload) flash(last);
  shownUpload = upload;
}

/** Lights `target` up once: something new just came in (`elapsed` ms ago, for one drawn again). */
export function flash(target: HTMLElement, elapsed = 0): void {
  target.classList.remove("fresh");
  void target.offsetWidth; // Restarts the animation.
  target.style.animationDelay = `-${elapsed}ms`;
  target.classList.add("fresh");
}

let timer: ReturnType<typeof setTimeout> | undefined;
/** Counts restarts: a read from before the last one is dropped and reads no more, so one loop runs. */
let run = 0;

/**
 * Reads the live log's name every `pollSecs` while the window shows, whatever the view: Home shows
 * it, and the title bar (on every view) pulses while it grows.
 */
async function pollLiveFile(current: number): Promise<void> {
  if (!get) return;
  const state = get();
  if (state.hasToken && state.logFolder?.exists && document.visibilityState === "visible") {
    let found: LiveFile | null = null;
    let error: string | null = null;
    try {
      found = await invoke<LiveFile | null>("get_live_file");
    } catch (err) {
      error = errorText(err);
    }
    if (current !== run) return;
    liveFile = found;
    liveFileError = error;
    renderHome();
  }
  timer = setTimeout(() => void pollLiveFile(current), state.pollSecs * 1000);
}

/** Reads the live log's name at once, then every `pollSecs`, in place of the loop running so far. */
function readLiveFile(): void {
  clearTimeout(timer);
  run += 1;
  void pollLiveFile(run);
}

/** The log folder changed: the live log is read afresh. */
export function homeFolderChanged(): void {
  liveFile = null;
  liveFileError = null;
  readLiveFile();
}

/** Binds Home. Call once, once `state()` can answer. */
export function setupHome(state: () => HomeState, onFix: (fix: Fix) => void): void {
  get = state;
  fix = onFix;
  // Fresh as soon as Home shows.
  onViewChange((view) => {
    if (view === "home") readLiveFile();
  });
  renderHome();
  readLiveFile();
}
