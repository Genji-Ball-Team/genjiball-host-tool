import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { watchDebug } from "./debug-view";
import { copyBuilt, setupDesktop } from "./desktop";
import { el, errorText, fillSelect, node, setLine, type Tone } from "./dom";
import { watchLog, type LogWatch } from "./match-view";
import type { TourneysStatus } from "./tourney-model";
import type { Fix } from "./home-model";
import { homeFolderChanged, renderHome, setupHome } from "./home-view";
import { renderTourneys, screenshotsDue, setupTourneys, tourneyContextChanged } from "./tourneys-view";
import { setupTitlebar } from "./titlebar";
import { toast } from "./toast";
import { renderOverlaySettings, setupOverlaySettings, type OverlayView } from "./overlay-settings";
import { closeMatchViews, renderUploads, setupUploads, showHistoryPage } from "./uploads-view";
import { describeAnswer, describeQueued, type AfkStatus, type Host, type UploadStatus } from "./uploads-model";
import { currentView, onViewChange, setupViews, showPane, showView } from "./views";

/** Mirrors `AppState` in src-tauri/src/lib.rs. */
interface AppState {
  version: string;
  serverUrl: string;
  defaultServerUrl: string;
  hasToken: boolean;
  /** Why the saved token can't be read (`hasToken` is then false). */
  tokenError: string | null;
  logFolder: LogFolder | null;
  /** Where the verify screenshot of a tourney lobby is offered from. */
  screenshotFolder: LogFolder | null;
  /** The biggest verify screenshot the server takes, in bytes. */
  screenshotMaxBytes: number;
  /** How often the window looks for a new screenshot while it asks for one, in seconds. */
  screenshotPollSecs: number;
  /** The region the host picked, `null` for their home region. */
  region: string | null;
  regions: Region[];
  /** Whether the live lobby is on, and the name it's listed under (`null`: none). */
  liveLobby: boolean;
  lobbyName: string | null;
  lobbyNameMax: number;
  settingsError: string | null;
  advanced: AdvancedSetting[];
  /** How often the match view reads its log again, in seconds. */
  matchViewPollSecs: number;
  /** The log level the host picked, `null` for `defaultLogLevel`. */
  logLevel: string | null;
  logLevels: string[];
  defaultLogLevel: string;
  /** The GenjiBall-CE release the ranked code is built from, `null` for the latest ranked one. */
  releaseTag: string | null;
  releaseTagSuffix: string;
  /** Whether uploads are a dry run: picked as usual, but not sent. */
  dryRun: boolean;
  /** Whether the tool looks for updates by itself, and the channel it reads. */
  autoUpdateCheck: boolean;
  updateChannel: string;
  updateChannels: string[];
  /** Per region, the data centers its codes can put the lobby on (the default first), and the one in use. */
  dataCenters: DataCenterChoice[];
  /** The choice that leaves the data center to the game. */
  bestAvailable: string;
  /** The overlay and the stream page (Settings → Overlay). */
  overlay: OverlayView;
}

/** Mirrors `DataCenterChoice` in src-tauri/src/lib.rs. */
interface DataCenterChoice {
  region: string;
  names: string[];
  chosen: string;
}

/** Mirrors `Region` in src-tauri/src/config.rs. */
interface Region {
  id: string;
  label: string;
}

/** Mirrors `AdvancedSetting` in src-tauri/src/lib.rs. All in seconds. */
interface AdvancedSetting {
  key: string;
  label: string;
  help: string;
  default: number;
  min: number;
  max: number;
  value: number | null;
}

/** Mirrors `LogFolder` in src-tauri/src/log_folder.rs. */
interface LogFolder {
  path: string;
  source: "custom" | "detected";
  exists: boolean;
}

/** Mirrors `TokenCheck` in src-tauri/src/server.rs. */
type TokenCheck =
  | { result: "ok"; host: Host }
  | { result: "unknown" }
  | { result: "revoked" }
  | { result: "unreachable"; message: string };

/** Mirrors `RankedCode` in src-tauri/src/lib.rs. */
interface RankedCode {
  code: string;
  serverUrl: string;
  chosenRegion: string | null;
  /** The region the tags were asked for: the one picked, else the home region. */
  requestedRegion: string | null;
  /** Whose leaderboard the tags are from. */
  region: string | null;
  release: string;
  tagsUpdatedAt: string;
  top: number;
  names: number;
  skippedNames: number;
  keepSecs: number;
  /** The data center the code puts the lobby on, `null`: the game picks. */
  dataCenter: string | null;
}

/** Mirrors `LobbyStatus` in src-tauri/src/live_lobby.rs: what `get_lobby_status` returns. */
interface LobbyStatus {
  serverUrl: string;
  on: boolean;
  /** What's being played in the live log. */
  live: { kind: "idle" } | { kind: "unranked" } | { kind: "playing"; players: number };
  /** The lobby as the site lists it, `null` when it isn't listed. Mirrors `ListedLobby` in src-tauri/src/server.rs. */
  listed: { region: string; name: string | null; players: number } | null;
  problem: LobbyProblem | null;
}

/** Mirrors `LobbyProblem` in src-tauri/src/live_lobby.rs. */
type LobbyProblem =
  | { kind: "settings" }
  | { kind: "noFolder" }
  | { kind: "folderUnreadable"; message: string }
  | { kind: "noToken" }
  | { kind: "tokenRejected"; revoked: boolean }
  | { kind: "noRegion" }
  | { kind: "failed"; message: string };

/** Mirrors `UpdateStatus` in src-tauri/src/updates.rs. */
interface UpdateStatus {
  /** The version waiting to be installed. */
  available: string | null;
  checkedAt: string | null;
  /** Why the last check failed: shown quietly. */
  error: string | null;
}

let state: AppState;
/** Set once `state` is: an upload status is only shown for the settings in it. */
let ready = false;
let editingToken = false;

/** The token's state as last said, for Home too. */
let tokenLine: { text: string; tone: Tone } = { text: "", tone: "muted" };
/** The last token check's result, and why it couldn't reach the server. */
let tokenCheck: TokenCheck | null = null;

function setStatus(text: string, tone: Tone = "muted"): void {
  const status = el("token-status");
  status.textContent = text;
  status.className = `status ${tone}`;
  tokenLine = { text, tone };
  if (ready) renderHome();
}

/** The host's home region as the server last said (`null`: none), `undefined` while not known. */
let homeRegion: string | null | undefined;

function regionLabel(id: string): string {
  return state.regions.find((r) => r.id === id)?.label ?? id.toUpperCase();
}

/** The region choice: the home region (named once it's known), then each region. */
function renderRegion(): void {
  const select = el<HTMLSelectElement>("region");
  const home = homeRegion === undefined ? "Home region" : homeRegion === null ? "Home region (none set)" : `Home region (${regionLabel(homeRegion)})`;
  const options = [{ id: "", label: home }, ...state.regions];
  if (select.options.length !== options.length || select.options.item(0)?.textContent !== home) {
    fillSelect(select, options.map((r) => ({ value: r.id, label: r.label })));
  }
  select.value = state.region ?? "";
}

/** One data center choice per region, built once; each follows `state`. */
function renderDataCenters(): void {
  const box = el("data-centers");
  if (!box.childElementCount) {
    box.replaceChildren(
      ...state.dataCenters.map((d) => {
        const label = node("label", regionLabel(d.region));
        label.htmlFor = `data-center-${d.region}`;
        const select = node("select");
        select.id = `data-center-${d.region}`;
        fillSelect(
          select,
          [...d.names, state.bestAvailable].map((name, i) => ({
            value: name,
            label: name === state.bestAvailable ? "Best available (the game picks)" : i === 0 ? `${name} (default)` : name,
          })),
        );
        select.addEventListener("change", () => {
          void busy(
            async () => {
              const next = await invoke<AppState>("set_data_center", { region: d.region, name: select.value });
              // A code kept for the old data center is no use now.
              uncopied = null;
              await show(next);
              toast(`${regionLabel(d.region)}: ${select.value === state.bestAvailable ? "the game picks the data center" : `lobbies on ${select.value}`}`);
            },
            (m) => {
              toast(`Couldn't save the data center`, m);
              renderDataCenters();
            },
          );
        });
        const row = node("div", "", "row");
        row.append(label, select);
        return row;
      }),
    );
  }
  for (const d of state.dataCenters) el<HTMLSelectElement>(`data-center-${d.region}`).value = d.chosen;
}

function setRegionStatus(text: string, tone: Tone = "muted"): void {
  const status = el("region-status");
  status.textContent = text;
  status.className = `status ${tone}`;
}

/** The host as last known, for Home. */
let knownHost: Host | null = null;

/** The host's name and, while they're untrusted, a tag saying so. `null` hides both. */
function showHost(host: Host | null): void {
  knownHost = host;
  homeRegion = host ? host.region : undefined;
  renderRegion();
  el("host").textContent = host?.name ?? "";
  const untrusted = host?.trust === "untrusted";
  el("host-trust").hidden = !untrusted;
  el("host-trust-note").hidden = !untrusted;
}

function showCheck(check: TokenCheck): void {
  tokenCheck = check;
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

/** What a folder setting says about its folder: none found, not there yet, or where it came from. */
function folderNote(folder: LogFolder | null, none: string, missing: string): string {
  if (!folder) return none;
  if (!folder.exists) return missing;
  return folder.source === "detected" ? "Found automatically." : "Chosen by you.";
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

  const askToken = !state.hasToken || editingToken;
  el("token-form").hidden = !askToken;
  el("token-cancel").hidden = !state.hasToken;
  el("token-actions").hidden = askToken;
  if (!state.hasToken) {
    showHost(null);
    el("host").textContent = "No token yet";
    if (state.tokenError) setStatus(state.tokenError, "bad");
  }

  const folder = state.logFolder;
  el("log-folder").textContent = folder?.path ?? "No Documents folder found";
  el("log-folder-note").textContent = folderNote(
    folder,
    "Choose the folder Overwatch writes Workshop logs to.",
    "This folder isn't there yet. Overwatch creates it the first time the Workshop writes a log, once log files are on in the game's settings.",
  );
  el("log-folder-reset").hidden = folder?.source !== "custom";

  const shots = state.screenshotFolder;
  el("screenshot-folder").textContent = shots?.path ?? "No Documents folder found";
  el("screenshot-folder-note").textContent = folderNote(
    shots,
    "Choose the folder Overwatch saves screenshots to.",
    "This folder isn't there yet. Overwatch creates it with your first screenshot; if yours go elsewhere, choose that folder.",
  );
  el("screenshot-folder-reset").hidden = shots?.source !== "custom";
  if (shots?.source === "custom" || (shots && !shots.exists)) el<HTMLDetailsElement>("screenshot-settings").open = true;
  tourneyContextChanged();

  renderRegion();
  renderDataCenters();
  renderLobbySettings();
  renderAdvanced();
  renderUpdateSettings();
  renderDebug();
  renderOverlaySettings(state.overlay);
  renderHome();
}

const updateChannelLabels: Record<string, string> = { stable: "Stable", prerelease: "Pre-release" };

/** The update checks' switch and channel (Settings → Updates). */
function renderUpdateSettings(): void {
  el<HTMLInputElement>("update-auto").checked = state.autoUpdateCheck;
  const select = el<HTMLSelectElement>("update-channel");
  if (select.options.length !== state.updateChannels.length) {
    fillSelect(select, state.updateChannels.map((channel) => ({ value: channel, label: updateChannelLabels[channel] ?? channel })));
  }
  select.value = state.updateChannel;
  el("update-auto-note").textContent = state.autoUpdateCheck
    ? "The tool looks for a new version when it starts and every few hours."
    : "The tool doesn't look for new versions by itself.";
}

/** The ranked code release, the log level and the dry run (Settings → Advanced). */
function renderDebug(): void {
  const tag = el<HTMLInputElement>("release-tag");
  if (document.activeElement !== tag) tag.value = state.releaseTag ?? "";
  el("release-example").textContent = `1.3.3${state.releaseTagSuffix}`;

  const select = el<HTMLSelectElement>("log-level");
  if (select.options.length !== state.logLevels.length) {
    fillSelect(select, state.logLevels.map((level) => ({ value: level, label: level === state.defaultLogLevel ? `${level} (default)` : level })));
  }
  select.value = state.logLevel ?? state.defaultLogLevel;
  el<HTMLInputElement>("dry-run").checked = state.dryRun;
}

/** The live lobby's switch and name, as saved, unless the host is editing the name. */
function renderLobbySettings(): void {
  el<HTMLInputElement>("lobby-on").checked = state.liveLobby;
  const name = el<HTMLInputElement>("lobby-name");
  name.maxLength = state.lobbyNameMax;
  if (document.activeElement !== name) name.value = state.lobbyName ?? "";
}

function tunableText(t: AdvancedSetting): string {
  return t.value === null ? "" : String(t.value);
}

function tunableInput(key: string): HTMLInputElement {
  return el<HTMLInputElement>(`tunable-${key}`);
}

/** One field per Advanced setting, built once; their values follow `state` unless being edited. */
function renderAdvanced(): void {
  const list = el("tunables");
  if (!list.childElementCount) {
    list.replaceChildren(
      ...state.advanced.map((t) => {
        const item = node("div", "", "tunable");
        const label = node("label", t.label);
        label.htmlFor = `tunable-${t.key}`;
        const input = node("input");
        input.id = `tunable-${t.key}`;
        input.type = "number";
        input.min = String(t.min);
        input.max = String(t.max);
        input.step = "1";
        input.placeholder = String(t.default);
        const row = node("div", "", "row");
        row.append(input, node("span", `seconds (default ${t.default})`, "muted"));
        item.append(label, row, node("p", t.help, "note"));
        return item;
      }),
    );
  }
  for (const t of state.advanced) {
    const input = tunableInput(t.key);
    if (document.activeElement !== input) input.value = tunableText(t);
  }
}

/** What's in the Advanced fields, by key: an empty one is left out (its default). */
function advancedValues(): Record<string, number> {
  const values: Record<string, number> = {};
  for (const t of state.advanced) {
    const text = tunableInput(t.key).value.trim();
    if (!text) continue;
    const value = Number(text);
    if (!Number.isInteger(value) || value < 0) throw new Error(`${t.label} must be a whole number of seconds`);
    values[t.key] = value;
  }
  return values;
}

/** Whether AFK is on, as last shown: the button turns it the other way. */
let afkOn = false;

function renderAfk(afk: AfkStatus): void {
  afkOn = afk.on;
  el("afk").classList.toggle("on", afk.on);
  document.body.classList.toggle("afk", afk.on);
  el("afk-tag").hidden = !afk.on;
  el("afk-toggle").setAttribute("aria-checked", String(afk.on));
  const line = el("afk-state");
  const rounds = afk.latest?.rounds ?? [];
  const skipped = rounds.length ? ` Not rated for you so far: round${rounds.length === 1 ? "" : "s"} ${rounds.join(", ")} of the latest match.` : "";
  line.textContent = afk.on ? `On.${skipped}` : "Off: your rounds are rated.";
  line.className = afk.on ? "status bad" : "status muted";
  const error = el("afk-error");
  error.hidden = !afk.error;
  error.textContent = afk.error ?? "";
}

function showUploads(status: UploadStatus): void {
  // A poll that began before the host changed the server or folder: not about what's shown.
  // Dropped before it touches the history, so it can't replace the new server's page.
  if (status.serverUrl !== state.serverUrl || status.logFolder !== (state.logFolder?.path ?? null) || status.chosenRegion !== state.region) return;
  uploadStatus = status;
  if (status.host) showHost(status.host);
  else if (status.problem?.kind === "tokenRejected") showHost(null);
  if (status.region !== uploadRegion) {
    uploadRegion = status.region;
    tourneyContextChanged();
  }
  renderUploads(status);
  renderHome();
}

/** The last upload status for the settings shown, `null` before one. */
let uploadStatus: UploadStatus | null = null;

/** The region uploads go as, as the upload status last said (`null`: not known). */
let uploadRegion: string | null = null;

function describeLobbyProblem(problem: LobbyProblem): string {
  switch (problem.kind) {
    case "settings":
      return "Not listed until the settings file is fixed (see above).";
    case "noFolder":
      return "Not listed: waiting for the Workshop log folder.";
    case "folderUnreadable":
      return `Not listed: can't read the log folder: ${problem.message}`;
    case "noToken":
      return "Not listed: there's no host token for this server.";
    case "tokenRejected":
      return problem.revoked ? "Not listed: this token was revoked." : "Not listed: the server doesn't know this token.";
    case "noRegion":
      return "Not listed: you have no home region yet. Pick the region you host in under Settings → Game.";
    case "failed":
      return `Couldn't reach the lobby list, retrying: ${problem.message}`;
  }
}

/** The server's name as the host knows it (`genjiball.us`). */
function serverName(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function players(n: number): string {
  return n === 1 ? "1 player" : `${n} players`;
}

/** The live lobby's last status for the settings shown, `null` before one. */
let lobbyStatus: LobbyStatus | null = null;

/** The quiet time in use (Settings → Advanced → Timing), in seconds. */
function quietSecs(): number {
  const quiet = state.advanced.find((t) => t.key === "quietSecs");
  return quiet ? (quiet.value ?? quiet.default) : 0;
}

function renderLobby(status: LobbyStatus): void {
  // A poll that began before the host changed the server: not about what's shown.
  if (status.serverUrl !== state.serverUrl) return;
  lobbyStatus = status;
  renderHome();
  const line = el("lobby-state");
  const listed = status.listed;
  let text: string;
  let tone: Tone = "muted";
  if (listed) {
    const name = listed.name ? ` as "${listed.name}"` : "";
    text = `Listed on ${serverName(status.serverUrl)} in ${regionLabel(listed.region)}${name}, ${players(listed.players)}.`;
    tone = "good";
    if (!status.on || status.live.kind !== "playing") text += " Taking it off the list…";
  } else if (!status.on) {
    text = "Off: your lobby isn't listed.";
  } else if (status.problem) {
    text = describeLobbyProblem(status.problem);
    tone = "bad";
  } else if (status.live.kind === "playing") {
    text = `Ranked match with ${players(status.live.players)}: listing it…`;
  } else if (status.live.kind === "unranked") {
    text = "Not listed: this match is unranked, so it won't count.";
  } else {
    text = "Not listed: no ranked match is being played.";
  }
  // A failure while listed: still listed, but the site may be behind.
  if (listed && status.problem) {
    text += ` ${describeLobbyProblem(status.problem)}`;
    tone = "bad";
  }
  line.textContent = text;
  line.className = tone;
}

async function saveLobbyName(): Promise<void> {
  setLine("lobby-form-state", "", "good");
  const name = el<HTMLInputElement>("lobby-name");
  const next = await invoke<AppState>("set_live_lobby", { on: state.liveLobby, name: name.value });
  // Shown as saved: spaces around it dropped.
  name.value = next.lobbyName ?? "";
  await show(next);
  toast("Lobby name saved");
}

/** Shows these settings, then the upload status for them (one sent before they were known was dropped). */
async function show(next: AppState): Promise<void> {
  state = next;
  ready = true;
  render();
  const status = await invoke<UploadStatus>("get_upload_status");
  renderAfk(status.afk);
  showUploads(status);
  renderLobby(await invoke<LobbyStatus>("get_lobby_status"));
  renderTourneys(await invoke<TourneysStatus>("get_tourneys"));
  renderHome();
}

async function refresh(): Promise<void> {
  await show(await invoke<AppState>("get_state"));
}

/** The version whose notification the host closed with "Later". */
let updateLater: string | null = null;

function renderUpdate(status: UpdateStatus): void {
  el("update-banner").hidden = !status.available || status.available === updateLater;
  el("update-banner-text").textContent = status.available ? `Version ${status.available} is ready to install. You have v${state.version}.` : "";
  el("update-banner").dataset.version = status.available ?? "";
  const line = el("update-state");
  if (status.error) line.textContent = `Couldn't check for updates: ${status.error}`;
  else if (status.available) line.textContent = `Version ${status.available} is available`;
  else if (status.checkedAt) line.textContent = `Up to date (checked ${new Date(status.checkedAt).toLocaleTimeString()})`;
  else line.textContent = "Checking…";
}

/** Actions running now. The buttons come back only when the last one ends. */
let running = 0;

function setButtonsDisabled(disabled: boolean): void {
  // A button marked `data-off` (a tourney code before its window) stays disabled. The sidebar stays usable.
  document.querySelectorAll<HTMLButtonElement>("main button, #update-banner button").forEach((b) => (b.disabled = disabled || b.dataset.off === "true"));
}

/** Runs a button's action with the buttons disabled, and shows its error next to its setting. */
async function busy(action: () => Promise<void>, showError: (message: string) => void): Promise<void> {
  running += 1;
  setButtonsDisabled(true);
  try {
    await action();
  } catch (err) {
    // Commands fail with strings, the browser with `Error`s.
    showError(errorText(err));
  } finally {
    running -= 1;
    if (!running) setButtonsDisabled(false);
  }
}

async function checkSaved(): Promise<void> {
  setStatus("Checking…");
  showCheck(await invoke<TokenCheck>("check_saved_token"));
}

const showTokenError = (m: string) => setStatus(m, "bad");

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
  }, showTokenError);
});

el("token-cancel").addEventListener("click", () => {
  editingToken = false;
  el<HTMLInputElement>("token").value = "";
  render();
});
el("token-check").addEventListener("click", () => void busy(checkSaved, showTokenError));
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
  }, showTokenError);
});

el("server-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void busy(async () => {
    setLine("server-state", "", "muted");
    const next = await invoke<AppState>("set_server_url", { url: el<HTMLInputElement>("server-url").value });
    // A code kept for the old server's tags is no use now.
    uncopied = null;
    showHost(null);
    setStatus("");
    await show(next);
    // Another server's history, from its newest page: drops any page of the old one on its way.
    await showHistoryPage(0);
    if (state.hasToken) await checkSaved();
  }, (m) => setLine("server-state", m, "bad"));
});

/** Counts region changes: the answer to one a later change replaced is dropped. */
let regionChange = 0;

el("region").addEventListener("change", () => {
  void busy(
    async () => {
      setRegionStatus("");
      const change = ++regionChange;
      const region = el<HTMLSelectElement>("region").value || null;
      const next = await invoke<AppState>("set_region", { region });
      if (change !== regionChange) return;
      // A code kept for the old region's tags is no use now.
      uncopied = null;
      await show(next);
      toast(`Region saved: ${next.region ? regionLabel(next.region) : "your home region"}`);
    },
    (m) => {
      setRegionStatus(m, "bad");
      renderRegion();
    },
  );
});

async function saveAdvanced(values: Record<string, number>): Promise<void> {
  setLine("advanced-state", "", "good");
  const next = await invoke<AppState>("set_advanced", { values });
  // Shown as saved, not as typed: a value at its default empties its field.
  for (const t of next.advanced) tunableInput(t.key).value = tunableText(t);
  await show(next);
  toast("Timing saved");
}

el("advanced-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void busy(() => saveAdvanced(advancedValues()), (m) => setLine("advanced-state", m, "bad"));
});
el("advanced-reset").addEventListener("click", () => {
  void busy(() => saveAdvanced({}), (m) => setLine("advanced-state", m, "bad"));
});

el("release-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void busy(
    async () => {
      setLine("release-state", "", "muted");
      const next = await invoke<AppState>("set_release_tag", { tag: el<HTMLInputElement>("release-tag").value });
      el<HTMLInputElement>("release-tag").value = next.releaseTag ?? "";
      // A code kept from the other release is no use now.
      uncopied = null;
      await show(next);
      toast("Release saved", next.releaseTag ? `The ranked code is built on ${next.releaseTag}.` : "The ranked code is built on the latest ranked release.");
    },
    (m) => setLine("release-state", m, "bad"),
  );
});

el("log-level").addEventListener("change", () => {
  void busy(
    async () => {
      setLine("debug-state", "", "muted");
      const level = el<HTMLSelectElement>("log-level").value;
      await show(await invoke<AppState>("set_log_level", { level: level === state.defaultLogLevel ? null : level }));
      toast(`Logging at ${level} from now on`);
    },
    (m) => {
      setLine("debug-state", m, "bad");
      renderDebug();
    },
  );
});

el("dry-run").addEventListener("change", () => {
  void busy(
    async () => {
      setLine("debug-state", "", "muted");
      const on = el<HTMLInputElement>("dry-run").checked;
      await show(await invoke<AppState>("set_dry_run", { on }));
      toast(on ? "Dry run on" : "Dry run off", on ? "Nothing is uploaded." : "Ranked logs are uploaded again.");
    },
    (m) => {
      setLine("debug-state", m, "bad");
      renderDebug();
    },
  );
});

const matchViewPollSecs = () => state.matchViewPollSecs;

/** The debug panel, reading while it's open. */
let debugWatch: { stop(): void } | null = null;

function watchDebugPanel(open: boolean): void {
  debugWatch?.stop();
  debugWatch = open ? watchDebug(matchViewPollSecs) : null;
}

el("debug-panel").addEventListener("toggle", () => watchDebugPanel(el<HTMLDetailsElement>("debug-panel").open));
function saveUpdateSettings(): void {
  void busy(
    async () => {
      setLine("updates-state", "", "muted");
      const autoCheck = el<HTMLInputElement>("update-auto").checked;
      const channel = el<HTMLSelectElement>("update-channel").value;
      await show(await invoke<AppState>("set_updates", { autoCheck, channel }));
      toast("Update settings saved");
    },
    (m) => {
      setLine("updates-state", m, "bad");
      renderUpdateSettings();
    },
  );
}

el("update-auto").addEventListener("change", saveUpdateSettings);
el("update-channel").addEventListener("change", saveUpdateSettings);

el("log-open").addEventListener("click", () => void busy(() => invoke<void>("open_log_folder"), (m) => setLine("debug-state", m, "bad")));

el("diagnostics-export").addEventListener("click", () => {
  void busy(
    async () => {
      setLine("debug-state", "", "muted");
      const path = await invoke<string | null>("export_diagnostics");
      if (path) toast("Diagnostics saved", `${path}. Attach it to your bug report.`);
    },
    (m) => setLine("debug-state", m, "bad"),
  );
});

/** Saves the log folder (`null`: the detected one); its error shows under it. */
async function saveLogFolder(path: string | null): Promise<void> {
  const next = await invoke<AppState>("set_log_folder", { path });
  logFolderChanged();
  await show(next);
}

function showLogFolderError(message: string): void {
  setLine("log-folder-state", message, "bad");
}

el("log-folder-choose").addEventListener("click", () => {
  setLine("log-folder-state", "", "muted");
  void busy(async () => {
    const path = await open({ directory: true, defaultPath: state.logFolder?.path, title: "Workshop log folder" });
    if (typeof path === "string") await saveLogFolder(path);
  }, showLogFolderError);
});
el("log-folder-reset").addEventListener("click", () => {
  setLine("log-folder-state", "", "muted");
  void busy(() => saveLogFolder(null), showLogFolderError);
});

function showScreenshotFolderError(message: string): void {
  setLine("screenshot-folder-state", message, "bad");
}

el("screenshot-folder-choose").addEventListener("click", () => {
  setLine("screenshot-folder-state", "", "muted");
  void busy(async () => {
    const path = await open({ directory: true, defaultPath: state.screenshotFolder?.path, title: "Screenshots folder" });
    if (typeof path === "string") await show(await invoke<AppState>("set_screenshot_folder", { path }));
  }, showScreenshotFolderError);
});
el("screenshot-folder-reset").addEventListener("click", () => {
  setLine("screenshot-folder-state", "", "muted");
  void busy(async () => show(await invoke<AppState>("set_screenshot_folder", { path: null })), showScreenshotFolderError);
});

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
  if (!kept || kept.built.serverUrl !== state.serverUrl || kept.built.chosenRegion !== state.region) return null;
  // The home region may have changed since (a new token, or an admin).
  if (kept.built.requestedRegion !== (state.region ?? homeRegion ?? null)) return null;
  return Date.now() - kept.at < kept.built.keepSecs * 1000 ? kept : null;
}

/** How long "Copy ranked code" says "Copied" after a copy, in milliseconds. */
const COPIED_MS = 1600;
let copiedTimer: ReturnType<typeof setTimeout> | undefined;

/** The ranked code button's look: at rest, building (a sweep runs across it), or just copied. */
function setCodeButton(mode: "idle" | "building" | "copied"): void {
  clearTimeout(copiedTimer);
  const copy = el("ranked-code-copy");
  copy.dataset.mode = mode;
  copy.textContent = mode === "building" ? "Building the code…" : mode === "copied" ? "Copied ✓" : "Copy ranked code";
  if (mode === "copied") copiedTimer = setTimeout(() => setCodeButton("idle"), COPIED_MS);
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
    setLine("ranked-code-state", "", "muted");
    setCodeButton("building");
    built = await invoke<RankedCode>("build_ranked_code");
    builtAt = Date.now();
    // A newer click, or a server change, while this one ran: its tags may be from the wrong server.
    if (build !== rankedBuild) return;
    if (built.serverUrl !== state.serverUrl) {
      setLine("ranked-code-state", "The server changed while the code was built. Click again for this server's.", "bad");
      return;
    }
    if (built.chosenRegion !== state.region) {
      setLine("ranked-code-state", "The region changed while the code was built. Click again for this region's.", "bad");
      return;
    }
  }
  await copyBuilt(built.code, () => (uncopied = { built, at: builtAt }));
  const region = built.region ? `${regionLabel(built.region)} ` : "";
  const tiers = built.names === 1 ? "1 more with their rank" : `${built.names} more with their rank`;
  const skipped = built.skippedNames ? ` (${built.skippedNames} left out: the Workshop can't show their names)` : "";
  setLine("ranked-code-state", "", "muted");
  setCodeButton("copied");
  toast("Ranked code copied", `Genji Ball ${built.release}: ${region}top ${built.top} tagged with place and rating, ${tiers}, from ${new Date(built.tagsUpdatedAt).toLocaleString()}${skipped}.${built.dataCenter ? ` Puts the lobby on ${built.dataCenter}.` : ""}`);
}

el("ranked-code-copy").addEventListener("click", () =>
  void busy(copyRankedCode, (m) => {
    setLine("ranked-code-state", m, "bad");
    // Copied with Ctrl+Shift+C from another view: the line on Home isn't in sight.
    if (currentView() !== "home") toast("Couldn't copy the ranked code", m);
  }).then(() => {
    // Failed, or a newer click took over: back at rest.
    if (el("ranked-code-copy").dataset.mode === "building") setCodeButton("idle");
  }),
);

function showUpdateError(message: string): void {
  el("update-state").textContent = message;
}

el("update-check").addEventListener("click", () => void busy(async () => renderUpdate(await invoke<UpdateStatus>("check_for_update")), showUpdateError));
el("update-install").addEventListener("click", () => void busy(() => invoke("install_update"), showUpdateError));
el("update-later").addEventListener("click", () => {
  updateLater = el("update-banner").dataset.version ?? null;
  el("update-banner").hidden = true;
});

el("afk-toggle").addEventListener("click", () => {
  void busy(
    async () => renderAfk(await invoke<AfkStatus>("set_afk", { on: !afkOn })),
    (m) => {
      const error = el("afk-error");
      error.hidden = false;
      error.textContent = m;
    },
  );
});

el("lobby-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void busy(saveLobbyName, (m) => setLine("lobby-form-state", m, "bad"));
});
el("lobby-on").addEventListener("change", () => {
  void busy(
    async () => {
      setLine("lobby-error", "", "bad");
      await show(await invoke<AppState>("set_live_lobby", { on: el<HTMLInputElement>("lobby-on").checked, name: state.lobbyName ?? "" }));
    },
    (m) => {
      setLine("lobby-error", m, "bad");
      renderLobbySettings();
    },
  );
});

/** The current match view, reading the live log while the Match view is shown. */
let liveWatch: LogWatch | null = null;

function watchLiveMatch(shown: boolean): void {
  if (shown && !liveWatch) liveWatch = watchLog(el("live-match"), { kind: "live" }, matchViewPollSecs);
  if (!shown && liveWatch) {
    liveWatch.stop();
    liveWatch = null;
  }
}

/** Another log folder: the match views read their logs from it afresh. */
function logFolderChanged(): void {
  homeFolderChanged();
  closeMatchViews();
  if (liveWatch) {
    liveWatch.stop();
    liveWatch = null;
    watchLiveMatch(true);
  }
  if (debugWatch) watchDebugPanel(true);
}

/** What Home shows, from the state above. */
function homeState() {
  const entry = uploadStatus?.history.page === 0 ? uploadStatus.history.entries[0] : undefined;
  let lastUpload = null;
  if (entry) {
    const now = entry.queued ? describeQueued(entry.queued) : entry.answer ? describeAnswer(entry.answer) : { text: "", tone: "muted" as Tone };
    lastUpload = { file: entry.file, when: entry.at ? new Date(entry.at).toLocaleString() : "Not uploaded yet", ...now };
  }
  const region = uploadStatus?.region ?? state.region ?? homeRegion ?? null;
  return {
    hasToken: state.hasToken,
    logFolder: state.logFolder,
    dryRun: state.dryRun,
    settingsError: Boolean(state.settingsError),
    upload: uploadStatus,
    tokenCheck: tokenCheck?.result ?? null,
    unreachable: tokenCheck?.result === "unreachable" ? tokenCheck.message : null,
    screenshotsDue: screenshotsDue(),
    serverName: serverName(state.serverUrl),
    host: knownHost && { name: knownHost.name, untrusted: knownHost.trust === "untrusted" },
    token: tokenLine,
    region: !region ? "None yet" : `${state.region ? regionLabel(region) : `${regionLabel(region)} (home region)`}${dataCenterNote(region)}`,
    lastUpload,
    pollSecs: state.matchViewPollSecs,
    lobbyLive: lobbyStatus?.on ? lobbyStatus.live.kind : null,
    quietSecs: quietSecs(),
  };
}

/** The data center a region's codes put the lobby on, for Home's region line. */
function dataCenterNote(region: string): string {
  const chosen = state.dataCenters.find((d) => d.region === region)?.chosen;
  return !chosen ? "" : chosen === state.bestAvailable ? ", best data center" : `, lobby on ${chosen}`;
}

/** A Home problem's button. */
function homeFix(target: Fix): void {
  switch (target.kind) {
    case "pane":
      showPane(target.pane);
      break;
    case "view":
      showView(target.view);
      break;
    case "changeToken":
      editingToken = true;
      render();
      showPane("account");
      el("token").focus();
      break;
    case "checkToken":
      void busy(checkSaved, showTokenError);
      break;
    case "dryRunOff":
      void busy(
        async () => {
          setLine("debug-state", "", "muted");
          await show(await invoke<AppState>("set_dry_run", { on: false }));
        },
        // Under the switch in Settings, and in a toast: Home has no line for it.
        (m) => {
          setLine("debug-state", m, "bad");
          toast("Couldn't turn the dry run off", m);
        },
      );
      break;
  }
}

el("home-region-change").addEventListener("click", () => showPane("game"));
el("home-uploads").addEventListener("click", () => showView("uploads"));
el("setup-token-add").addEventListener("click", () => {
  showPane("account");
  el("token").focus();
});
el("setup-folder-choose").addEventListener("click", () => showPane("game"));

setupDesktop({
  showView,
  copyRankedCode: () => {
    const copy = el<HTMLButtonElement>("ranked-code-copy");
    if (state?.hasToken && !copy.disabled) copy.click();
  },
});

// The live log is read once the settings (its poll time) are known.
onViewChange((view) => {
  if (ready) watchLiveMatch(view === "match");
});
setupViews();

void setupTitlebar();

/** The window couldn't start (its settings or events): said above every view. */
function showStartError(message: string): void {
  const banner = el("settings-error");
  banner.hidden = false;
  banner.textContent = `The window couldn't start: ${message}`;
}

void busy(async () => {
  await listen<UploadStatus>("upload-status", (event) => {
    if (!ready) return; // `show` asks for the status once the settings are known.
    // AFK is the same whatever the settings, so even a status for others shows it.
    renderAfk(event.payload.afk);
    // The settings file was fixed by hand (the uploader reads it again): show what's in it now.
    if (state.settingsError && event.payload.problem?.kind !== "settings") void refresh();
    else showUploads(event.payload);
  });
  await listen<LobbyStatus>("lobby-status", (event) => {
    if (ready) renderLobby(event.payload);
  });
  await listen<TourneysStatus>("tourneys-status", (event) => {
    if (!ready) return;
    renderTourneys(event.payload);
    renderHome();
  });
  await setupTourneys(() => ({
    serverUrl: state.serverUrl,
    uploadRegion,
    screenshotFolder: state.screenshotFolder?.exists ? state.screenshotFolder.path : null,
    screenshotMaxBytes: state.screenshotMaxBytes,
    screenshotPollSecs: state.screenshotPollSecs,
    regionLabel,
    busy,
    isBusy: () => running > 0,
  }));
  setupUploads(() => ({ serverUrl: state.serverUrl, matchViewPollSecs: state.matchViewPollSecs, regionLabel, busy }));
  setupOverlaySettings({ busy, show: (next) => show(next as AppState) });
  // Edit mode ended from the overlay, or with its hotkey.
  await listen("overlay-changed", () => {
    if (ready) void refresh();
  });
  await listen<UpdateStatus>("update-status", (event) => {
    if (ready) renderUpdate(event.payload);
  });
  await refresh();
  watchLiveMatch(currentView() === "match");
  setupHome(homeState, homeFix);
  renderUpdate(await invoke<UpdateStatus>("get_update_status"));
  if (state.hasToken) await busy(checkSaved, showTokenError);
  // First start: Home shows the setup.
  else showView("home");
}, showStartError);
