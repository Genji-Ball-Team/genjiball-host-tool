import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

/** Mirrors `AppState` in src-tauri/src/lib.rs. */
interface AppState {
  version: string;
  serverUrl: string;
  defaultServerUrl: string;
  hasToken: boolean;
  logFolder: LogFolder | null;
  settingsError: string | null;
}

/** Mirrors `LogFolder` in src-tauri/src/log_folder.rs. */
interface LogFolder {
  path: string;
  source: "custom" | "detected";
  exists: boolean;
}

/** Mirrors `Host` in src-tauri/src/server.rs. */
interface Host {
  id: number;
  name: string;
  trust: "trusted" | "untrusted";
}

/** Mirrors `TokenCheck` in src-tauri/src/server.rs. */
type TokenCheck =
  | { result: "ok"; host: Host }
  | { result: "unknown" }
  | { result: "revoked" }
  | { result: "unreachable"; message: string };

/** Mirrors `UploadStatus` in src-tauri/src/uploader.rs. */
interface UploadStatus {
  serverUrl: string;
  logFolder: string | null;
  problem: Problem | null;
  waiting: number;
  retrying: string | null;
  history: HistoryPage;
  historyRevision: number;
  host: Host | null;
}

/** Mirrors `Problem` in src-tauri/src/uploader.rs. */
type Problem =
  | { kind: "settings" }
  | { kind: "noFolder" }
  | { kind: "folderUnreadable"; message: string }
  | { kind: "noToken" }
  | { kind: "tokenRejected"; revoked: boolean }
  | { kind: "local"; message: string };

/** Mirrors `Page` in src-tauri/src/history.rs: what `get_upload_history` returns. */
interface HistoryPage {
  entries: HistoryEntry[];
  page: number;
  pageSize: number;
  total: number;
}

/** Mirrors `Entry` in src-tauri/src/history.rs. */
interface HistoryEntry {
  file: string;
  at: string | null;
  players: string[];
  answer: Answer | null;
  queued: QueueState | null;
}

/** Mirrors `Answer` in src-tauri/src/uploads.rs. */
type Answer = { kind: "answered"; result: "stored" | "unchanged" | "duplicate"; matches: UploadedMatch[] } | { kind: "refused"; error: string; message: string };

/** Mirrors `QueueState` in src-tauri/src/watcher.rs. */
type QueueState = { kind: "playing" } | { kind: "due" } | { kind: "failed"; error: string };

/** Mirrors `UploadedMatch` in src-tauri/src/server.rs. */
interface UploadedMatch {
  matchKey: string | null;
  matchId: number | null;
  lineCount: number;
  action: "insert" | "replace" | "repoint" | "skip";
  status: "accepted" | "review" | "rejected" | "void";
  rejection: { code: string; message: string } | null;
  reviewReasons: string[];
}

/** Mirrors `RankedCode` in src-tauri/src/lib.rs. */
interface RankedCode {
  code: string;
  serverUrl: string;
  release: string;
  tagsUpdatedAt: string;
  names: number;
  skippedNames: number;
  keepSecs: number;
}

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found as T;
}

let state: AppState;
/** Set once `state` is: an upload status is only shown for the settings in it. */
let ready = false;
let editingToken = false;

function setStatus(text: string, tone: "good" | "bad" | "muted" = "muted"): void {
  const status = el("token-status");
  status.textContent = text;
  status.className = `status ${tone}`;
}

/** The host's name and, while they're untrusted, a tag saying so. `null` hides both. */
function showHost(host: Host | null): void {
  el("host").textContent = host?.name ?? "";
  const untrusted = host?.trust === "untrusted";
  el("host-trust").hidden = !untrusted;
  el("host-trust-note").hidden = !untrusted;
}

function showCheck(check: TokenCheck): void {
  switch (check.result) {
    case "ok":
      showHost(check.host);
      setStatus("Token works", "good");
      break;
    case "unknown":
      showHost(null);
      setStatus("The server doesn't know this token", "bad");
      break;
    case "revoked":
      showHost(null);
      setStatus("This token was revoked. Ask an admin for a new one", "bad");
      break;
    case "unreachable":
      setStatus(check.message, "bad");
      break;
  }
}

function render(): void {
  el("version").textContent = `v${state.version}`;
  const settingsError = el("settings-error");
  settingsError.hidden = !state.settingsError;
  settingsError.textContent = state.settingsError ? `${state.settingsError}. Uploads are paused until it's fixed. The defaults are shown; changing a setting writes a new file.` : "";

  el("welcome").hidden = state.hasToken;
  el("server").textContent = state.serverUrl;
  el("default-server").textContent = state.defaultServerUrl;
  const serverUrl = el<HTMLInputElement>("server-url");
  if (document.activeElement !== serverUrl) {
    serverUrl.value = state.serverUrl === state.defaultServerUrl ? "" : state.serverUrl;
  }
  if (state.serverUrl !== state.defaultServerUrl) el<HTMLDetailsElement>("advanced").open = true;

  const askToken = !state.hasToken || editingToken;
  el("token-form").hidden = !askToken;
  el("token-cancel").hidden = !state.hasToken;
  el("token-actions").hidden = askToken;
  if (!state.hasToken) {
    showHost(null);
    el("host").textContent = "No token yet";
  }

  const folder = state.logFolder;
  el("log-folder").textContent = folder?.path ?? "No Documents folder found";
  el("log-folder-note").textContent = !folder
    ? "Choose the folder Overwatch writes Workshop logs to."
    : !folder.exists
      ? "This folder isn't there yet. Overwatch creates it the first time the Workshop writes a log, once log files are on in the game's settings."
      : folder.source === "detected"
        ? "Found automatically."
        : "Chosen by you.";
  el("log-folder-reset").hidden = folder?.source !== "custom";
}

function describeProblem(problem: Problem): string {
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
    case "local":
      return problem.message;
  }
}

/** Why a match waits for an admin. An untrusted host's every match does: the host line says so. */
const reviewReasons: Record<string, string | null> = {
  duplicate_name: "two players with the same name",
  untrusted_host: null,
};

function describeMatch(match: UploadedMatch): string {
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

type Tone = "good" | "bad" | "muted";

function describeAnswer(answer: Answer): { text: string; tone: Tone } {
  if (answer.kind === "refused") return { text: answer.message, tone: "bad" };
  if (answer.result === "duplicate") return { text: "Already uploaded", tone: "muted" };
  if (!answer.matches.length) return { text: "No match in it", tone: "muted" };
  const text = answer.matches.map(describeMatch).join("; ");
  const tone = answer.matches.every((m) => m.status === "accepted") ? "good" : answer.matches.some((m) => m.status === "rejected") ? "bad" : "muted";
  return { text: text.charAt(0).toUpperCase() + text.slice(1), tone };
}

function renderUploads(status: UploadStatus): void {
  // A poll that began before the host changed the server or folder: not about what's shown.
  // Dropped before it touches the history, so it can't replace the new server's page.
  if (status.serverUrl !== state.serverUrl || status.logFolder !== (state.logFolder?.path ?? null)) return;
  if (status.host) showHost(status.host);
  else if (status.problem?.kind === "tokenRejected") showHost(null);
  const line = el("upload-state");
  line.className = status.problem ? "bad" : "";
  line.textContent = status.problem
    ? describeProblem(status.problem)
    : status.waiting === 1
      ? "Watching. 1 ranked log to upload."
      : status.waiting
        ? `Watching. ${status.waiting} ranked logs to upload.`
        : "Watching for ranked matches.";

  const retrying = el("upload-retrying");
  retrying.hidden = !status.retrying;
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

function describeQueued(queued: QueueState): { text: string; tone: Tone } {
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
function onSite(match: UploadedMatch): match is UploadedMatch & { matchId: number } {
  return match.matchId !== null && (match.status === "accepted" || match.status === "void");
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
    void busy(action, showUploadsError);
  });
  return found;
}

function historyItem(entry: HistoryEntry): HTMLLIElement {
  const item = document.createElement("li");
  item.append(span(entry.file, "path"), span(entry.at ? new Date(entry.at).toLocaleString() : "Not uploaded yet", "muted"));
  if (entry.players.length) item.append(span(entry.players.join(", "), "soft"));

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
  if (actions.childElementCount) item.append(actions);
  return item;
}

/** The history page the host asked for, from 0 (the newest). */
let historyPage = 0;
/** Counts history requests: the answer to one that a later request replaced is dropped. */
let historyRequest = 0;
/** `UploadStatus.historyRevision` last seen. */
let historyRevision = -1;

function renderHistory(page: HistoryPage): void {
  // Past the end gives the last page: stay there.
  historyPage = page.page;
  el("uploads").replaceChildren(...page.entries.map(historyItem));
  const first = page.page * page.pageSize;
  el("uploads-pager").hidden = page.total <= page.pageSize;
  el("uploads-newer").hidden = page.page === 0;
  el("uploads-older").hidden = first + page.entries.length >= page.total;
  el("uploads-range").textContent = page.total ? `${first + 1}–${first + page.entries.length} of ${page.total}` : "";
}

/** Asks for a page of the history and shows it, unless another page was asked for meanwhile. */
async function showHistoryPage(page: number): Promise<void> {
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

/** Shows these settings, then the upload status for them (one sent before they were known was dropped). */
async function show(next: AppState): Promise<void> {
  state = next;
  ready = true;
  render();
  renderUploads(await invoke<UploadStatus>("get_upload_status"));
}

async function refresh(): Promise<void> {
  await show(await invoke<AppState>("get_state"));
}

/** Actions running now. The buttons come back only when the last one ends. */
let running = 0;

function setButtonsDisabled(disabled: boolean): void {
  document.querySelectorAll("button").forEach((b) => (b.disabled = disabled));
}

/** Runs a button's action with the buttons disabled, and shows its error (by default next to the token). */
async function busy(action: () => Promise<void>, showError: (message: string) => void = (m) => setStatus(m, "bad")): Promise<void> {
  running += 1;
  setButtonsDisabled(true);
  try {
    await action();
  } catch (err) {
    // Commands fail with strings, the browser with `Error`s.
    showError(err instanceof Error ? err.message : String(err));
  } finally {
    running -= 1;
    if (!running) setButtonsDisabled(false);
  }
}

async function checkSaved(): Promise<void> {
  setStatus("Checking…");
  showCheck(await invoke<TokenCheck>("check_saved_token"));
}

el("token-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void busy(async () => {
    const input = el<HTMLInputElement>("token");
    setStatus("Checking…");
    const check = await invoke<TokenCheck>("save_token", { token: input.value });
    if (check.result !== "unknown" && check.result !== "revoked") {
      input.value = "";
      editingToken = false;
      await refresh();
    }
    showCheck(check);
    if (check.result === "unreachable") {
      // The new token's host isn't known yet: don't show the old one's.
      showHost(null);
      setStatus(`Saved, but not checked: ${check.message}`, "bad");
    }
  });
});

el("token-cancel").addEventListener("click", () => {
  editingToken = false;
  el<HTMLInputElement>("token").value = "";
  render();
});
el("token-check").addEventListener("click", () => void busy(checkSaved));
el("token-change").addEventListener("click", () => {
  editingToken = true;
  render();
  el("token").focus();
});
el("token-forget").addEventListener("click", () => {
  void busy(async () => {
    await invoke("forget_token");
    setStatus("");
    await refresh();
  });
});

el("server-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void busy(async () => {
    const next = await invoke<AppState>("set_server_url", { url: el<HTMLInputElement>("server-url").value });
    // A code kept for the old server's tags is no use now.
    uncopied = null;
    showHost(null);
    setStatus("");
    await show(next);
    // Another server's history, from its newest page: drops any page of the old one on its way.
    await showHistoryPage(0);
    if (state.hasToken) await checkSaved();
  });
});

el("log-folder-choose").addEventListener("click", () => {
  void busy(async () => {
    const path = await open({ directory: true, defaultPath: state.logFolder?.path, title: "Workshop log folder" });
    if (typeof path !== "string") return;
    await show(await invoke<AppState>("set_log_folder", { path }));
  });
});
el("log-folder-reset").addEventListener("click", () => {
  void busy(async () => {
    await show(await invoke<AppState>("set_log_folder", { path: null }));
  });
});

function setRankedCodeState(text: string, tone: "good" | "bad" | "muted"): void {
  const line = el("ranked-code-state");
  line.hidden = !text;
  line.textContent = text;
  line.className = tone;
}

/** Counts builds, so only the latest click's code is copied. */
let rankedBuild = 0;
/**
 * The last code built but not copied (the window lost focus while it was built): the next click
 * copies it as is, if it's for the current server and younger than its `keepSecs`.
 */
let uncopied: { built: RankedCode; at: number } | null = null;

function takeUncopied(): { built: RankedCode; at: number } | null {
  const kept = uncopied;
  uncopied = null;
  if (!kept || kept.built.serverUrl !== state.serverUrl) return null;
  return Date.now() - kept.at < kept.built.keepSecs * 1000 ? kept : null;
}

async function copyRankedCode(): Promise<void> {
  const build = ++rankedBuild;
  const kept = takeUncopied();
  let built: RankedCode;
  let builtAt: number;
  if (kept) {
    built = kept.built;
    builtAt = kept.at;
  } else {
    setRankedCodeState("Building the code…", "muted");
    built = await invoke<RankedCode>("build_ranked_code");
    builtAt = Date.now();
    // A newer click, or a server change, while this one ran: its tags may be from the wrong server.
    if (build !== rankedBuild) return;
    if (built.serverUrl !== state.serverUrl) {
      setRankedCodeState("The server changed while the code was built. Click again for this server's.", "bad");
      return;
    }
  }
  try {
    await navigator.clipboard.writeText(built.code);
  } catch (err) {
    // Usually "Document is not focused": the host switched windows while it was built.
    uncopied = { built, at: builtAt };
    const reason = err instanceof Error ? err.message : String(err);
    throw new Error(`Built the code but couldn't copy it (${reason}). Click again to copy.`, { cause: err });
  }
  const names = built.names === 1 ? "1 name" : `${built.names} names`;
  const skipped = built.skippedNames ? ` (${built.skippedNames} left out: the Workshop can't show them)` : "";
  setRankedCodeState(`Copied. Genji Ball ${built.release}, rank tags from ${new Date(built.tagsUpdatedAt).toLocaleString()}, ${names}${skipped}.`, "good");
}

el("ranked-code-copy").addEventListener("click", () => void busy(copyRankedCode, (m) => setRankedCodeState(m, "bad")));

el("uploads-newer").addEventListener("click", () => void busy(() => showHistoryPage(historyPage - 1), showUploadsError));
el("uploads-older").addEventListener("click", () => void busy(() => showHistoryPage(historyPage + 1), showUploadsError));

void busy(async () => {
  await listen<UploadStatus>("upload-status", (event) => {
    if (!ready) return; // `show` asks for the status once the settings are known.
    // The settings file was fixed by hand (the uploader reads it again): show what's in it now.
    if (state.settingsError && event.payload.problem?.kind !== "settings") void refresh();
    else renderUploads(event.payload);
  });
  await refresh();
  if (state.hasToken) await checkSaved();
  else el("token").focus();
});
