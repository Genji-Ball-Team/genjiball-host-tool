import { invoke } from "@tauri-apps/api/core";
import { errorText, node } from "./dom";
import { readLog, type LogView, type MatchView, type PlayerView, type Reason, type RoundView } from "./match-model";

/**
 * The match view (#16): a log's matches laid out like the site's match page (genjiball-ranked
 * `public/site.js` `matchPage`): the facts, the winner and standings, then a grid of players down
 * and rounds across. The log's text is untrusted: everything from it goes in with `textContent`.
 */

/** Mirrors `LogText` in src-tauri/src/match_log.rs: what `read_match_log` and `read_live_log` return. */
interface LogText {
  file: string;
  size: number;
  /** `null` when the window already has this file at this size. */
  text: string | null;
}

/** Mirrors `Known` in src-tauri/src/match_log.rs. */
interface Known {
  file: string;
  size: number;
}

/** Which log a view follows: the live one (the newest in the folder) or a file of the upload list. */
export type LogSource = { kind: "live" } | { kind: "file"; file: string };

export interface LogWatch {
  stop(): void;
}

/**
 * Shows `source`'s matches in `box`, and reads the log again every `pollSecs()` while the window is
 * visible, until `stop`. Only a log that grew is sent and drawn again.
 */
export function watchLog(box: HTMLElement, source: LogSource, pollSecs: () => number): LogWatch {
  let known: Known | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  box.classList.add("match-view");
  if (!box.childElementCount) box.replaceChildren(node("p", "Reading the log…", "muted"));

  const read = (): Promise<LogText | null> =>
    source.kind === "live" ? invoke<LogText | null>("read_live_log", { known }) : invoke<LogText>("read_match_log", { file: source.file, known });

  const tick = async () => {
    if (document.visibilityState === "visible") {
      try {
        const log = await read();
        if (stopped) return;
        if (!log) {
          known = null;
          box.replaceChildren(node("p", "No Workshop log yet. One starts when a lobby with ranked logging on starts.", "muted"));
        } else if (log.text !== null) {
          known = { file: log.file, size: log.size };
          box.replaceChildren(...renderLog(readLog(log.text), source.kind === "live" ? log.file : null));
        }
      } catch (err) {
        if (stopped) return;
        known = null;
        box.replaceChildren(node("p", errorText(err), "bad"));
      }
    }
    if (!stopped) timer = setTimeout(() => void tick(), pollSecs() * 1000);
  };
  void tick();
  return {
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}

/** A log's matches. `liveFile` names the live log; only its last match can be in progress. */
export function renderLog(log: LogView, liveFile: string | null): Node[] {
  if (log.legacy) return [node("p", "A log from an older game version (no ranked logging): it's never ranked.", "muted")];
  if (!log.matches.length) {
    return [node("p", liveFile ? `No ranked match in the newest log (${liveFile}) yet.` : "No ranked match in this log: ranked logging was off.", "muted")];
  }
  const last = log.matches.length - 1;
  return log.matches.map((match, i) => renderMatch(match, liveFile !== null && i === last && !match.ended));
}

function renderMatch(m: MatchView, live: boolean): HTMLElement {
  const section = node("section", "", "match");

  const headline = node("div", "", "headline");
  headline.append(node("h3", `Match ${m.matchKey || "without a key"}`));
  if (live) headline.append(chip("Live", "live"));
  else if (!m.ended) headline.append(chip("Unfinished", "plain", "The log ends before the match did"));
  if (m.rejected.some((r) => !soFar(m, r))) headline.append(chip("Won't count", "bad"));
  else if (m.review.length) headline.append(chip("Review", "plain", "An admin checks it before it counts"));
  section.append(headline);

  const facts = [m.map, m.preset && `${m.preset} preset`, m.addOns.length ? `Add-ons: ${m.addOns.join(", ")}` : null, m.gameVersion && `Game version ${m.gameVersion}`, count(m.players.length, "player")];
  section.append(list("facts", facts.filter((f): f is string => !!f).map((f) => node("li", f))));
  section.append(outlook(m));

  if (m.live) section.append(liveRound(m.live));
  if (m.players.length) section.append(...summary(m, live));
  if (m.rounds.length) {
    section.append(node("h4", "Rounds"), grid(m));
  }
  const notes = [
    m.rounds.length ? "Each numbered column is a round: where the player finished, L if they left. A crossed-out round isn't rated." : "No finished round yet.",
    ...m.rounds.map(roundNote).filter((n): n is string => !!n),
  ];
  const note = node("p", "", "note");
  notes.forEach((n, i) => note.append(...(i ? [node("br"), n] : [n])));
  section.append(note);
  if (m.problems.length) section.append(problems(m));
  return section;
}

/** Too few players in rated rounds, in a match that hasn't ended: it may still get them. */
function soFar(m: MatchView, reason: Reason): boolean {
  return reason.code === "too_few_players" && !m.ended;
}

/** Why it won't count or waits for an admin, as the server will read it once it's uploaded. */
function outlook(m: MatchView): HTMLElement {
  const items: HTMLElement[] = [];
  for (const reason of m.rejected) {
    items.push(soFar(m, reason) ? node("li", `So far: ${reason.message}. It counts once rounds are rated.`, "muted") : node("li", `Won't count: ${reason.message}.`, "bad"));
  }
  for (const reason of m.review) items.push(node("li", `${reason.message}.`, "warn"));
  if (!items.length) items.push(node("li", m.ended ? "Counts for the ratings once it's uploaded." : "Counts for the ratings so far.", "good"));
  return list("outlook", items);
}

function liveRound(round: NonNullable<MatchView["live"]>): HTMLElement {
  const box = node("div", "", "live-round");
  box.append(node("h4", `Round ${round.number} in progress`));
  const lines: [string, string[]][] = [
    ["Still in", round.alive],
    ["Out", round.out.map((o) => (o.by ? `${o.name} (by ${o.by})` : o.name))],
    ["Left", round.left],
  ];
  const dl = node("dl");
  for (const [label, names] of lines) {
    if (!names.length && label !== "Still in") continue;
    dl.append(node("dt", label), node("dd", names.join(", ") || "–"));
  }
  box.append(dl);
  return box;
}

/** The winner (or who leads) and everyone's overall place and stats, like the site's recap. */
function summary(m: MatchView, live: boolean): HTMLElement[] {
  const top = m.players.filter((p) => p.place === 1 && p.roundWins > 0);
  const parts: HTMLElement[] = [];
  if (top.length) {
    const podium = node("p", "", "podium");
    const label = live || !m.ended ? "Leading" : top.length > 1 ? "Shared win" : "Winner";
    const inner = node("span");
    inner.append(node("small", label), node("b", top.map((p) => p.name).join(" & ")), node("small", `${count(top[0]!.roundWins, "round")} won, ${count(top[0]!.kills, "kill")}`));
    podium.append(inner);
    parts.push(podium);
  }
  const head = row("th", ["#", "Player", "Won", "Kills", "Deflects", "Streak"], ["place", "name"]);
  const titles = ["", "", "Rounds won, rated or not", "Players they eliminated, not counting themselves", "Balls they deflected", "Most rounds won in a row"];
  head.querySelectorAll("th").forEach((th, i) => {
    th.scope = "col";
    if (titles[i]) th.title = titles[i]!;
  });
  const body = m.players.map((p) => {
    const tr = row("td", [String(p.place), "", String(p.roundWins), String(p.kills), String(p.deflects), String(p.longestStreak)], ["place num", "name"]);
    tr.querySelector(".name")!.append(playerName(p));
    if (p.place === 1 && top.length) tr.className = "first";
    return tr;
  });
  parts.push(scroll(table("standings", head, body)));
  return parts;
}

function playerName(p: PlayerView): HTMLElement {
  const name = node("span", p.name, "player");
  if (p.host) name.append(" ", chip("Host", "plain"));
  if (p.left) name.append(" ", chip("Left", "plain", "Left the match and didn't come back"));
  return name;
}

/** Players down, rounds across: each cell the place, the win, or L for a leaver. */
function grid(m: MatchView): HTMLElement {
  const head = node("tr");
  const nameHead = node("th", "Player", "name");
  nameHead.scope = "col";
  head.append(nameHead);
  for (const r of m.rounds) {
    const th = node("th", String(r.number), r.rated ? "r" : "r unrated");
    th.scope = "col";
    if (!r.rated) th.title = "Not rated";
    head.append(th);
  }
  const body = m.players.map((p, i) => {
    const tr = node("tr");
    tr.append(node("td", p.name, "name"));
    for (const r of m.rounds) tr.append(cell(r, i));
    return tr;
  });
  return scroll(table("grid", head, body));
}

function cell(r: RoundView, row: number): HTMLElement {
  const c = r.cells[row] ?? null;
  const td = node("td");
  td.className = r.rated ? "r" : "r unrated";
  if (!c) return td;
  const stats = [c.kills && count(c.kills, "kill"), c.deflects && count(c.deflects, "deflect")].filter(Boolean).join(", ");
  if (c.left) {
    td.classList.add("left");
    td.textContent = "L";
    td.title = stats ? `Left, ${stats}` : "Left";
  } else if (c.won) {
    td.classList.add("first");
    td.append(node("span", "1"));
    td.title = stats ? `Won, ${stats}` : "Won";
  } else {
    td.textContent = c.position === null ? "–" : String(c.position);
    if (stats) td.title = stats;
  }
  return td;
}

/** Why a round isn't rated, as the site says it. */
function roundNote(r: RoundView): string {
  if (r.result === "NONE") return `Round ${r.number}: everyone died, so it was replayed.`;
  if (r.result === "ABORT") return `Round ${r.number}: stopped before anyone won.`;
  if (r.broken.length) return `Round ${r.number} isn't rated: ${r.broken.join(", ")}.`;
  if (!r.rated) return `Round ${r.number} isn't rated.`;
  return "";
}

function problems(m: MatchView): HTMLElement {
  const details = node("details", "", "problems");
  details.append(node("summary", `${count(m.problems.length, "line")} the server will skip`));
  details.append(list("", m.problems.map((p) => node("li", `Line ${p.line}: ${p.message}`))));
  return details;
}

function chip(label: string, kind: string, title = ""): HTMLElement {
  const found = node("span", label, `chip ${kind}`);
  if (title) found.title = title;
  return found;
}

function list(className: string, items: HTMLElement[]): HTMLUListElement {
  const ul = node("ul", "", className);
  ul.append(...items);
  return ul;
}

function row(cellTag: "th" | "td", cells: string[], classes: string[]): HTMLTableRowElement {
  const tr = node("tr");
  cells.forEach((content, i) => tr.append(node(cellTag, content, classes[i] ?? "num")));
  return tr;
}

function table(className: string, head: HTMLTableRowElement, body: HTMLTableRowElement[]): HTMLTableElement {
  const found = node("table", "", className);
  const thead = node("thead");
  thead.append(head);
  const tbody = node("tbody");
  tbody.append(...body);
  found.append(thead, tbody);
  return found;
}

function scroll(inner: HTMLElement): HTMLElement {
  const box = node("div", "", "scroll");
  box.append(inner);
  return box;
}

function count(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}
