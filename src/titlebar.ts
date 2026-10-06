/**
 * The custom title bar (#37): the window has no Windows frame. It's dragged by the bar
 * (`data-tauri-drag-region`, which also maximises on a double-click), and has its own minimise,
 * maximise and close. Close still hides the window to the tray (Rust's `CloseRequested`). Resting
 * the pointer on maximise shows Windows' snap layouts, as on a native one (`show_snap_layouts`).
 */
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** How long the pointer rests on maximise before the snap layouts show, in milliseconds (Windows waits about as long). */
const SNAP_HOVER_MS = 500;

/** The maximise button's icon: one square, or two for restore. */
const MAXIMIZE_ICON = "M1.5 1.5h7v7h-7z";
const RESTORE_ICON = "M1.5 3.5h5v5h-5zM3.5 3.5v-2h5v5h-2";

function el<T extends Element = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found as unknown as T;
}

/** The connection state next to the title: what Home's status card says, in a word or two. */
export function setTitlebarState(text: string, tone: "good" | "warn" | "bad"): void {
  const state = el("titlebar-state");
  state.hidden = !text;
  state.dataset.tone = tone;
  el("titlebar-state-text").textContent = text;
}

async function showMaximized(): Promise<void> {
  const maximized = await getCurrentWindow().isMaximized();
  el<SVGPathElement>("window-maximize-icon").setAttribute("d", maximized ? RESTORE_ICON : MAXIMIZE_ICON);
  const label = maximized ? "Restore" : "Maximise";
  const button = el("window-maximize");
  button.setAttribute("aria-label", label);
  button.title = label;
}

/** Binds the window's buttons. Call once. */
export async function setupTitlebar(): Promise<void> {
  const window = getCurrentWindow();
  el("window-minimize").addEventListener("click", () => void window.minimize());
  el("window-maximize").addEventListener("click", () => void window.toggleMaximize());
  el("window-close").addEventListener("click", () => void window.close());

  let snapTimer: ReturnType<typeof setTimeout> | undefined;
  const maximize = el("window-maximize");
  maximize.addEventListener("pointerenter", () => {
    snapTimer = setTimeout(() => void invoke("show_snap_layouts"), SNAP_HOVER_MS);
  });
  maximize.addEventListener("pointerleave", () => clearTimeout(snapTimer));
  maximize.addEventListener("pointerdown", () => clearTimeout(snapTimer));

  await window.onResized(() => void showMaximized());
  await showMaximized();
}
