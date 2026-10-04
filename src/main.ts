import { invoke } from "@tauri-apps/api/core";
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

/** Mirrors `TokenCheck` in src-tauri/src/server.rs. */
type TokenCheck =
  | { result: "ok"; host: { id: number; name: string; trust: "trusted" | "untrusted" } }
  | { result: "unknown" }
  | { result: "revoked" }
  | { result: "unreachable"; message: string };

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

function showCheck(check: TokenCheck): void {
  switch (check.result) {
    case "ok":
      el("host").textContent = check.host.name;
      setStatus(check.host.trust === "trusted" ? "Token works" : "Token works. Untrusted host: an admin reviews your matches", "good");
      break;
    case "unknown":
      el("host").textContent = "";
      setStatus("The server doesn't know this token", "bad");
      break;
    case "revoked":
      el("host").textContent = "";
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
  if (!state.hasToken) el("host").textContent = "No token yet";

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
    el("host").textContent = "";
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

void busy(async () => {
  await refresh();
  if (state.hasToken) await checkSaved();
  else el("token").focus();
});
