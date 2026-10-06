/**
 * The "Tourneys" section (#8, #9, #10): the tourney lobbies the host is assigned to, with their
 * start and countdown, "Copy tourney code" during the code window, and the verify screenshot of
 * the final standings: offered from the screenshots folder, or picked, dropped or pasted, previewed,
 * then uploaded, replaced or deleted until an admin verified it. Rust asks the server and does the
 * uploads; this only shows them.
 */
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import {
  checkScreenshot,
  codeState,
  countdown,
  isDone,
  regionWarning,
  screenshotState,
  takenBeforeStart,
  type CodeState,
  type LobbyView,
  type ScreenshotFile,
  type TourneyCode,
  type TourneyProblem,
  type TourneysStatus,
} from "./tourney-model";
import { markView } from "./views";

/** What the section needs from the rest of the window. */
export interface TourneyContext {
  serverUrl: string;
  /** The region uploads go as (picked, else home), `null` while not known. */
  uploadRegion: string | null;
  /** The screenshots folder in use, `null` when there's none. */
  screenshotFolder: string | null;
  screenshotMaxBytes: number;
  screenshotPollSecs: number;
  regionLabel(id: string): string;
  /** Runs a button's action with the buttons disabled, and shows its error. */
  busy(action: () => Promise<void>, showError: (message: string) => void): Promise<void>;
  /** Whether an action runs now: new buttons start disabled then. */
  isBusy(): boolean;
}

/** The image the host is about to upload for a lobby. */
interface Candidate {
  bytes: Uint8Array;
  type: string;
  /** A `blob:` URL for the preview. */
  url: string;
  /** Where it came from, in words. */
  caption: string;
  /** From the screenshots folder: which file, so a newer one replaces it. Else the host chose it. */
  newest: ScreenshotFile | null;
}

type Line = { text: string; tone: "good" | "bad" | "muted" };

/** How often the countdowns are written again, in milliseconds. */
const COUNTDOWN_TICK_MS = 1000;

let context: () => TourneyContext;
let status: TourneysStatus | null = null;
/** Per lobby: what "Copy tourney code" last said. */
const codeLines = new Map<number, Line>();
/** Per lobby: a code built but not copied (the window lost focus while it was built). */
const uncopied = new Map<number, TourneyCode>();
/** Per lobby: the image waiting to be uploaded, and what came of the last try. */
const candidates = new Map<number, Candidate>();
const shotLines = new Map<number, Line>();
/** The lobby whose screenshot panel is open: drops and pastes go to it. */
let openShot: number | null = null;
/** Lobbies whose panel was opened by itself once (the host may close it). */
const autoOpened = new Set<number>();
/** Each lobby's code state at the last render: a change draws the list again. */
const drawnStates = new Map<number, CodeState["kind"]>();

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

function showError(message: string): void {
  const error = el("tourneys-error");
  error.hidden = !message;
  error.textContent = message;
}

function button(text: string, quiet: boolean, action: () => Promise<void>, onError: (m: string) => void = showError): HTMLButtonElement {
  const found = node("button", text, quiet ? "quiet" : "");
  found.type = "button";
  found.disabled = context().isBusy();
  found.addEventListener("click", () => {
    showError("");
    void context().busy(action, onError);
  });
  return found;
}

/** A local date and time the host reads (`Sat 10 Oct, 19:00`). */
function when(iso: string): string {
  return new Date(iso).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

/** A countdown to `iso`, kept up to date every second. */
function until(iso: string): HTMLSpanElement {
  const found = node("span", countdown(new Date(iso).getTime() - Date.now()));
  found.dataset.until = iso;
  return found;
}

function describeProblem(problem: TourneyProblem): string {
  switch (problem.kind) {
    case "settings":
      return "Not checked until the settings file is fixed (see above).";
    case "noToken":
      return "Save your host token to see the tourneys you host.";
    case "tokenRejected":
      return problem.revoked ? "The server says your token was revoked." : "The server doesn't know your token.";
    case "failed":
      return `Couldn't check your tourneys: ${problem.message}`;
  }
}

/** The lobby's name for the upload history, by its key: `null` when the tool doesn't know it. */
export function tourneyLabel(lobbyKey: string): string | null {
  const lobby = status?.lobbies.find((l) => l.lobbyKey === lobbyKey);
  return lobby ? `${lobby.tourney.name}, ${lobby.label}` : null;
}

/** Shows the list the uploader's tourney loop read, for the current server. */
export function renderTourneys(next: TourneysStatus): void {
  if (next.serverUrl !== context().serverUrl) return;
  status = next;
  // A needed screenshot opens its panel by itself, once.
  const due = next.lobbies.find((l) => l.needsScreenshot && !autoOpened.has(l.id));
  if (due && openShot === null) {
    openShot = due.id;
    autoOpened.add(due.id);
  }
  if (openShot !== null && !next.lobbies.some((l) => l.id === openShot && !l.verified)) openShot = null;
  draw();
  void watchNewest();
}

/** The settings changed (server, region): draws the list again for them. */
export function tourneyContextChanged(): void {
  if (status && status.serverUrl !== context().serverUrl) {
    status = null;
    codeLines.clear();
    uncopied.clear();
    for (const id of [...candidates.keys()]) dropCandidate(id);
    openShot = null;
  }
  draw();
}

function draw(): void {
  const line = el("tourneys-state");
  const list = el("tourney-list");
  if (!status) {
    line.textContent = "Checking…";
    line.className = "muted";
    list.replaceChildren();
    markView("tourneys", false);
    return;
  }
  const { lobbies, problem, checkedAt } = status;
  // The sidebar marks a lobby waiting for its verify screenshot.
  markView("tourneys", lobbies.some((l) => l.needsScreenshot));
  let text: string;
  let tone: Line["tone"] = "muted";
  if (problem) {
    text = describeProblem(problem);
    tone = "bad";
    if (lobbies.length && checkedAt) text += ` The list is from ${new Date(checkedAt).toLocaleTimeString()}.`;
  } else if (!checkedAt) {
    text = "Checking…";
  } else if (!lobbies.length) {
    text = "You aren't assigned to a tourney lobby. When an admin assigns you one, it shows here with its start time and code.";
  } else {
    const due = lobbies.filter((l) => l.needsScreenshot).length;
    text = due ? `${due === 1 ? "A lobby needs" : `${due} lobbies need`} your verify screenshot.` : `You host ${lobbies.length === 1 ? "1 tourney lobby" : `${lobbies.length} tourney lobbies`}.`;
    tone = due ? "bad" : "muted";
  }
  line.textContent = text;
  line.className = tone;
  const now = new Date();
  drawnStates.clear();
  list.replaceChildren(...lobbies.map((lobby) => lobbyItem(lobby, now)));
}

function lobbyItem(lobby: LobbyView, now: Date): HTMLLIElement {
  const ctx = context();
  const item = node("li");
  const head = node("div", "", "headline");
  head.append(node("b", lobby.tourney.name), node("span", lobby.label), node("span", lobby.region.toUpperCase(), "tag"));
  if (lobby.tourney.status === "live") head.append(node("span", "Live", "tag live"));
  if (lobby.tourney.status === "cancelled") head.append(node("span", "Cancelled", "tag"));
  item.append(head);

  const facts = node("p", "", "soft");
  const starts = new Date(lobby.tourney.startsAt);
  facts.append(`Starts ${when(lobby.tourney.startsAt)}`);
  if (starts.getTime() > now.getTime()) facts.append(" (", until(lobby.tourney.startsAt), ")");
  facts.append(` · ${lobby.roundLimit} rounds`);
  item.append(facts);

  if (regionWarning(lobby.region, ctx.uploadRegion) && !isDone(lobby)) {
    const lobbyRegion = ctx.regionLabel(lobby.region);
    item.append(
      node(
        "p",
        `This tourney is in ${lobbyRegion}, and you upload as ${ctx.regionLabel(ctx.uploadRegion ?? "")}. The tool uploads this lobby's match as ${lobbyRegion} (it learns the lobby once its code is available), and the code has ${lobbyRegion}'s rank tags. Your other matches still go as ${ctx.regionLabel(ctx.uploadRegion ?? "")}.`,
        "warn",
      ),
    );
  }

  const state = codeState(lobby, now);
  drawnStates.set(lobby.id, state.kind);
  if (state.kind !== "closed") {
    const row = node("div", "", "row");
    const copy = button("Copy tourney code", false, () => copyCode(lobby), (m) => setCodeLine(lobby.id, { text: m, tone: "bad" }));
    off(copy, state.kind !== "open");
    row.append(copy);
    if (state.kind === "opens") {
      const note = node("span", "", "muted");
      note.append(`Available from ${when(state.at.toISOString())} (`, until(state.at.toISOString()), ")");
      row.append(note);
    }
    item.append(row);
  }
  const code = codeLines.get(lobby.id);
  if (code) item.append(node("p", code.text, code.tone));

  item.append(...screenshotBlock(lobby));
  return item;
}

/** A button that stays disabled (`data-off`) until it's drawn again, whatever other actions do. */
function off(target: HTMLButtonElement, isOff: boolean): void {
  target.dataset.off = String(isOff);
  if (isOff) target.disabled = true;
}

function setCodeLine(id: number, line: Line): void {
  codeLines.set(id, line);
  draw();
}

async function copyCode(lobby: LobbyView): Promise<void> {
  const ctx = context();
  let built = uncopied.get(lobby.id);
  uncopied.delete(lobby.id);
  if (!built || built.serverUrl !== ctx.serverUrl) {
    setCodeLine(lobby.id, { text: "Building the code…", tone: "muted" });
    built = await invoke<TourneyCode>("build_tourney_code", { lobbyId: lobby.id });
    if (built.serverUrl !== context().serverUrl) {
      setCodeLine(lobby.id, { text: "The server changed while the code was built. Click again for this server's.", tone: "bad" });
      return;
    }
  }
  try {
    await navigator.clipboard.writeText(built.code);
  } catch (err) {
    // Usually "Document is not focused": the host switched windows while it was built.
    uncopied.set(lobby.id, built);
    const reason = err instanceof Error ? err.message : String(err);
    throw new Error(`Built the code but couldn't copy it (${reason}). Click again to copy.`, { cause: err });
  }
  const v = built.values;
  const skipped = built.skippedNames ? `, ${built.skippedNames} names left out (the Workshop can't show them)` : "";
  setCodeLine(lobby.id, {
    text: `Copied. ${v.name}, ${v.label}: ${v.roundLimit} rounds, lobby key ${v.lobbyKey}. Genji Ball ${built.release} with ${ctx.regionLabel(built.region)}'s rank tags (top ${built.top}, ${built.names} more${skipped}). Import it, then set the preset (Workshop settings, 00 - Preset) to Tournament.`,
    tone: "good",
  });
}

/** The lobby's verify screenshot: where it stands, and the panel to upload, replace or delete it. */
function screenshotBlock(lobby: LobbyView): HTMLElement[] {
  const state = screenshotState(lobby);
  if (!state) return [];
  const parts: HTMLElement[] = [node("p", state.text, state.tone)];
  if (lobby.verified) return parts;
  const isOpen = openShot === lobby.id;
  const row = node("div", "", "row");
  row.append(
    button(isOpen ? "Close" : lobby.screenshot ? "Replace screenshot" : "Add screenshot", true, async () => {
      openShot = isOpen ? null : lobby.id;
      draw();
      await watchNewest();
    }),
  );
  if (lobby.screenshot) {
    row.append(
      button("Delete screenshot", true, async () => {
        const next = await invoke<TourneysStatus>("delete_screenshot", { lobbyId: lobby.id });
        shotLines.set(lobby.id, { text: "Deleted.", tone: "muted" });
        renderTourneys(next);
      }, (m) => setShotLine(lobby.id, { text: m, tone: "bad" })),
    );
  }
  parts.push(row);
  if (isOpen) parts.push(shotPanel(lobby));
  const line = shotLines.get(lobby.id);
  if (line) parts.push(node("p", line.text, line.tone));
  return parts;
}

function setShotLine(id: number, line: Line): void {
  shotLines.set(id, line);
  draw();
}

function shotPanel(lobby: LobbyView): HTMLElement {
  const panel = node("div", "", "shot");
  const candidate = candidates.get(lobby.id);
  const preview = node("div", "", "shot-preview");
  if (candidate) {
    const img = node("img");
    img.src = candidate.url;
    img.alt = "The screenshot to upload";
    preview.append(img);
  } else {
    preview.append(node("span", "No image yet. Take the screenshot while the final standings are on screen.", "muted"));
  }
  panel.append(preview);
  if (candidate) {
    panel.append(node("p", candidate.caption, "muted"));
    if (candidate.newest && takenBeforeStart(candidate.newest.takenAt, lobby.tourney.startsAt)) {
      panel.append(node("p", "This screenshot was taken before the tourney started: check it's this lobby's final standings.", "warn"));
    }
  }
  const row = node("div", "", "row");
  const fail = (m: string) => setShotLine(lobby.id, { text: m, tone: "bad" });
  row.append(
    button("Use newest screenshot", true, () => useNewest(lobby.id, true), fail),
    button("Choose image…", true, async () => {
      const path = await open({ multiple: false, directory: false, defaultPath: context().screenshotFolder ?? undefined, title: "Verify screenshot", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }] });
      if (typeof path === "string") await usePath(lobby.id, path, `Chosen: ${path}`);
    }, fail),
  );
  const upload = button(lobby.screenshot ? "Upload, replacing the one there" : "Upload", false, () => uploadShot(lobby), fail);
  off(upload, !candidate);
  row.append(upload);
  panel.append(row);
  panel.append(node("p", "Or drop an image on this window, or paste one (Ctrl+V), from any folder or capture tool. PNG, JPEG or WebP.", "note"));
  return panel;
}

function dropCandidate(id: number): void {
  const old = candidates.get(id);
  if (old) URL.revokeObjectURL(old.url);
  candidates.delete(id);
}

/** Makes `bytes` the lobby's image to upload, if the server would take it. */
function setCandidate(id: number, bytes: Uint8Array, caption: string, newest: ScreenshotFile | null): void {
  const checked = checkScreenshot(bytes, context().screenshotMaxBytes);
  if (!checked.ok) throw new Error(checked.error);
  dropCandidate(id);
  const url = URL.createObjectURL(new Blob([bytes as Uint8Array<ArrayBuffer>], { type: checked.type }));
  candidates.set(id, { bytes, type: checked.type, url, caption, newest });
  shotLines.delete(id);
  draw();
}

async function usePath(id: number, path: string, caption: string, newest: ScreenshotFile | null = null): Promise<void> {
  const bytes = new Uint8Array(await invoke<ArrayBuffer>("read_screenshot", { path }));
  setCandidate(id, bytes, caption, newest);
}

/** Offers the newest image in the screenshots folder (`asked`: the host clicked, so say when there's none). */
async function useNewest(id: number, asked: boolean): Promise<void> {
  const newest = await invoke<ScreenshotFile | null>("newest_screenshot");
  if (!newest) {
    if (asked) throw new Error("There's no image in the screenshots folder yet");
    return;
  }
  const current = candidates.get(id)?.newest;
  if (!asked && current && current.path === newest.path && current.takenAt === newest.takenAt) return;
  await usePath(id, newest.path, `Newest in your screenshots folder: ${newest.name}, taken ${new Date(newest.takenAt).toLocaleString()}`, newest);
}

let watching = false;

/**
 * While a lobby's panel is open and it has no image the host chose, offers the newest screenshot,
 * and a newer one as soon as it's taken: every `screenshotPollSecs` until the panel closes.
 */
async function watchNewest(): Promise<void> {
  if (watching) return;
  watching = true;
  try {
    while (openShot !== null) {
      const id = openShot;
      const candidate = candidates.get(id);
      if (!candidate || candidate.newest) {
        try {
          await useNewest(id, false);
        } catch {
          // No folder, or a file being written: the next look tries again.
        }
      }
      await new Promise((resolve) => setTimeout(resolve, context().screenshotPollSecs * 1000));
    }
  } finally {
    watching = false;
  }
}

async function uploadShot(lobby: LobbyView): Promise<void> {
  const candidate = candidates.get(lobby.id);
  if (!candidate) throw new Error("Pick an image first");
  setShotLine(lobby.id, { text: "Uploading…", tone: "muted" });
  const next = await invoke<TourneysStatus>("upload_screenshot", candidate.bytes, { headers: { "lobby-id": String(lobby.id) } });
  dropCandidate(lobby.id);
  openShot = null;
  shotLines.set(lobby.id, { text: "Uploaded. An admin checks it against the match.", tone: "good" });
  renderTourneys(next);
}

/** The lobby a dropped or pasted image is for: the open panel's, else the one lobby that needs one. */
function shotTarget(): number | null {
  if (openShot !== null) return openShot;
  const due = status?.lobbies.filter((l) => l.needsScreenshot) ?? [];
  const [only] = due;
  return due.length === 1 && only ? only.id : null;
}

function takeImage(load: (id: number) => Promise<void>): void {
  const id = shotTarget();
  if (id === null) return;
  openShot = id;
  void context().busy(
    () => load(id),
    (m) => setShotLine(id, { text: m, tone: "bad" }),
  );
}

/** Binds the section's buttons, drops and pastes, and keeps the countdowns going. Call once. */
export async function setupTourneys(get: () => TourneyContext): Promise<void> {
  context = get;
  el("tourneys-check").addEventListener("click", () => {
    showError("");
    void context().busy(async () => renderTourneys(await invoke<TourneysStatus>("check_tourneys")), showError);
  });

  // Tauri hands a dropped file over as its path (the page gets no file).
  await getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type !== "drop") return;
    const [path] = event.payload.paths;
    if (!path) return;
    takeImage((id) => usePath(id, path, `Dropped: ${path}`));
  });
  document.addEventListener("paste", (event) => {
    const target = event.target;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
    const file = [...(event.clipboardData?.files ?? [])].find((f) => f.type.startsWith("image/"));
    if (!file) return;
    event.preventDefault();
    takeImage(async (id) => setCandidate(id, new Uint8Array(await file.arrayBuffer()), "Pasted from the clipboard", null));
  });

  setInterval(() => {
    const now = Date.now();
    document.querySelectorAll<HTMLElement>("#tourney-list [data-until]").forEach((span) => {
      span.textContent = countdown(new Date(span.dataset.until ?? "").getTime() - now);
    });
    // A code window opened (or a start passed): draw the buttons again.
    if (status?.lobbies.some((l) => drawnStates.get(l.id) !== codeState(l, new Date(now)).kind)) draw();
  }, COUNTDOWN_TICK_MS);
}
