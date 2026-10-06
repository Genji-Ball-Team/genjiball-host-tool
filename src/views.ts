/**
 * The app shell (#33): the sidebar and its views. One view shows at a time and fills the window;
 * the last one shown opens again at the next start (`localStorage`).
 */

export const VIEWS = ["home", "match", "uploads", "tourneys", "settings"] as const;
export type View = (typeof VIEWS)[number];

const VIEW_KEY = "view";

/** The view to open with: the one kept from last time, else Home. */
export function savedView(stored: string | null): View {
  return VIEWS.find((v) => v === stored) ?? "home";
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
const listeners: ((view: View) => void)[] = [];

function el(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found;
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
  for (const v of VIEWS) {
    el(`view-${v}`).hidden = v !== view;
    const nav = el(`nav-${v}`);
    if (v === view) nav.setAttribute("aria-current", "page");
    else nav.removeAttribute("aria-current");
  }
  store(VIEW_KEY, view);
  for (const listener of listeners) listener(view);
}

/** Binds the sidebar and shows the view kept from last time. Call once, after `onViewChange`s. */
export function setupViews(): void {
  for (const v of VIEWS) el(`nav-${v}`).addEventListener("click", () => showView(v));
  showView(savedView(stored(VIEW_KEY)));
}

/** Marks a view's sidebar button as needing the host: a red dot, and its name says so. */
export function markView(view: View, needsHost: boolean): void {
  el(`nav-${view}-dot`).hidden = !needsHost;
  const nav = el(`nav-${view}`);
  nav.setAttribute("aria-label", needsHost ? `${nav.title}, needs attention` : nav.title);
}
