import "./overlay.css";
import { httpSource, inTauri, tauriSource, type Feed, type Source } from "./feed";
import { overlayModel, readLiveLog, type OverlayModel, type ReadLog } from "./model";
import { sample } from "./sample";
import { docks, draw, h, widgetLabels } from "./widgets";

/**
 * The overlay page (#49): the overlay window's, and the stream page's in OBS (#55). It reads its
 * feed every `pollMs`, draws the widgets that are on, and lets the host drag them while editing.
 * Widgets sit in three columns (`docks`) until dragged; a dragged one keeps its place as a share
 * of the screen, so it fits any resolution.
 */

const root = document.getElementById("overlay")!;
const columns = {
  left: h("div", "dock left"),
  right: h("div", "dock right"),
  top: h("div", "dock top"),
};
const free = h("div", "free");
root.append(columns.left, columns.right, columns.top, free);

const widgets = new Map<string, { el: HTMLElement; drawn: string }>();
let known: { file: string; size: number } | null = null;
let text: string | null = null;
let log: ReadLog = readLiveLog(null);
let model: OverlayModel | null = null;
let feed: Feed | null = null;
let timer: number | undefined;
let source: Source;
/** The places dragged to since the feed's last layout, kept until it has them. */
let dragged: Record<string, [number, number]> | null = null;

async function poll(): Promise<void> {
  window.clearTimeout(timer);
  try {
    const next = await source.read({ known, names: model?.names ?? [], matchKey: model?.matchKey ?? null });
    if (!next.log) {
      known = null;
      text = null;
      log = readLiveLog(null);
    } else if (next.log.text !== null) {
      known = { file: next.log.file, size: next.log.size };
      text = next.log.text;
      log = readLiveLog(text);
    }
    feed = next;
    model = overlayModel(next, log, Date.parse(next.now) || Date.now());
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
  const layout = dragged ?? feed.layout;
  for (const [key, widget] of widgets) {
    if (!feed.widgets.includes(key)) {
      widget.el.remove();
      widgets.delete(key);
    }
  }
  for (const key of feed.widgets) {
    let drawn = draw(key, model);
    if (!drawn && editing) drawn = draw(key, sample);
    let widget = widgets.get(key);
    if (!widget) {
      widget = { el: h("section", "widget"), drawn: "" };
      widget.el.dataset.key = key;
      widget.el.addEventListener("pointerdown", (e) => startDrag(e, key));
      widgets.set(key, widget);
    }
    const { el } = widget;
    el.hidden = !drawn;
    if (drawn) {
      el.dataset.tone = drawn.tone;
      // Drawn again only when it says something else.
      const html = drawn.body.outerHTML;
      if (html !== widget.drawn) {
        el.replaceChildren(drawn.body, h("span", "label", widgetLabels[key] ?? key));
        widget.drawn = html;
      }
    }
    place(el, key, layout[key]);
  }
}

/** In its dock, in the order the widgets are listed, or where the host dragged it. */
function place(el: HTMLElement, key: string, at: [number, number] | undefined): void {
  if (at) {
    if (el.parentElement !== free) free.append(el);
    el.style.left = `${at[0] * 100}%`;
    el.style.top = `${at[1] * 100}%`;
  } else {
    el.style.left = el.style.top = "";
    const dock = columns[docks[key] ?? "left"];
    if (el.parentElement !== dock || dock.lastElementChild !== el) dock.append(el);
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
  reset.addEventListener("click", () => void saveLayout({}));
  done.addEventListener("click", () => void source.setEditing?.(false).then(poll));
  bar = h("div", "edit-bar", h("p", null, "Drag the widgets where you want them", keys && h("span", "soft", `. ${keys} ends it too`)), reset, done);
  bar.id = "edit-bar";
  document.body.append(bar);
}

async function saveLayout(layout: Record<string, [number, number]>): Promise<void> {
  dragged = layout;
  render();
  try {
    await source.saveLayout?.(layout);
  } finally {
    dragged = null;
    await poll();
  }
}

function startDrag(e: PointerEvent, key: string): void {
  if (!feed?.editing || !source.saveLayout || e.button !== 0) return;
  const el = e.currentTarget as HTMLElement;
  const box = el.getBoundingClientRect();
  const offset = { x: e.clientX - box.left, y: e.clientY - box.top };
  el.setPointerCapture(e.pointerId);
  el.classList.add("dragging");
  const at = (ev: PointerEvent): [number, number] => {
    const x = Math.min(Math.max(ev.clientX - offset.x, 0), window.innerWidth - box.width);
    const y = Math.min(Math.max(ev.clientY - offset.y, 0), window.innerHeight - box.height);
    return [x / window.innerWidth, y / window.innerHeight];
  };
  const move = (ev: PointerEvent) => {
    dragged = { ...(dragged ?? feed!.layout), [key]: at(ev) };
    place(el, key, dragged[key]);
  };
  const end = (ev: PointerEvent) => {
    el.removeEventListener("pointermove", move);
    el.classList.remove("dragging");
    void saveLayout({ ...(dragged ?? feed!.layout), [key]: at(ev) });
  };
  el.addEventListener("pointermove", move);
  el.addEventListener("pointerup", end, { once: true });
}

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
