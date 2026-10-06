import "./overlay.css";
import { httpSource, inTauri, tauriSource, type Feed, type Source } from "./feed";
import { overlayModel, readLiveLog, type OverlayModel, type ReadLog } from "./model";
import { sample } from "./sample";
import { docks, draw, h, widgetLabels } from "./widgets";

/**
 * The overlay page (#49): the overlay window's, and the stream page's in OBS (#55). It reads its
 * feed every `pollMs` and draws the widgets that are on. While the host edits the layout, each
 * widget can be dragged, resized (its corner grip, or Ctrl+wheel) and changed from its right-click
 * menu. Widgets sit in three columns (`docks`) until dragged; a dragged one keeps its place as a
 * share of the screen, and each its size as a share of its normal size, so they fit any resolution.
 */

const root = document.getElementById("overlay")!;
const columns = {
  left: h("div", "dock left"),
  right: h("div", "dock right"),
  top: h("div", "dock top"),
};
const free = h("div", "free");
root.append(columns.left, columns.right, columns.top, free);

/** A widget: its slot is placed, the widget inside it is drawn and sized. */
interface Widget {
  slot: HTMLElement;
  el: HTMLElement;
  /** What it says, drawn again only when that changed; its name and grip are kept. */
  content: HTMLElement;
  label: HTMLElement;
  drawn: string;
}

type Layout = Record<string, [number, number]>;
type Sizes = Record<string, number>;

const widgets = new Map<string, Widget>();
let known: { file: string; size: number } | null = null;
let log: ReadLog = readLiveLog(null);
let model: OverlayModel | null = null;
let feed: Feed | null = null;
let timer: number | undefined;
let source: Source;
/** Places and sizes changed since the feed's last, kept until it has them. */
let pending: { layout: Layout; sizes: Sizes } | null = null;
let sizeSave: number | undefined;

const layoutNow = (): Layout => pending?.layout ?? feed?.layout ?? {};
const sizesNow = (): Sizes => pending?.sizes ?? feed?.sizes ?? {};
const canEdit = () => Boolean(feed?.editing && source.saveLayout);

async function poll(): Promise<void> {
  window.clearTimeout(timer);
  try {
    const next = await source.read({ known, names: model?.names ?? [], matchKey: model?.matchKey ?? null });
    if (!next.log) {
      known = null;
      log = readLiveLog(null);
    } else if (next.log.text !== null) {
      known = { file: next.log.file, size: next.log.size };
      log = readLiveLog(next.log.text);
    }
    feed = next;
    model = overlayModel(next, log, Date.parse(next.now) || Date.now());
    if (!next.editing) closeMenu();
    render();
    root.classList.remove("offline");
  } catch (err) {
    // The tool quit, or OBS opened the page before the tool started: try again.
    console.warn("Overlay feed:", err);
    root.classList.add("offline");
  }
  timer = window.setTimeout(() => void poll(), feed?.pollMs ?? 1000);
}

function render(): void {
  if (!feed || !model) return;
  const editing = feed.editing;
  document.body.classList.toggle("editing", editing);
  document.documentElement.style.setProperty("--scale", String(feed.scale / 100));
  document.documentElement.style.setProperty("--opacity", String(feed.opacity / 100));
  toolbar(editing);
  for (const [key, widget] of widgets) {
    if (!feed.widgets.includes(key)) {
      widget.slot.remove();
      widgets.delete(key);
    }
  }
  for (const key of feed.widgets) {
    let drawn = draw(key, model);
    if (!drawn && editing) drawn = draw(key, sample);
    const widget = widgets.get(key) ?? create(key);
    const { slot, el } = widget;
    slot.hidden = !drawn;
    if (drawn) {
      el.dataset.tone = drawn.tone;
      const html = drawn.body.outerHTML;
      if (html !== widget.drawn) {
        widget.content.replaceChildren(drawn.body);
        widget.drawn = html;
      }
    }
    size(widget, key);
    place(slot, key, layoutNow()[key]);
  }
}

function create(key: string): Widget {
  const content = h("div", "content");
  const label = h("span", "label");
  const el = h("section", "widget", content, label, grip(key));
  el.dataset.key = key;
  el.dataset.dock = docks[key] ?? "left";
  const slot = h("div", "slot", el);
  slot.dataset.key = key;
  slot.addEventListener("pointerdown", (e) => startDrag(e, key));
  slot.addEventListener("wheel", (e) => wheel(e, key), { passive: false });
  const widget = { slot, el, content, label, drawn: "" };
  widgets.set(key, widget);
  return widget;
}

function size(widget: Widget, key: string): void {
  const value = sizesNow()[key] ?? 1;
  widget.el.style.zoom = String(value);
  const name = widgetLabels[key] ?? key;
  widget.label.textContent = value === 1 ? name : `${name} ${Math.round(value * 100)}%`;
}

/** In its dock, in the order the widgets are listed, or where the host dragged it. */
function place(slot: HTMLElement, key: string, at: [number, number] | undefined): void {
  if (at) {
    if (slot.parentElement !== free) free.append(slot);
    slot.style.left = `${at[0] * 100}%`;
    slot.style.top = `${at[1] * 100}%`;
  } else {
    slot.style.left = slot.style.top = "";
    const dock = columns[docks[key] ?? "left"];
    if (slot.parentElement !== dock || dock.lastElementChild !== slot) dock.append(slot);
  }
}

function toolbar(editing: boolean): void {
  let bar = document.getElementById("edit-bar");
  if (!editing || !source.saveLayout) {
    bar?.remove();
    return;
  }
  if (bar) return;
  const keys = feed?.hotkeys.find((k) => k.action === "edit")?.keys;
  const reset = h("button", "quiet", "Put them back");
  const done = h("button", null, "Done");
  reset.addEventListener("click", () => void save({}, {}));
  done.addEventListener("click", () => void finish());
  bar = h(
    "div",
    "edit-bar",
    h("p", null, "Drag to move, the corner to resize, right-click for more", keys && h("span", "soft", `. ${keys} ends it too`)),
    reset,
    done,
  );
  bar.id = "edit-bar";
  document.body.append(bar);
}

async function finish(): Promise<void> {
  closeMenu();
  await source.setEditing?.(false);
  await poll();
}

async function save(layout: Layout, sizes: Sizes): Promise<void> {
  pending = { layout, sizes };
  render();
  try {
    await source.saveLayout?.(layout, sizes);
  } finally {
    pending = null;
    await poll();
  }
}

/** A size kept in range, on whole percents. */
function clampSize(value: number): number {
  const [min, max] = feed?.sizeRange ?? [0.5, 2.5, 0.1];
  return Math.round(Math.min(max, Math.max(min, value)) * 100) / 100;
}

function resize(key: string, value: number | null, later = false): void {
  const sizes = { ...sizesNow() };
  if (value === null || clampSize(value) === 1) delete sizes[key];
  else sizes[key] = clampSize(value);
  if (later) {
    // While the wheel turns: shown at once, saved once it stops.
    pending = { layout: layoutNow(), sizes };
    render();
    window.clearTimeout(sizeSave);
    sizeSave = window.setTimeout(() => void save(layoutNow(), sizesNow()), 400);
  } else {
    void save(layoutNow(), sizes);
  }
}

function startDrag(e: PointerEvent, key: string): void {
  if (!canEdit() || e.button !== 0) return;
  closeMenu();
  const slot = e.currentTarget as HTMLElement;
  const box = slot.getBoundingClientRect();
  const offset = { x: e.clientX - box.left, y: e.clientY - box.top };
  slot.setPointerCapture(e.pointerId);
  slot.classList.add("dragging");
  const at = (ev: PointerEvent): [number, number] => {
    const x = Math.min(Math.max(ev.clientX - offset.x, 0), window.innerWidth - box.width);
    const y = Math.min(Math.max(ev.clientY - offset.y, 0), window.innerHeight - box.height);
    return [x / window.innerWidth, y / window.innerHeight];
  };
  const move = (ev: PointerEvent) => {
    pending = { layout: { ...layoutNow(), [key]: at(ev) }, sizes: sizesNow() };
    place(slot, key, pending.layout[key]);
  };
  const end = (ev: PointerEvent) => {
    slot.removeEventListener("pointermove", move);
    slot.classList.remove("dragging");
    void save({ ...layoutNow(), [key]: at(ev) }, sizesNow());
  };
  slot.addEventListener("pointermove", move);
  slot.addEventListener("pointerup", end, { once: true });
}

/** The corner a widget is resized by, while editing. */
function grip(key: string): HTMLElement {
  const handle = h("span", "grip");
  handle.title = "Drag to resize";
  handle.addEventListener("pointerdown", (e) => {
    if (!canEdit() || e.button !== 0) return;
    e.stopPropagation();
    closeMenu();
    const widget = widgets.get(key)!;
    const start = sizesNow()[key] ?? 1;
    const box = widget.el.getBoundingClientRect();
    const from = { x: e.clientX, y: e.clientY };
    handle.setPointerCapture(e.pointerId);
    widget.slot.classList.add("dragging");
    // Grows with the pointer's distance from the widget's corner, keeping its shape.
    const sizeAt = (ev: PointerEvent) => clampSize(start * Math.max((box.width + ev.clientX - from.x) / box.width, (box.height + ev.clientY - from.y) / box.height));
    const move = (ev: PointerEvent) => {
      pending = { layout: layoutNow(), sizes: { ...sizesNow(), [key]: sizeAt(ev) } };
      size(widget, key);
    };
    const end = (ev: PointerEvent) => {
      handle.removeEventListener("pointermove", move);
      widget.slot.classList.remove("dragging");
      resize(key, sizeAt(ev));
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end, { once: true });
  });
  return handle;
}

function wheel(e: WheelEvent, key: string): void {
  if (!canEdit() || !e.ctrlKey) return;
  e.preventDefault();
  const step = feed?.sizeRange[2] ?? 0.1;
  resize(key, (sizesNow()[key] ?? 1) + (e.deltaY < 0 ? step : -step), true);
}

/** The right-click menu while editing: for a widget, or for the whole layout. */
function openMenu(e: MouseEvent): void {
  closeMenu();
  const slot = (e.target as HTMLElement).closest<HTMLElement>(".slot");
  const key = slot?.dataset.key;
  const item = (text: string, action: () => void, disabled = false) => {
    const button = h("button", null, text) as HTMLButtonElement;
    button.disabled = disabled;
    button.addEventListener("click", () => {
      closeMenu();
      action();
    });
    return button;
  };
  const items: HTMLElement[] = [];
  if (key) {
    const current = sizesNow()[key] ?? 1;
    const [min, max, step] = feed?.sizeRange ?? [0.5, 2.5, 0.1];
    items.push(
      h("p", "menu-title", widgetLabels[key] ?? key, h("span", "soft", ` ${Math.round(current * 100)}%`)),
      item("Bigger", () => resize(key, current + step), current >= max),
      item("Smaller", () => resize(key, current - step), current <= min),
      item("Normal size", () => resize(key, null), current === 1),
      item("Back to its place", () => {
        void save(Object.fromEntries(Object.entries(layoutNow()).filter(([k]) => k !== key)), sizesNow());
      }, !layoutNow()[key]),
      h("hr", null),
      item("Hide this widget", () => void source.hideWidget?.(key).then(poll)),
    );
  } else {
    items.push(item("Put every widget back", () => void save({}, {})), item("Done", () => void finish()));
  }
  const menu = h("div", "menu", ...items);
  menu.id = "menu";
  menu.setAttribute("role", "menu");
  document.body.append(menu);
  const { width, height } = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(e.clientX, window.innerWidth - width - 4)}px`;
  menu.style.top = `${Math.min(e.clientY, window.innerHeight - height - 4)}px`;
}

function closeMenu(): void {
  document.getElementById("menu")?.remove();
}

// No browser menu here, ever: while editing, the overlay's own.
document.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (canEdit()) openMenu(e);
});
document.addEventListener("pointerdown", (e) => {
  if (!(e.target as HTMLElement).closest("#menu")) closeMenu();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeMenu();
});

async function start(): Promise<void> {
  if (inTauri()) {
    source = await tauriSource();
    await source.onChange?.(() => void poll());
  } else {
    document.body.classList.add("stream");
    source = httpSource(new URLSearchParams(location.search).get("feed") ?? location.href);
  }
  await poll();
}

void start();
