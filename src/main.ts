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
  problem: Problem | null;
  waiting: number;
  retrying: string | null;
  recent: RecentUpload[];
  host: Host | null;
}

/** Mirrors `Problem` in src-tauri/src/uploader.rs. */
type Problem =
  | { kind: "noFolder" }
  | { kind: "folderUnreadable"; message: string }
  | { kind: "noToken" }
  | { kind: "tokenRejected"; revoked: boolean }
  | { kind: "local"; message: string };

/** Mirrors `RecentUpload` and `Answer` in src-tauri/src/uploads.rs. */
interface RecentUpload {
  file: string;
  at: string;
  answer: { kind: "answered"; result: "stored" | "unchanged" | "duplicate"; matches: UploadedMatch[] } | { kind: "refused"; error: string; message: string };
}

/** Mirrors `UploadedMatch` in src-tauri/src/server.rs. */
interface UploadedMatch {
  matchKey: string | null;
  lineCount: number;
  action: "insert" | "replace" | "repoint" | "skip";
  status: "accepted" | "review" | "rejected" | "void";
  rejection: { code: string; message: string } | null;
  reviewReasons: string[];
}

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found as T;
}

let state: AppState;
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
  settingsError.textContent = state.settingsError ? `${state.settingsError}. Using the defaults; changing a setting writes a new file.` : "";

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
      const reasons = match.reviewReasons.map((r) => (r in reviewReasons ? reviewReasons[r] : r)).filter((r) => r !== null);
      return reasons.length ? `waiting for an admin: ${reasons.join(", ")}` : "waiting for an admin";
    }
    case "rejected":
      return `rejected: ${match.rejection?.message || match.rejection?.code || "no reason given"}`;
    case "void":
      return "voided by an admin";
  }
}

function describeUpload(upload: RecentUpload): { text: string; tone: "good" | "bad" | "muted" } {
  const answer = upload.answer;
  if (answer.kind === "refused") return { text: answer.message, tone: "bad" };
  if (answer.result === "duplicate") return { text: "Already uploaded", tone: "muted" };
  if (!answer.matches.length) return { text: "No match in it", tone: "muted" };
  const text = answer.matches.map(describeMatch).join("; ");
  const tone = answer.matches.every((m) => m.status === "accepted") ? "good" : answer.matches.some((m) => m.status === "rejected") ? "bad" : "muted";
  return { text: text.charAt(0).toUpperCase() + text.slice(1), tone };
}

function renderUploads(status: UploadStatus): void {
  if (status.host) showHost(status.host);
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

  el("uploads").replaceChildren(
    ...status.recent.map((upload) => {
      const { text, tone } = describeUpload(upload);
      const item = document.createElement("li");
      const file = document.createElement("span");
      file.className = "path";
      file.textContent = upload.file;
      const at = document.createElement("span");
      at.className = "muted";
      at.textContent = new Date(upload.at).toLocaleString();
      const result = document.createElement("span");
      result.className = tone;
      result.textContent = text;
      item.append(file, at, result);
      return item;
    }),
  );
}

async function refresh(): Promise<void> {
  state = await invoke<AppState>("get_state");
  render();
}

/** Runs a button's action with the buttons disabled, and shows its error. */
async function busy(action: () => Promise<void>): Promise<void> {
  const buttons = [...document.querySelectorAll("button")];
  buttons.forEach((b) => (b.disabled = true));
  try {
    await action();
  } catch (err) {
    setStatus(String(err), "bad");
  } finally {
    buttons.forEach((b) => (b.disabled = false));
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
    if (check.result === "unreachable") setStatus(`Saved, but not checked: ${check.message}`, "bad");
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
    state = await invoke<AppState>("set_server_url", { url: el<HTMLInputElement>("server-url").value });
    showHost(null);
    setStatus("");
    render();
    if (state.hasToken) await checkSaved();
  });
});

el("log-folder-choose").addEventListener("click", () => {
  void busy(async () => {
    const path = await open({ directory: true, defaultPath: state.logFolder?.path, title: "Workshop log folder" });
    if (typeof path !== "string") return;
    state = await invoke<AppState>("set_log_folder", { path });
    render();
  });
});
el("log-folder-reset").addEventListener("click", () => {
  void busy(async () => {
    state = await invoke<AppState>("set_log_folder", { path: null });
    render();
  });
});

void listen<UploadStatus>("upload-status", (event) => renderUploads(event.payload));
void invoke<UploadStatus>("get_upload_status").then(renderUploads);

void busy(async () => {
  await refresh();
  if (state.hasToken) await checkSaved();
  else el("token").focus();
});
