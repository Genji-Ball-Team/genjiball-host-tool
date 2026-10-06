import { invoke } from "@tauri-apps/api/core";
import { watchLog, type LogWatch } from "./match-view";

/**
 * The debug panel (Advanced → Debug panel): the files waiting to upload, the latest uploads and dry
 * runs with what was sent and what the server answered, and the newest log's last events. Read
 * again while the panel is open. Log lines and answers are untrusted: they go in with `textContent`.
 */

/** Mirrors `DebugInfo` in src-tauri/src/lib.rs: what `get_debug` returns. */
interface DebugInfo {
  queue: Queued[];
  uploads: Attempt[];
  /** Mirrors `LiveEvents` in src-tauri/src/debug.rs. `null` while there's no log. */
  events: { file: string; lines: string[] } | null;
  eventsError: string | null;
}

/** Mirrors `Queued` in src-tauri/src/watcher.rs. */
interface Queued {
  file: string;
  players: string[];
  state: { kind: "playing" } | { kind: "due" } | { kind: "failed"; error: string };
}

/** Mirrors `Attempt` in src-tauri/src/debug.rs. */
interface Attempt {
  at: string;
  serverUrl: string;
  file: string;
  bytes: number;
  /** Sent besides the token, as `[name, value]`. */
  headers: [string, string][];
  matchKeys: string[];
  outcome: Outcome;
}

/** Mirrors `Outcome` in src-tauri/src/debug.rs. */
type Outcome = { kind: "dryRun" } | { kind: "tooLarge" } | { kind: "answered"; answer: { status: number; body: string } } | { kind: "unanswered"; message: string };

type Tone = "good" | "bad" | "muted";

/**
 * A log line split for the panel: `[00:01:02] ROUND_START|2.38|1|...` gives its time, event and
 * fields. A line that isn't an event (another inspector line) has no event, only its text.
 */
export function eventParts(line: string): { time: string; event: string | null; fields: string[] } {
  const prefixed = /^\[(\d+:\d{2}:\d{2})\] ?(.*)$/.exec(line);
  const time = prefixed?.[1] ?? "";
  const rest = (prefixed?.[2] ?? line).trimEnd();
  const [first = "", ...fields] = rest.split("|");
  return /^[A-Z][A-Z_]*$/.test(first) ? { time, event: first, fields } : { time, event: null, fields: [rest] };
}

export function describeOutcome(outcome: Outcome): { text: string; tone: Tone } {
  switch (outcome.kind) {
    case "dryRun":
      return { text: "Dry run: not sent", tone: "muted" };
    case "tooLarge":
      return { text: "Not sent: over the server's size limit", tone: "bad" };
    case "answered": {
      const { status } = outcome.answer;
      return { text: `The server answered ${status}`, tone: status >= 200 && status < 300 ? "good" : "bad" };
    }
    case "unanswered":
      return { text: `No answer: ${outcome.message}`, tone: "bad" };
  }
}

/** An answer's body, indented if it's JSON. */
export function prettyBody(body: string): string {
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}

function el(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found;
}

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text: string, className = ""): HTMLElementTagNameMap[K] {
  const found = document.createElement(tag);
  found.textContent = text;
  if (className) found.className = className;
  return found;
}

function describeQueued(queued: Queued): string {
  switch (queued.state.kind) {
    case "playing":
      return "being played";
    case "due":
      return "due";
    case "failed":
      return `failed: ${queued.state.error}`;
  }
}

/** The match views open in the panel, by upload: kept while the panel is drawn again. */
const shownMatches = new Map<string, { box: HTMLElement; watch: LogWatch }>();

const attemptKey = (attempt: Attempt) => `${attempt.at}|${attempt.serverUrl}|${attempt.file}`;

function attemptItem(attempt: Attempt, pollSecs: () => number): HTMLLIElement {
  const item = document.createElement("li");
  const kb = (attempt.bytes / 1024).toFixed(1);
  item.append(node("span", attempt.file, "path"), node("span", `${new Date(attempt.at).toLocaleTimeString()} · ${kb} KB`, "muted"));
  const outcome = describeOutcome(attempt.outcome);
  item.append(node("span", outcome.text, outcome.tone));
  if (attempt.matchKeys.length) item.append(node("span", `Match keys: ${attempt.matchKeys.join(", ")}`, "soft"));
  const sent = [`POST ${attempt.serverUrl}/api/upload`, ...attempt.headers.map(([name, value]) => `${name}: ${value}`)];
  item.append(node("pre", sent.join("\n"), "raw"));
  if (attempt.outcome.kind === "answered" && attempt.outcome.answer.body) item.append(node("pre", prettyBody(attempt.outcome.answer.body), "raw"));

  const key = attemptKey(attempt);
  const shown = shownMatches.get(key);
  const toggle = node("button", shown ? "Hide match" : "Show match", "quiet");
  toggle.type = "button";
  toggle.setAttribute("aria-expanded", String(Boolean(shown)));
  toggle.addEventListener("click", () => {
    const open = shownMatches.get(key);
    if (open) {
      open.watch.stop();
      shownMatches.delete(key);
    } else {
      const box = document.createElement("div");
      shownMatches.set(key, { box, watch: watchLog(box, { kind: "file", file: attempt.file }, pollSecs) });
    }
    const next = attemptItem(attempt, pollSecs);
    item.replaceWith(next);
    next.querySelector<HTMLButtonElement>("button[aria-expanded]")?.focus();
  });
  const actions = document.createElement("div");
  actions.className = "row";
  actions.append(toggle);
  item.append(actions);
  if (shown) item.append(shown.box);
  return item;
}

function eventItem(line: string): HTMLLIElement {
  const item = document.createElement("li");
  const { time, event, fields } = eventParts(line);
  item.append(node("span", time, "muted"));
  if (event) item.append(node("b", event), node("span", fields.join(" | ")));
  else item.append(node("span", fields.join(""), "muted"));
  return item;
}

function render(info: DebugInfo, pollSecs: () => number): void {
  el("debug-queue").replaceChildren(...info.queue.map((q) => node("li", `${q.file}: ${describeQueued(q)}`)));
  el("debug-queue-empty").hidden = info.queue.length > 0;

  // A match view stops reading once its upload is off the list.
  for (const [key, shown] of shownMatches) {
    if (info.uploads.some((a) => attemptKey(a) === key)) continue;
    shown.watch.stop();
    shownMatches.delete(key);
  }
  el("debug-uploads").replaceChildren(...info.uploads.map((a) => attemptItem(a, pollSecs)));
  el("debug-uploads-empty").hidden = info.uploads.length > 0;

  el("debug-events-file").textContent = info.events?.file ?? "";
  el("debug-events").replaceChildren(...(info.events?.lines ?? []).map(eventItem));
  const error = el("debug-error");
  error.hidden = !info.eventsError;
  error.textContent = info.eventsError ?? "";
}

/** Reads the panel's state every `pollSecs()` while the window is visible, until `stop`. */
export function watchDebug(pollSecs: () => number): { stop(): void } {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  /** What was drawn last: the panel is drawn again only when it changed, so focus stays put. */
  let drawn = "";
  const tick = async () => {
    if (document.visibilityState === "visible") {
      try {
        const info = await invoke<DebugInfo>("get_debug");
        const seen = JSON.stringify(info);
        if (!stopped && seen !== drawn) {
          drawn = seen;
          render(info, pollSecs);
        }
      } catch (err) {
        if (stopped) return;
        drawn = "";
        const error = el("debug-error");
        error.hidden = false;
        error.textContent = err instanceof Error ? err.message : String(err);
      }
    }
    if (!stopped) timer = setTimeout(() => void tick(), pollSecs() * 1000);
  };
  void tick();
  return {
    stop() {
      stopped = true;
      clearTimeout(timer);
      for (const shown of shownMatches.values()) shown.watch.stop();
      shownMatches.clear();
    },
  };
}
