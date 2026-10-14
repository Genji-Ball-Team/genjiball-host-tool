/**
 * The Uploads view: where uploads stand, the region they go as, and the history, a page at a time,
 * each upload with its answer, "Retry now", "View on the site" and "Show match". What it says is
 * worked out in `uploads-model.ts`.
 */
import { invoke } from "@tauri-apps/api/core";
import { el } from "./dom";
import { flash } from "./home-view";
import { watchLog, type LogWatch } from "./match-view";
import { tourneyLabel } from "./tourneys-view";
import { answerRegions, describeAnswer, describeQueued, describeUploads, onSite, type HistoryEntry, type HistoryPage, type UploadStatus } from "./uploads-model";
import { markView } from "./views";

/** What the view needs from the rest of the window. */
export interface UploadsContext {
  serverUrl: string;
  /** How often a match view reads its log again, in seconds. */
  matchViewPollSecs: number;
  regionLabel(id: string): string;
  /** Runs a button's action with the buttons disabled, and shows its error. */
  busy(action: () => Promise<void>, showError: (message: string) => void): Promise<void>;
}

let context: () => UploadsContext;

/** Draws an upload status for the settings shown. */
export function renderUploads(status: UploadStatus): void {
  renderUploadRegion(status);
  const line = el("upload-state");
  line.className = status.problem ? "bad" : "";
  line.textContent = describeUploads(status);

  const retrying = el("upload-retrying");
  retrying.hidden = !status.retrying;
  // No token yet: Home shows the setup instead.
  markView("uploads", Boolean((status.problem && status.problem.kind !== "noToken") || status.retrying));
  retrying.textContent = status.retrying ? `Last upload failed, retrying: ${status.retrying}` : "";

  // The status carries the newest page. An older one is asked for again when anything in the
  // history changed, since it may be on that page or have moved it.
  const changed = status.historyRevision !== historyRevision;
  historyRevision = status.historyRevision;
  if (historyPage === 0) {
    historyRequest++; // Newer than any page 0 still on its way.
    renderHistory(status.history);
  } else if (changed) {
    showHistoryPage(historyPage).catch(showUploadsError);
  }
}

/** Which region matches upload as, so a night in the other one isn't stored as this one. */
function renderUploadRegion(status: UploadStatus): void {
  const line = el("upload-region");
  line.hidden = status.problem?.kind === "noRegion";
  if (!status.region) {
    line.textContent = "Uploading as your home region.";
    return;
  }
  const strong = document.createElement("strong");
  strong.textContent = context().regionLabel(status.region);
  line.replaceChildren("Uploading as ", strong, status.chosenRegion ? "." : " (your home region).");
}
function span(text: string, className: string): HTMLSpanElement {
  const found = document.createElement("span");
  found.className = className;
  found.textContent = text;
  return found;
}

function button(text: string, action: () => Promise<void>): HTMLButtonElement {
  const found = document.createElement("button");
  found.type = "button";
  found.className = "quiet";
  found.textContent = text;
  found.addEventListener("click", () => {
    el("uploads-error").hidden = true;
    void context().busy(action, showUploadsError);
  });
  return found;
}

function historyItem(entry: HistoryEntry): HTMLLIElement {
  const item = document.createElement("li");
  const regions = entry.answer ? answerRegions(entry.answer) : "";
  const at = entry.at ? new Date(entry.at).toLocaleString() : "Not uploaded yet";
  item.append(span(entry.file, "path"), span(regions ? `${regions} · ${at}` : at, "muted"));
  if (entry.players.length) item.append(span(entry.players.join(", "), "soft"));
  for (const t of entry.tourneys) {
    const name = tourneyLabel(t.lobbyKey) ?? `lobby key ${t.lobbyKey}`;
    item.append(span(`Tourney match: ${name}${t.ended ? "" : ", not ended yet"}`, "soft"));
  }

  const answer = entry.answer && describeAnswer(entry.answer);
  const now = entry.queued ? describeQueued(entry.queued) : answer;
  if (now) item.append(span(now.text, now.tone));
  // A file uploaded before that grew since: what the server said to the shorter copy.
  if (entry.queued && answer) item.append(span(`Last upload: ${answer.text}`, "muted"));

  const actions = document.createElement("div");
  actions.className = "row";
  if (entry.queued?.kind === "failed") {
    actions.append(button("Retry now", () => invoke<void>("retry_upload", { file: entry.file })));
  }
  const matches = entry.answer?.kind === "answered" ? entry.answer.matches.filter(onSite) : [];
  for (const match of matches) {
    actions.append(button(matches.length > 1 ? `Match ${match.matchId} on the site` : "View on the site", () => invoke<void>("open_match", { matchId: match.matchId })));
  }
  actions.append(matchToggle(entry, item));
  item.append(actions);
  const shown = shownMatches.get(entry.file);
  if (shown) item.append(shown.box);
  return item;
}

/** The match views open in the upload list, by file: kept while the list is drawn again. */
const shownMatches = new Map<string, { box: HTMLElement; watch: LogWatch }>();

/** "Show match": the file's matches as the server will read them, under its line. */
function matchToggle(entry: HistoryEntry, item: HTMLLIElement): HTMLButtonElement {
  const open = shownMatches.has(entry.file);
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "quiet";
  toggle.textContent = open ? "Hide match" : "Show match";
  toggle.setAttribute("aria-expanded", String(open));
  toggle.addEventListener("click", () => {
    const shown = shownMatches.get(entry.file);
    if (shown) {
      shown.watch.stop();
      shownMatches.delete(entry.file);
    } else {
      const box = document.createElement("div");
      shownMatches.set(entry.file, { box, watch: watchLog(box, { kind: "file", file: entry.file }, () => context().matchViewPollSecs) });
    }
    const next = historyItem(entry);
    item.replaceWith(next);
    next.querySelector<HTMLButtonElement>("button[aria-expanded]")?.focus();
  });
  return toggle;
}

/** The history page the host asked for, from 0 (the newest). */
let historyPage = 0;
/** Counts history requests: the answer to one that a later request replaced is dropped. */
let historyRequest = 0;
/** `UploadStatus.historyRevision` last seen. */
let historyRevision = -1;

/** The newest page's uploads at its last drawing, by file and time: a new one is lit up. */
let seenUploads: { server: string; keys: Set<string> } | null = null;
/** When each new upload came in, while it's lit up (the `.fresh` animation's length). */
const freshSince = new Map<string, number>();
const FRESH_MS = 2000;

function renderHistory(page: HistoryPage): void {
  // Past the end gives the last page: stay there.
  historyPage = page.page;
  // A match view stops reading once its file is off the page.
  for (const [file, shown] of shownMatches) {
    if (page.entries.some((e) => e.file === file)) continue;
    shown.watch.stop();
    shownMatches.delete(file);
  }
  const items = page.entries.map(historyItem);
  el("uploads").replaceChildren(...items);
  // New since the last newest page of this server: lit up once, and still while it's drawn again.
  const keys = page.entries.map((e) => `${e.file}|${e.at ?? ""}`);
  if (page.page === 0) {
    const now = Date.now();
    const server = context().serverUrl;
    if (seenUploads?.server === server) {
      for (const key of keys) if (!seenUploads.keys.has(key)) freshSince.set(key, now);
    } else freshSince.clear();
    seenUploads = { server, keys: new Set(keys) };
    items.forEach((item, i) => {
      const since = freshSince.get(keys[i] ?? "");
      if (since === undefined) return;
      if (now - since > FRESH_MS) freshSince.delete(keys[i] ?? "");
      else flash(item, now - since);
    });
  }
  const first = page.page * page.pageSize;
  el("uploads-pager").hidden = page.total <= page.pageSize;
  el("uploads-newer").hidden = page.page === 0;
  el("uploads-older").hidden = first + page.entries.length >= page.total;
  el("uploads-range").textContent = page.total ? `${first + 1}–${first + page.entries.length} of ${page.total}` : "";
}

/** Asks for a page of the history and shows it, unless another page was asked for meanwhile. */
export async function showHistoryPage(page: number): Promise<void> {
  historyPage = page;
  const request = ++historyRequest;
  const found = await invoke<HistoryPage>("get_upload_history", { page });
  if (request === historyRequest) renderHistory(found);
}

function showUploadsError(err: unknown): void {
  const error = el("uploads-error");
  error.hidden = false;
  error.textContent = String(err);
}

/** Another log folder: the open match views stop reading. */
export function closeMatchViews(): void {
  for (const shown of shownMatches.values()) shown.watch.stop();
  shownMatches.clear();
}

/** Binds the pager. Call once. */
export function setupUploads(get: () => UploadsContext): void {
  context = get;
  el("uploads-newer").addEventListener("click", () => void context().busy(() => showHistoryPage(historyPage - 1), showUploadsError));
  el("uploads-older").addEventListener("click", () => void context().busy(() => showHistoryPage(historyPage + 1), showUploadsError));
}
