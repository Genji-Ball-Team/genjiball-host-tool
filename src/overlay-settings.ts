import { invoke } from "@tauri-apps/api/core";
import { toast } from "./toast";

/**
 * Settings → Overlay (#49, #55): the overlay's switch, its widgets, look and hotkeys, and the
 * stream page. Every change is saved at once through `set_overlay`, which checks it; where the
 * widgets sit is the overlay's to save (edit mode).
 */

/** Mirrors `OverlaySettings` in src-tauri/src/settings.rs: what `set_overlay` takes. */
export interface OverlaySettings {
  on: boolean | null;
  widgets: Record<string, boolean>;
  opacity: number | null;
  scale: number | null;
  onlyWithGame: boolean | null;
  layout: Record<string, [number, number]>;
  hotkeys: Record<string, string>;
  stream: boolean | null;
  streamPort: number | null;
  streamWidgets: Record<string, boolean>;
}

interface Range {
  default: number;
  min: number;
  max: number;
}

/** Mirrors `OverlayView` in src-tauri/src/lib.rs. */
export interface OverlayView {
  settings: OverlaySettings;
  on: boolean;
  widgets: { key: string; label: string; group: string; help: string; overlay: boolean; stream: boolean }[];
  widgetsOn: string[];
  streamWidgetsOn: string[];
  opacity: number;
  opacityRange: Range;
  scale: number;
  scaleRange: Range;
  onlyWithGame: boolean;
  hotkeys: { action: string; label: string; default: string; keys: string | null }[];
  hotkeyErrors: string[];
  editing: boolean;
  stream: boolean;
  streamPort: number;
  streamPortRange: Range;
  streamUrl: string | null;
  streamError: string | null;
}

interface Context {
  /** Runs a change with the buttons disabled; errors go to `showError`. */
  busy: (action: () => Promise<void>, showError?: (message: string) => void) => Promise<void>;
  /** Shows the state a command returned. */
  show: (next: { overlay: OverlayView }) => Promise<void>;
}

let view: OverlayView | null = null;
let context: Context;

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found as T;
}

/**
 * The keys a key press makes, as the hotkey settings take them (`Ctrl+Alt+O`): `null` while only
 * modifiers are down. Letters and digits by their key on the keyboard, whatever its layout.
 */
export function hotkeyFrom(e: Pick<KeyboardEvent, "ctrlKey" | "altKey" | "shiftKey" | "metaKey" | "code">): string | null {
  if (/^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/.test(e.code)) return null;
  const key = e.code.replace(/^Key(?=[A-Z]$)/, "").replace(/^Digit(?=\d$)/, "");
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  return [...mods, key].join("+");
}

/** The settings to save: the view's, with `change` made. */
export function withChange(current: OverlayView, change: Partial<OverlaySettings>): OverlaySettings {
  const all = (on: string[]) => Object.fromEntries(current.widgets.map((w) => [w.key, on.includes(w.key)]));
  return {
    ...current.settings,
    on: current.on,
    widgets: all(current.widgetsOn),
    opacity: current.opacity,
    scale: current.scale,
    onlyWithGame: current.onlyWithGame,
    hotkeys: Object.fromEntries(current.hotkeys.map((h) => [h.action, h.keys ?? ""])),
    stream: current.stream,
    streamPort: current.streamPort,
    streamWidgets: all(current.streamWidgetsOn),
    ...change,
  };
}

function setState(text: string, tone: "good" | "bad"): void {
  const state = el("overlay-state");
  state.hidden = !text;
  state.textContent = text;
  state.className = `note ${tone}`;
}

async function save(change: Partial<OverlaySettings>, done?: string): Promise<void> {
  if (!view) return;
  const settings = withChange(view, change);
  await context.busy(
    async () => {
      setState("", "good");
      await context.show(await invoke<{ overlay: OverlayView }>("set_overlay", { settings }));
      if (done) toast(done);
    },
    (message) => {
      setState(message, "bad");
      if (view) render(view);
    },
  );
}

/** The widget switches, by Settings group. */
function widgetGroups(container: HTMLElement, on: string[], stream: boolean): void {
  container.replaceChildren();
  const groups = [...new Set(view!.widgets.map((w) => w.group))];
  for (const group of groups) {
    const box = document.createElement("fieldset");
    const legend = document.createElement("legend");
    legend.textContent = group;
    box.append(legend);
    for (const widget of view!.widgets.filter((w) => w.group === group)) {
      const label = document.createElement("label");
      label.className = "check";
      label.title = widget.help;
      const input = document.createElement("input");
      input.type = "checkbox";
      input.checked = on.includes(widget.key);
      input.addEventListener("change", () => {
        const next = input.checked ? [...on, widget.key] : on.filter((k) => k !== widget.key);
        const map = Object.fromEntries(view!.widgets.map((w) => [w.key, next.includes(w.key)]));
        void save(stream ? { streamWidgets: map } : { widgets: map });
      });
      const text = document.createElement("span");
      text.textContent = widget.label;
      const help = document.createElement("small");
      help.textContent = widget.help;
      label.append(input, text, help);
      box.append(label);
    }
    container.append(box);
  }
}

function hotkeyInputs(): void {
  const container = el("overlay-hotkeys");
  container.replaceChildren();
  for (const hotkey of view!.hotkeys) {
    const label = document.createElement("label");
    label.textContent = hotkey.label;
    const input = document.createElement("input");
    input.type = "text";
    input.readOnly = true;
    input.className = "hotkey";
    input.value = hotkey.keys ?? "";
    input.placeholder = "None";
    input.setAttribute("aria-label", `${hotkey.label}: press the keys`);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Tab") return;
      e.preventDefault();
      if (e.key === "Escape") {
        input.value = hotkey.keys ?? "";
        input.blur();
        return;
      }
      const keys = e.key === "Backspace" || e.key === "Delete" ? "" : hotkeyFrom(e);
      if (keys === null) return;
      input.value = keys;
      const hotkeys = Object.fromEntries(view!.hotkeys.map((h) => [h.action, h.action === hotkey.action ? keys : (h.keys ?? "")]));
      void save({ hotkeys }, keys ? `${hotkey.label}: ${keys}` : `${hotkey.label}: none`);
    });
    const reset = document.createElement("button");
    reset.type = "button";
    reset.className = "quiet";
    reset.textContent = "Default";
    // Kept in the grid when hidden, so the rows stay lined up.
    reset.style.visibility = hotkey.keys === hotkey.default ? "hidden" : "";
    reset.addEventListener("click", () => {
      const hotkeys = Object.fromEntries(view!.hotkeys.map((h) => [h.action, h.action === hotkey.action ? h.default : (h.keys ?? "")]));
      void save({ hotkeys });
    });
    container.append(label, input, reset);
  }
}

function range(id: string, value: number, r: Range): void {
  const input = el<HTMLInputElement>(id);
  input.min = String(r.min);
  input.max = String(r.max);
  input.step = "1";
  if (document.activeElement !== input) input.value = String(value);
  el(`${id}-value`).textContent = `${input.value}%`;
}

export function renderOverlaySettings(next: OverlayView): void {
  view = next;
  render(next);
}

function render(v: OverlayView): void {
  el<HTMLInputElement>("overlay-on").checked = v.on;
  el("overlay-layout-actions").hidden = !v.on;
  const edit = el<HTMLButtonElement>("overlay-edit");
  edit.textContent = v.editing ? "Done editing" : "Edit layout";
  el("overlay-reset").hidden = Object.keys(v.settings.layout).length === 0;
  widgetGroups(el("overlay-widgets"), v.widgetsOn, false);
  range("overlay-opacity", v.opacity, v.opacityRange);
  range("overlay-scale", v.scale, v.scaleRange);
  el<HTMLInputElement>("overlay-with-game").checked = v.onlyWithGame;
  hotkeyInputs();
  const errors = el("overlay-hotkey-error");
  errors.hidden = v.hotkeyErrors.length === 0;
  errors.textContent = v.hotkeyErrors.join(". ");

  el<HTMLInputElement>("stream-on").checked = v.stream;
  el("stream-settings").hidden = !v.stream;
  el("stream-url").textContent = v.streamUrl ?? "Not served";
  el<HTMLButtonElement>("stream-copy").hidden = !v.streamUrl;
  const streamError = el("stream-error");
  streamError.hidden = !v.streamError;
  streamError.textContent = v.streamError ?? "";
  const port = el<HTMLInputElement>("stream-port");
  port.min = String(v.streamPortRange.min);
  port.max = String(v.streamPortRange.max);
  if (document.activeElement !== port) port.value = String(v.streamPort);
  widgetGroups(el("stream-widgets"), v.streamWidgetsOn, true);
}

export function setupOverlaySettings(ctx: Context): void {
  context = ctx;
  el("overlay-on").addEventListener("change", () => {
    const on = el<HTMLInputElement>("overlay-on").checked;
    void save({ on }, on ? "Overlay on" : "Overlay off");
  });
  el("overlay-edit").addEventListener("click", () => {
    void context.busy(async () => context.show(await invoke<{ overlay: OverlayView }>("set_overlay_editing", { on: !view?.editing })), (m) => setState(m, "bad"));
  });
  el("overlay-reset").addEventListener("click", () => {
    void context.busy(async () => {
      await invoke("set_overlay_layout", { layout: {} });
      await context.show(await invoke<{ overlay: OverlayView }>("set_overlay_editing", { on: view?.editing ?? false }));
      toast("Widgets back in their places");
    });
  });
  for (const [id, key] of [
    ["overlay-opacity", "opacity"],
    ["overlay-scale", "scale"],
  ] as const) {
    const input = el<HTMLInputElement>(id);
    input.addEventListener("input", () => (el(`${id}-value`).textContent = `${input.value}%`));
    input.addEventListener("change", () => void save({ [key]: Number(input.value) }));
  }
  el("overlay-with-game").addEventListener("change", () => void save({ onlyWithGame: el<HTMLInputElement>("overlay-with-game").checked }));
  el("stream-on").addEventListener("change", () => {
    const stream = el<HTMLInputElement>("stream-on").checked;
    void save({ stream }, stream ? "Stream page on" : "Stream page off");
  });
  el("stream-port-form").addEventListener("submit", (e) => {
    e.preventDefault();
    void save({ streamPort: Number(el<HTMLInputElement>("stream-port").value) }, "Saved");
  });
  el("stream-copy").addEventListener("click", () => {
    if (!view?.streamUrl) return;
    const url = view.streamUrl;
    void navigator.clipboard.writeText(url).then(() => toast("Copied", url));
  });
}
