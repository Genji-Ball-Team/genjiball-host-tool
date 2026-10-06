import { type Kill, type OverlayModel, type Placing, type StatusLine, type Tone } from "./model";

/**
 * Draws each overlay widget from the model (#50–#54). One element per widget, kept between polls:
 * its content is drawn again only when what it says changed, so the kill feed slides in only the
 * kills that are new.
 */

/** Settings' names for the widgets, shown on each while the host places them. Mirrors `config::OVERLAY_WIDGETS`. */
export const widgetLabels: Record<string, string> = {
  logging: "Logging",
  uploads: "Uploads",
  afk: "AFK",
  logCopies: "Log copies",
  rankedCode: "Ranked code",
  roster: "Lobby roster",
  eliminations: "Eliminations",
  standings: "Standings",
  killFeed: "Kill feed",
  roundResult: "Round result",
  matchSummary: "Match summary",
  session: "Session",
  tourney: "Tourney",
};

/** Where a widget sits until the host drags it: one of three columns that stack what shows. */
export const docks: Record<string, "left" | "right" | "top"> = {
  logging: "left",
  uploads: "left",
  afk: "left",
  logCopies: "left",
  rankedCode: "left",
  roster: "left",
  session: "left",
  killFeed: "right",
  eliminations: "right",
  standings: "right",
  tourney: "top",
  roundResult: "top",
  matchSummary: "top",
};

type Child = Node | string | null | false | undefined;

/** An element with a class and children. */
export function h(tag: string, className: string | null, ...children: Child[]): HTMLElement {
  const el = document.createElement(tag);
  if (className) el.className = className;
  for (const child of children) if (child) el.append(child);
  return el;
}

const whole = (n: number) => Math.round(n).toString();

function signed(change: number): string {
  const n = Math.round(change);
  return n > 0 ? `+${n}` : n < 0 ? `−${-n}` : "±0";
}

/** The widget's content, `null` when it has nothing to say now (then it's hidden). */
export function draw(key: string, model: OverlayModel): { tone: Tone; body: HTMLElement } | null {
  switch (key) {
    case "logging":
      return status(model.logging);
    case "uploads":
      return status(model.uploads);
    case "rankedCode":
      return status(model.rankedCode);
    case "afk":
      return model.afk.on ? afk(model.afk.keys) : null;
    case "logCopies":
      return model.logCopies === null || model.logCopies === 0 ? null : logCopies(model.logCopies);
    case "roster":
      return model.roster && roster(model.roster);
    case "eliminations":
      return model.eliminations && eliminations(model.eliminations);
    case "standings":
      return model.standings && { tone: "idle", body: placings("Standings", model.standings) };
    case "killFeed":
      return model.killFeed && { tone: "idle", body: killFeed(model.killFeed) };
    case "roundResult":
      return model.roundResult && roundResult(model.roundResult);
    case "matchSummary":
      return model.matchSummary && summary(model.matchSummary);
    case "session":
      return model.session && session(model.session);
    case "tourney":
      return model.tourney && tourney(model.tourney);
    default:
      return null;
  }
}

function status(line: StatusLine): { tone: Tone; body: HTMLElement } {
  return { tone: line.tone, body: h("div", "status", h("span", "dot"), h("div", null, h("p", "title", line.title), line.detail && h("p", "detail", line.detail))) };
}

function afk(keys: string | null): { tone: Tone; body: HTMLElement } {
  return {
    tone: "bad",
    body: h("div", "afk", h("p", "title", "AFK: your rating is frozen"), h("p", "detail", keys ? `Rounds that start don't count for you. ${keys} turns it off` : "Rounds that start don't count for you")),
  };
}

function logCopies(count: number): { tone: Tone; body: HTMLElement } {
  const text = count === 1 ? "This match is in one log file" : `This match is split over ${count} log files`;
  return { tone: count === 1 ? "good" : "idle", body: h("div", "status", h("span", "dot"), h("div", null, h("p", "title", text), count > 1 && h("p", "detail", "The server keeps the longest copy"))) };
}

function tierChip(tier: { label: string; color: [number, number, number] } | null): HTMLElement {
  if (!tier) return h("span", null);
  const chip = h("span", "tier", tier.label);
  chip.style.setProperty("--tier", `rgb(${tier.color.join(" ")})`);
  return chip;
}

function roster(r: NonNullable<OverlayModel["roster"]>): { tone: Tone; body: HTMLElement } {
  const rows = r.players.map((p) => {
    const rating =
      p.rating !== null ? h("span", "num", whole(p.rating)) : p.state === "unknown" ? h("span", "new", "New") : h("span", "num faint", "…");
    return h(
      "li",
      p.doubled ? "doubled" : null,
      h("span", "rank num", p.rank !== null ? `#${p.rank}` : ""),
      h("span", "name", p.name, p.host && h("span", "host", "host")),
      tierChip(p.tier),
      rating,
    );
  });
  const head = h(
    "header",
    null,
    h("h2", null, `${r.players.length} in the lobby`),
    r.average !== null && h("span", "num soft", `average ${whole(r.average)}`),
  );
  const notes = [
    r.newPlayers > 0 && h("p", "note", r.newPlayers === 1 ? "1 player has no rating yet" : `${r.newPlayers} players have no rating yet`),
    r.doubled && h("p", "note warn", "Two players share a name: the match goes to review"),
  ];
  return { tone: r.doubled ? "warn" : "idle", body: h("div", "roster", head, h("ol", null, ...rows), ...notes) };
}

function eliminations(e: NonNullable<OverlayModel["eliminations"]>): { tone: Tone; body: HTMLElement } {
  const rows = e.out.map((o, i) => h("li", null, h("span", "num faint", `${i + 1}`), h("span", "name", o.name), h("span", "by", o.by ? `by ${o.by}` : "fell")));
  return {
    tone: "idle",
    body: h("div", "elims", h("header", null, h("h2", null, `Round ${e.round}`), h("span", "soft", e.out.length === 0 ? "Nobody out yet" : `${e.out.length} out`)), rows.length > 0 && h("ol", null, ...rows)),
  };
}

function placings(title: string, rows: Placing[]): HTMLElement {
  return h(
    "div",
    "placings",
    h("header", null, h("h2", null, title), h("span", "cols soft", h("span", null, "wins"), h("span", null, "kills"))),
    h(
      "ol",
      null,
      ...rows.map((p) =>
        h("li", p.left ? "left" : null, h("span", "place num", whole(p.place)), h("span", "name", p.name), h("span", "num", whole(p.wins)), h("span", "num soft", whole(p.kills))),
      ),
    ),
  );
}

/** Kills already drawn: only new ones slide in. */
const seenKills = new Set<string>();

function killFeed(kills: Kill[]): HTMLElement {
  const rows = kills.map((k) => {
    const chain = k.chain.length > 1 ? k.chain.slice(0, -1).map((name) => h("span", "via", name)) : [];
    const row = h(
      "li",
      seenKills.has(k.key) ? null : "fresh",
      ...chain,
      h("span", k.killer ? "killer" : "killer none", k.killer ?? "Fell"),
      h("span", "victim", k.victim),
      k.speed !== null && h("span", "speed num", whole(k.speed)),
    );
    return row;
  });
  for (const k of kills) seenKills.add(k.key);
  return h("ol", "feed", ...rows);
}

function roundResult(r: NonNullable<OverlayModel["roundResult"]>): { tone: Tone; body: HTMLElement } {
  if (r.result !== "WIN") {
    const why = r.result === "NONE" ? "Everyone went out: the round starts again" : "The round was stopped";
    return { tone: "warn", body: h("div", "result", h("header", null, h("h2", null, `Round ${r.number}`)), h("p", "soft", why)) };
  }
  const [winner, ...rest] = r.order;
  return {
    tone: "good",
    body: h(
      "div",
      "result",
      h("header", null, h("h2", null, `Round ${r.number}`), winner && h("p", "winner", winner.name, h("span", "soft", " wins"))),
      rest.length > 0 &&
        h("ol", "order", ...rest.map((p) => h("li", p.left ? "left" : null, h("span", "num faint", p.position === null ? "left" : whole(p.position)), h("span", null, p.name)))),
    ),
  };
}

function summary(s: NonNullable<OverlayModel["matchSummary"]>): { tone: Tone; body: HTMLElement } {
  const anyRating = s.rows.some((p) => p.ratingAfter !== null);
  const rows = s.rows.map((p) => {
    const change =
      p.ratingBefore !== null && p.ratingAfter !== null
        ? h("span", `change ${p.ratingAfter >= p.ratingBefore ? "up" : "down"}`, signed(p.ratingAfter - p.ratingBefore))
        : p.ratingAfter !== null
          ? h("span", "change up", "first rating")
          : h("span", "change none", "");
    return h(
      "tr",
      p.left ? "left" : null,
      h("td", "place num", whole(p.place)),
      h("td", "name", p.name),
      h("td", "num", whole(p.wins)),
      h("td", "num soft", whole(p.kills)),
      anyRating && h("td", "num", p.ratingAfter !== null ? whole(p.ratingAfter) : ""),
      anyRating && h("td", null, change),
    );
  });
  const head = h("tr", null, h("th", null, ""), h("th", null, ""), h("th", null, "wins"), h("th", null, "kills"), anyRating && h("th", null, "rating"), anyRating && h("th", null, ""));
  const facts = [
    `${s.rounds} ${s.rounds === 1 ? "round" : "rounds"}`,
    s.topSpeed && `fastest ball ${whole(s.topSpeed.speed)} by ${s.topSpeed.name}`,
  ].filter(Boolean);
  return {
    tone: "good",
    body: h(
      "div",
      "summary",
      h("header", null, h("h2", null, "Match over"), h("span", "soft", facts.join(", "))),
      h("table", null, h("thead", null, head), h("tbody", null, ...rows)),
      !s.rated && h("p", "note", "Rating changes show once the server has rated the match"),
    ),
  };
}

function session(s: NonNullable<OverlayModel["session"]>): { tone: Tone; body: HTMLElement } {
  const stat = (value: string, label: string) => h("div", "stat", h("span", "num big", value), h("span", "soft", label));
  const change =
    s.hostChange !== null && h("div", "stat", h("span", `num big change ${s.hostChange >= 0 ? "up" : "down"}`, signed(s.hostChange)), h("span", "soft", "your rating"));
  return {
    tone: "idle",
    body: h(
      "div",
      "session",
      h("header", null, h("h2", null, "This session")),
      h("div", "stats", stat(whole(s.matches), s.matches === 1 ? "match" : "matches"), stat(whole(s.rounds), "rounds"), stat(whole(s.players), "players"), change),
    ),
  };
}

function tourney(t: NonNullable<OverlayModel["tourney"]>): { tone: Tone; body: HTMLElement } {
  const progress = t.limit !== null ? `Round ${t.round} of ${t.limit}` : `Round ${t.round}`;
  const bar = h("div", "bar");
  if (t.limit) bar.style.setProperty("--done", `${Math.min(1, t.round / t.limit)}`);
  return {
    tone: t.ended ? "warn" : "idle",
    body: h(
      "div",
      "tourney",
      h("header", null, h("h2", null, t.name), h("span", "soft", t.label)),
      h("p", "progress", h("span", null, progress), t.left !== null && !t.ended && h("span", "soft", t.left === 1 ? "1 round left" : `${t.left} rounds left`)),
      t.limit !== null && bar,
      t.ended && h("p", "shot", "Take the verify screenshot now, while the final standings show"),
      placings("Standings", t.standings.slice(0, 5)),
    ),
  };
}

