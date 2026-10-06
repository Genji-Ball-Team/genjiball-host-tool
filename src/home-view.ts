/**
 * Home (#34): the status a host checks while hosting, without scrolling: the connection, the log
 * being read, problems with a button to the place that fixes them, the ranked code, AFK, the live
 * lobby and the last upload. A first start (no token yet) shows a short setup instead.
 */
import { invoke } from "@tauri-apps/api/core";
import { ago, headline, matchInProgress, problems, type Fix, type HomeInput } from "./home-model";
import { setTitlebarState } from "./titlebar";
import { markView, onViewChange } from "./views";

/** Mirrors `LiveFile` in src-tauri/src/match_log.rs: what `get_live_file` returns. */
interface LiveFile {
  file: string;
  writtenAt: string;
}

type Tone = "good" | "bad" | "muted";

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

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found as T;
}

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = "", className = ""): HTMLElementTagNameMap[K] {
  const found = document.createElement(tag);
  if (text) found.textContent = text;
  if (className) found.className = className;
  return found;
}

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

/** Reads the live log's name every `pollSecs`: Home shows it, and the title bar pulses while it grows. */
let timer: ReturnType<typeof setTimeout> | undefined;

async function pollLiveFile(): Promise<void> {
  clearTimeout(timer);
  if (!get) return;
  const state = get();
  if (state.hasToken && state.logFolder?.exists && document.visibilityState === "visible") {
    try {
      liveFile = await invoke<LiveFile | null>("get_live_file");
      liveFileError = null;
    } catch (err) {
      liveFileError = err instanceof Error ? err.message : String(err);
    }
    renderHome();
  }
  timer = setTimeout(() => void pollLiveFile(), state.pollSecs * 1000);
}

/** The log folder changed: the live log is read afresh. */
export function homeFolderChanged(): void {
  liveFile = null;
  liveFileError = null;
  void pollLiveFile();
}

/** Binds Home. Call once, once `state()` can answer. */
export function setupHome(state: () => HomeState, onFix: (fix: Fix) => void): void {
  get = state;
  fix = onFix;
  // Fresh as soon as Home shows.
  onViewChange((view) => {
    if (view === "home") void pollLiveFile();
  });
  renderHome();
  void pollLiveFile();
}
