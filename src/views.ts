/**
 * The app shell (#33): the sidebar and its views. One view shows at a time and fills the window;
 * the last one shown opens again at the next start (`localStorage`). The Settings view has panes
 * (#35) the same way.
 */

export const VIEWS = ["home", "match", "uploads", "tourneys", "settings"] as const;
export type View = (typeof VIEWS)[number];

export const PANES = ["account", "game", "lobby", "updates", "advanced"] as const;
export type Pane = (typeof PANES)[number];

const VIEW_KEY = "view";
const PANE_KEY = "settingsPane";

/** The view to open with: the one kept from last time, else Home. */
export function savedView(stored: string | null): View {
  return VIEWS.find((v) => v === stored) ?? "home";
}

/** The Settings pane to open with: the one kept from last time, else Account. */
export function savedPane(stored: string | null): Pane {
  return PANES.find((p) => p === stored) ?? "account";
}

/** `localStorage`, which can be off or full: then nothing is kept, and the window goes on. */
function stored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function store(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch (err) {
    console.warn(`Couldn't keep ${key}:`, err);
  }
}

let current: View | null = null;
let currentPane: Pane | null = null;
const listeners: ((view: View) => void)[] = [];

function el(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found;
}

/** Shows `shown`'s panel only, and marks its button. */
function mark<T extends string>(names: readonly T[], shown: T, panel: (name: T) => string, nav: (name: T) => string): void {
  for (const name of names) {
    el(panel(name)).hidden = name !== shown;
    if (name === shown) el(nav(name)).setAttribute("aria-current", "page");
    else el(nav(name)).removeAttribute("aria-current");
  }
}

export function currentView(): View | null {
  return current;
}

/** Calls `listener` with each view shown from now on. */
export function onViewChange(listener: (view: View) => void): void {
  listeners.push(listener);
}

export function showView(view: View): void {
  if (view === current) return;
  current = view;
  mark(VIEWS, view, (v) => `view-${v}`, (v) => `nav-${v}`);
  store(VIEW_KEY, view);
  for (const listener of listeners) listener(view);
}

function choosePane(pane: Pane): void {
  if (pane === currentPane) return;
  currentPane = pane;
  mark(PANES, pane, (p) => `pane-${p}`, (p) => `pane-${p}-tab`);
  store(PANE_KEY, pane);
}

/** Shows a Settings pane, in the Settings view. */
export function showPane(pane: Pane): void {
  choosePane(pane);
  showView("settings");
}

/** Binds the sidebar and the Settings panes, and shows those kept from last time. Call once, after `onViewChange`s. */
export function setupViews(): void {
  for (const v of VIEWS) el(`nav-${v}`).addEventListener("click", () => showView(v));
  for (const p of PANES) el(`pane-${p}-tab`).addEventListener("click", () => choosePane(p));
  choosePane(savedPane(stored(PANE_KEY)));
  showView(savedView(stored(VIEW_KEY)));
}

/** Marks a view's sidebar button as needing the host: a red dot, and its name says so. */
export function markView(view: View, needsHost: boolean): void {
  el(`nav-${view}-dot`).hidden = !needsHost;
  const nav = el(`nav-${view}`);
  nav.setAttribute("aria-label", needsHost ? `${nav.title}, needs attention` : nav.title);
}
