/**
 * Desktop feel (#36): the window behaves like an app, not a browser page. No browser context menu
 * outside text fields, no zoom, reload, print or find, no dragging the page's own images and links
 * around, and the app's own shortcuts: Ctrl+1 to Ctrl+5 switch views, Ctrl+Shift+C copies the
 * ranked code. Text selection and overscroll are turned off in the CSS.
 */
import { VIEWS, type View } from "./views";

/** The keys of a key press that matter here. */
export interface Keys {
  key: string;
  code: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

export type Shortcut = { kind: "view"; view: View } | { kind: "copyRankedCode" } | { kind: "block" };

/** The browser's own shortcuts, by key with Ctrl (lower case): reload, print, find, view source, save, downloads, history, zoom. */
const BROWSER_CTRL_KEYS = new Set(["r", "p", "f", "g", "u", "s", "j", "h", "+", "=", "-", "0"]);
/** The browser's own keys without Ctrl: reload, find next, caret browsing. */
const BROWSER_KEYS = new Set(["F5", "F3", "F7", "BrowserBack", "BrowserForward", "BrowserRefresh"]);

/** What a key press does in the window: one of its shortcuts, a browser shortcut to drop, or nothing. */
export function shortcut(keys: Keys): Shortcut | null {
  const ctrl = keys.ctrlKey || keys.metaKey;
  // `code`, so Ctrl+1 works on any keyboard layout (AZERTY gives `&` as the key).
  const digit = /^(?:Digit|Numpad)([1-9])$/.exec(keys.code)?.[1];
  if (ctrl && !keys.shiftKey && !keys.altKey && digit) {
    const view = VIEWS[Number(digit) - 1];
    return view ? { kind: "view", view } : null;
  }
  if (ctrl && keys.shiftKey && keys.code === "KeyC") return { kind: "copyRankedCode" };
  if (ctrl && BROWSER_CTRL_KEYS.has(keys.key.toLowerCase())) return { kind: "block" };
  if (BROWSER_KEYS.has(keys.key)) return { kind: "block" };
  // Back and forward.
  if (keys.altKey && (keys.key === "ArrowLeft" || keys.key === "ArrowRight")) return { kind: "block" };
  return null;
}

/** Text fields keep the browser's menu (cut, copy, paste) and their own keys. */
function isTextField(target: EventTarget | null): boolean {
  return target instanceof HTMLTextAreaElement || (target instanceof HTMLInputElement && !["checkbox", "radio", "button", "submit"].includes(target.type));
}

/** Binds the window's keys, menu, wheel and drags. Call once. */
export function setupDesktop(actions: { showView(view: View): void; copyRankedCode(): void }): void {
  document.addEventListener("keydown", (event) => {
    const found = shortcut(event);
    if (!found) return;
    // In text fields too: their editing keys (Ctrl+C, V, X, A, Z) aren't in the lists.
    event.preventDefault();
    if (found.kind === "view") actions.showView(found.view);
    else if (found.kind === "copyRankedCode") actions.copyRankedCode();
  });
  document.addEventListener("contextmenu", (event) => {
    if (!isTextField(event.target)) event.preventDefault();
  });
  // Ctrl+wheel zooms the page.
  document.addEventListener(
    "wheel",
    (event) => {
      if (event.ctrlKey) event.preventDefault();
    },
    { passive: false },
  );
  // The page's images and links aren't dragged; a file dropped on the window still reaches Tauri.
  document.addEventListener("dragstart", (event) => {
    if (!isTextField(event.target)) event.preventDefault();
  });
}
