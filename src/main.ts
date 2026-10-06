import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { watchDebug } from "./debug-view";
import { setupDesktop } from "./desktop";
import { watchLog, type LogWatch } from "./match-view";
import type { TourneysStatus } from "./tourney-model";
import type { Fix } from "./home-model";
import { flash, homeFolderChanged, renderHome, setupHome } from "./home-view";
import { renderTourneys, screenshotsDue, setupTourneys, tourneyContextChanged, tourneyLabel } from "./tourneys-view";
import { setupTitlebar } from "./titlebar";
import { toast } from "./toast";
import { currentView, markView, onViewChange, setupViews, showPane, showView } from "./views";

/** Mirrors `AppState` in src-tauri/src/lib.rs. */
interface AppState {
  version: string;
  serverUrl: string;
  defaultServerUrl: string;
  hasToken: boolean;
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
  defaultUpdateChannel: string;
  /** Per region, the data centers its codes can put the lobby on (the default first), and the one in use. */
  dataCenters: DataCenterChoice[];
  /** The choice that leaves the data center to the game. */
  bestAvailable: string;
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

/** Mirrors `Host` in src-tauri/src/server.rs. */
interface Host {
  id: number;
  name: string;
  trust: "trusted" | "untrusted";
  /** The home region, `null` for none. */
  region: string | null;
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
  chosenRegion: string | null;
  /** What uploads go as: the region picked, else the home region. `null` while not known. */
  region: string | null;
  problem: Problem | null;
  waiting: number;
  retrying: string | null;
  history: HistoryPage;
  historyRevision: number;
  host: Host | null;
  afk: AfkStatus;
  dryRun: boolean;
}

/** Mirrors `AfkStatus` in src-tauri/src/afk.rs: what `set_afk` returns. */
interface AfkStatus {
  on: boolean;
  /** The match a round was last skipped in for AFK, and those rounds. */
  latest: { matchKey: string; rounds: number[] } | null;
  /** Why `afk.json` couldn't be read or written. */
  error: string | null;
}

/** Mirrors `Problem` in src-tauri/src/uploader.rs. */
type Problem =
  | { kind: "settings" }
  | { kind: "noFolder" }
  | { kind: "folderUnreadable"; message: string }
  | { kind: "noToken" }
  | { kind: "tokenRejected"; revoked: boolean }
  | { kind: "noRegion" }
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
  /** Its tourney matches (none: only ranked ones). Mirrors `SentTourney` in src-tauri/src/uploads.rs. */
  tourneys: { lobbyKey: string; ended: boolean }[];
}

/** Mirrors `Answer` in src-tauri/src/uploads.rs. */
type Answer = { kind: "answered"; result: "stored" | "unchanged" | "duplicate"; region: string | null; matches: UploadedMatch[] } | { kind: "refused"; error: string; message: string };

/** Mirrors `QueueState` in src-tauri/src/watcher.rs. */
type QueueState = { kind: "playing" } | { kind: "due" } | { kind: "failed"; error: string };

/** Mirrors `UploadedMatch` in src-tauri/src/server.rs. */
interface UploadedMatch {
  matchKey: string | null;
  matchId: number | null;
  lineCount: number;
  region: string | null;
  action: "insert" | "replace" | "repoint" | "skip";
  status: "accepted" | "review" | "rejected" | "void";
  rejection: { code: string; message: string } | null;
  reviewReasons: string[];
}

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

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found as T;
}

let state: AppState;
/** Set once `state` is: an upload status is only shown for the settings in it. */
let ready = false;
let editingToken = false;

/** The token's state as last said, for Home too. */
let tokenLine: { text: string; tone: "good" | "bad" | "muted" } = { text: "", tone: "muted" };
/** The last token check's result, and why it couldn't reach the server. */
let tokenCheck: TokenCheck | null = null;

function setStatus(text: string, tone: "good" | "bad" | "muted" = "muted"): void {
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
    select.replaceChildren(
      ...options.map((r) => {
        const option = document.createElement("option");
        option.value = r.id;
        option.textContent = r.label;
        return option;
      }),
    );
  }
  select.value = state.region ?? "";
}

/** One data center choice per region, built once; each follows `state`. */
function renderDataCenters(): void {
  const box = el("data-centers");
  if (!box.childElementCount) {
    box.replaceChildren(
      ...state.dataCenters.map((d) => {
        const label = document.createElement("label");
        label.htmlFor = `data-center-${d.region}`;
        label.textContent = regionLabel(d.region);
        const select = document.createElement("select");
        select.id = `data-center-${d.region}`;
        select.append(
          ...[...d.names, state.bestAvailable].map((name, i) => {
            const option = document.createElement("option");
            option.value = name;
            option.textContent = name === state.bestAvailable ? "Best available (the game picks)" : i === 0 ? `${name} (default)` : name;
            return option;
          }),
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
        const row = document.createElement("div");
        row.className = "row";
        row.append(label, select);
        return row;
      }),
    );
  }
  for (const d of state.dataCenters) el<HTMLSelectElement>(`data-center-${d.region}`).value = d.chosen;
}

function setRegionStatus(text: string, tone: "good" | "bad" | "muted" = "muted"): void {
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

  const shots = state.screenshotFolder;
  el("screenshot-folder").textContent = shots?.path ?? "No Documents folder found";
  el("screenshot-folder-note").textContent = !shots
    ? "Choose the folder Overwatch saves screenshots to."
    : !shots.exists
      ? "This folder isn't there yet. Overwatch creates it with your first screenshot; if yours go elsewhere, choose that folder."
      : shots.source === "detected"
        ? "Found automatically."
        : "Chosen by you.";
  el("screenshot-folder-reset").hidden = shots?.source !== "custom";
  if (shots?.source === "custom" || (shots && !shots.exists)) el<HTMLDetailsElement>("screenshot-settings").open = true;
  tourneyContextChanged();

  renderRegion();
  renderDataCenters();
  renderLobbySettings();
  renderAdvanced();
  renderUpdateSettings();
  renderDebug();
  renderHome();
}

const updateChannelLabels: Record<string, string> = { stable: "Stable", prerelease: "Pre-release" };

/** The update checks' switch and channel (Settings → Updates). */
function renderUpdateSettings(): void {
  el<HTMLInputElement>("update-auto").checked = state.autoUpdateCheck;
  const select = el<HTMLSelectElement>("update-channel");
  if (select.options.length !== state.updateChannels.length) {
    select.replaceChildren(
      ...state.updateChannels.map((channel) => {
        const option = document.createElement("option");
        option.value = channel;
        option.textContent = updateChannelLabels[channel] ?? channel;
        return option;
      }),
    );
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
    select.replaceChildren(
      ...state.logLevels.map((level) => {
        const option = document.createElement("option");
        option.value = level;
        option.textContent = level === state.defaultLogLevel ? `${level} (default)` : level;
        return option;
      }),
    );
  }
  select.value = state.logLevel ?? state.defaultLogLevel;
  el<HTMLInputElement>("dry-run").checked = state.dryRun;
}

function setLine(id: string, text: string, tone: "good" | "bad" | "muted"): void {
  const line = el(id);
  line.hidden = !text;
  line.textContent = text;
  line.className = tone;
}

/** The live lobby's switch and name, as saved, unless the host is editing the name. */
function renderLobbySettings(): void {
  el<HTMLInputElement>("lobby-on").checked = state.liveLobby;
  const name = el<HTMLInputElement>("lobby-name");
  name.maxLength = state.lobbyNameMax;
  if (document.activeElement !== name) name.value = state.lobbyName ?? "";
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
        const item = document.createElement("div");
        item.className = "tunable";
        const label = document.createElement("label");
        label.htmlFor = `tunable-${t.key}`;
        label.textContent = t.label;
        const input = document.createElement("input");
        input.id = `tunable-${t.key}`;
        input.type = "number";
        input.min = String(t.min);
        input.max = String(t.max);
        input.step = "1";
        input.placeholder = String(t.default);
        const unit = document.createElement("span");
        unit.className = "muted";
        unit.textContent = `seconds (default ${t.default})`;
        const row = document.createElement("div");
        row.className = "row";
        row.append(input, unit);
        const help = document.createElement("p");
        help.className = "note";
        help.textContent = t.help;
        item.append(label, row, help);
        return item;
      }),
    );
  }
  for (const t of state.advanced) {
    const input = tunableInput(t.key);
    if (document.activeElement !== input) input.value = t.value === null ? "" : String(t.value);
  }
}

function setAdvancedState(text: string, tone: "good" | "bad"): void {
  const line = el("advanced-state");
  line.hidden = !text;
  line.textContent = text;
  line.className = tone;
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
    case "noRegion":
      return "Paused: you have no home region yet. Pick the region you host in under Settings → Game.";
    case "local":
      return problem.message;
  }
}

/** Why a match waits for an admin. An untrusted host's every match does: the host line says so. */
const reviewReasons: Record<string, string | null> = {
  duplicate_name: "two players with the same name",
  untrusted_host: null,
  // A tourney match the server couldn't link to its lobby (genjiball-ranked docs/api.md, "Tourney matches").
  tourney_unknown_lobby: "the tourney lobby isn't known",
  tourney_cancelled: "the tourney was cancelled",
  tourney_wrong_host: "you aren't the lobby's host",
  tourney_wrong_region: "uploaded as another region than the tourney's",
  tourney_lobby_taken: "the lobby already has a match",
  tourney_round_limit: "the round limit isn't the lobby's",
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

function renderUploads(status: UploadStatus): void {
  // A poll that began before the host changed the server or folder: not about what's shown.
  // Dropped before it touches the history, so it can't replace the new server's page.
  if (status.serverUrl !== state.serverUrl || status.logFolder !== (state.logFolder?.path ?? null) || status.chosenRegion !== state.region) return;
  uploadStatus = status;
  if (status.host) showHost(status.host);
  else if (status.problem?.kind === "tokenRejected") showHost(null);
  renderUploadRegion(status);
  if (status.region !== uploadRegion) {
    uploadRegion = status.region;
    tourneyContextChanged();
  }
  const line = el("upload-state");
  line.className = status.problem ? "bad" : "";
  line.textContent = status.problem
    ? describeProblem(status.problem)
    : status.dryRun
      ? "Dry run: ranked logs are picked as usual but not uploaded. The debug panel (Settings → Advanced) lists them."
      : status.waiting === 1
      ? "Watching. 1 ranked log to upload."
      : status.waiting
        ? `Watching. ${status.waiting} ranked logs to upload.`
        : "Watching for ranked matches.";

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
  renderHome();
}

/** The last upload status for the settings shown, `null` before one. */
let uploadStatus: UploadStatus | null = null;

/** The region uploads go as, as the upload status last said (`null`: not known). */
let uploadRegion: string | null = null;

/** Which region matches upload as, so a night in the other one isn't stored as this one. */
function renderUploadRegion(status: UploadStatus): void {
  const line = el("upload-region");
  line.hidden = status.problem?.kind === "noRegion";
  if (!status.region) {
    line.textContent = "Uploading as your home region.";
    return;
  }
  const strong = document.createElement("strong");
  strong.textContent = regionLabel(status.region);
  line.replaceChildren("Uploading as ", strong, status.chosenRegion ? "." : " (your home region).");
}

/** The regions an upload's matches are stored in, as their ids in capitals (`EU`). */
function answerRegions(answer: Answer): string {
  if (answer.kind !== "answered") return "";
  const regions = new Set(answer.matches.map((m) => m.region).filter((r): r is string => r !== null));
  if (!regions.size && answer.region) regions.add(answer.region);
  return [...regions].map((r) => r.toUpperCase()).join(", ");
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

const matchViewPollSecs = () => state.matchViewPollSecs;

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
      shownMatches.set(entry.file, { box, watch: watchLog(box, { kind: "file", file: entry.file }, matchViewPollSecs) });
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
    if (seenUploads?.server === state.serverUrl) {
      for (const key of keys) if (!seenUploads.keys.has(key)) freshSince.set(key, now);
    } else freshSince.clear();
    seenUploads = { server: state.serverUrl, keys: new Set(keys) };
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

function setLobbyFormState(text: string, tone: "good" | "bad"): void {
  const line = el("lobby-form-state");
  line.hidden = !text;
  line.textContent = text;
  line.className = tone;
}

async function saveLobbyName(): Promise<void> {
  setLobbyFormState("", "good");
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
  renderUploads(status);
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
  setAdvancedState("", "good");
  const next = await invoke<AppState>("set_advanced", { values });
  // Shown as saved, not as typed: a value at its default empties its field.
  for (const t of next.advanced) tunableInput(t.key).value = t.value === null ? "" : String(t.value);
  await show(next);
  toast("Timing saved");
}

el("advanced-form").addEventListener("submit", (e) => {
  e.preventDefault();
  void busy(() => saveAdvanced(advancedValues()), (m) => setAdvancedState(m, "bad"));
});
el("advanced-reset").addEventListener("click", () => {
  void busy(() => saveAdvanced({}), (m) => setAdvancedState(m, "bad"));
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

el("log-folder-choose").addEventListener("click", () => {
  void busy(async () => {
    const path = await open({ directory: true, defaultPath: state.logFolder?.path, title: "Workshop log folder" });
    if (typeof path !== "string") return;
    const next = await invoke<AppState>("set_log_folder", { path });
    logFolderChanged();
    await show(next);
  });
});
el("log-folder-reset").addEventListener("click", () => {
  void busy(async () => {
    const next = await invoke<AppState>("set_log_folder", { path: null });
    logFolderChanged();
    await show(next);
  });
});

el("screenshot-folder-choose").addEventListener("click", () => {
  void busy(async () => {
    const path = await open({ directory: true, defaultPath: state.screenshotFolder?.path, title: "Screenshots folder" });
    if (typeof path !== "string") return;
    await show(await invoke<AppState>("set_screenshot_folder", { path }));
  });
});
el("screenshot-folder-reset").addEventListener("click", () => {
  void busy(async () => show(await invoke<AppState>("set_screenshot_folder", { path: null })));
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
    setRankedCodeState("", "muted");
    setCodeButton("building");
    built = await invoke<RankedCode>("build_ranked_code");
    builtAt = Date.now();
    // A newer click, or a server change, while this one ran: its tags may be from the wrong server.
    if (build !== rankedBuild) return;
    if (built.serverUrl !== state.serverUrl) {
      setRankedCodeState("The server changed while the code was built. Click again for this server's.", "bad");
      return;
    }
    if (built.chosenRegion !== state.region) {
      setRankedCodeState("The region changed while the code was built. Click again for this region's.", "bad");
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
  const region = built.region ? `${regionLabel(built.region)} ` : "";
  const tiers = built.names === 1 ? "1 more with their rank" : `${built.names} more with their rank`;
  const skipped = built.skippedNames ? ` (${built.skippedNames} left out: the Workshop can't show their names)` : "";
  setRankedCodeState("", "muted");
  setCodeButton("copied");
  toast("Ranked code copied", `Genji Ball ${built.release}: ${region}top ${built.top} tagged with place and rating, ${tiers}, from ${new Date(built.tagsUpdatedAt).toLocaleString()}${skipped}.${built.dataCenter ? ` Puts the lobby on ${built.dataCenter}.` : ""}`);
}

el("ranked-code-copy").addEventListener("click", () =>
  void busy(copyRankedCode, (m) => {
    setRankedCodeState(m, "bad");
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
  void busy(saveLobbyName, (m) => setLobbyFormState(m, "bad"));
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
  for (const shown of shownMatches.values()) shown.watch.stop();
  shownMatches.clear();
  if (liveWatch) {
    liveWatch.stop();
    liveWatch = null;
    watchLiveMatch(true);
  }
  if (debugWatch) watchDebugPanel(true);
}

el("uploads-newer").addEventListener("click", () => void busy(() => showHistoryPage(historyPage - 1), showUploadsError));
el("uploads-older").addEventListener("click", () => void busy(() => showHistoryPage(historyPage + 1), showUploadsError));

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
      void busy(checkSaved);
      break;
    case "dryRunOff":
      void busy(async () => show(await invoke<AppState>("set_dry_run", { on: false })));
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

void busy(async () => {
  await listen<UploadStatus>("upload-status", (event) => {
    if (!ready) return; // `show` asks for the status once the settings are known.
    // AFK is the same whatever the settings, so even a status for others shows it.
    renderAfk(event.payload.afk);
    // The settings file was fixed by hand (the uploader reads it again): show what's in it now.
    if (state.settingsError && event.payload.problem?.kind !== "settings") void refresh();
    else renderUploads(event.payload);
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
  await listen<UpdateStatus>("update-status", (event) => {
    if (ready) renderUpdate(event.payload);
  });
  await refresh();
  watchLiveMatch(currentView() === "match");
  setupHome(homeState, homeFix);
  renderUpdate(await invoke<UpdateStatus>("get_update_status"));
  if (state.hasToken) await checkSaved();
  // First start: Home shows the setup.
  else showView("home");
});
