// @vitest-environment happy-dom
import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { feedQuery, oneAtATime } from "../src/overlay/feed";
import { hotkeyFrom, renderOverlaySettings, setupOverlaySettings, withChange, type OverlaySettings, type OverlayView } from "../src/overlay-settings";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const press = (code: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) => ({
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  code,
  ...mods,
});

describe("a hotkey from a key press", () => {
  it("names the keys as the settings take them", () => {
    expect(hotkeyFrom(press("KeyO", { ctrlKey: true, altKey: true }))).toBe("Ctrl+Alt+O");
    expect(hotkeyFrom(press("Digit5", { altKey: true, shiftKey: true }))).toBe("Alt+Shift+5");
    expect(hotkeyFrom(press("F9", { ctrlKey: true, metaKey: true }))).toBe("Ctrl+Super+F9");
  });

  it("waits while only modifiers are down", () => {
    expect(hotkeyFrom(press("ControlLeft", { ctrlKey: true }))).toBeNull();
    expect(hotkeyFrom(press("AltRight", { altKey: true }))).toBeNull();
  });
});

it("saves every setting as shown, with the change made", () => {
  const view = {
    settings: { on: true, widgets: {}, opacity: null, scale: null, onlyWithGame: null, layout: { roster: [0.1, 0.2] }, hotkeys: {}, stream: null, streamPort: null, streamWidgets: {} },
    on: true,
    widgets: [
      { key: "afk", label: "AFK", group: "Host status", help: "", overlay: true, stream: false },
      { key: "killFeed", label: "Kill feed", group: "Match", help: "", overlay: false, stream: true },
    ],
    widgetsOn: ["afk"],
    streamWidgetsOn: ["killFeed"],
    opacity: 92,
    scale: 100,
    onlyWithGame: true,
    hotkeys: [
      { action: "toggle", label: "", default: "Ctrl+Alt+O", keys: "Ctrl+Alt+O" },
      { action: "afk", label: "", default: "Ctrl+Alt+A", keys: null },
    ],
    stream: false,
    streamPort: 47623,
  } as unknown as OverlayView;
  expect(withChange(view, { opacity: 50 })).toEqual({
    on: true,
    widgets: { afk: true, killFeed: false },
    opacity: 50,
    scale: 100,
    onlyWithGame: true,
    layout: { roster: [0.1, 0.2] },
    hotkeys: { toggle: "Ctrl+Alt+O", afk: "" },
    stream: false,
    streamPort: 47623,
    streamWidgets: { afk: false, killFeed: true },
  });
});

it("asks the stream page's feed in the query the tool reads", () => {
  expect(feedQuery({ known: { file: "Log-a.txt", size: 12 }, names: ["Kenzo", "A&B"], matchKey: "482913507226" })).toBe(
    "file=Log-a.txt&size=12&name=Kenzo&name=A%26B&matchKey=482913507226",
  );
  expect(feedQuery({ known: null, names: [], matchKey: null })).toBe("");
});

describe("oneAtATime", () => {
  it("reads once more after the running read, however many calls came meanwhile", async () => {
    const ends: (() => void)[] = [];
    const read = vi.fn(() => new Promise<void>((resolve) => ends.push(resolve)));
    const poll = oneAtATime(read);
    const first = poll();
    const second = poll();
    const third = poll();
    expect(second).toBe(third);
    expect(read).toHaveBeenCalledTimes(1);
    ends.shift()?.();
    await first;
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    ends.shift()?.();
    await second;
    expect(read).toHaveBeenCalledTimes(2);
    // Nothing running: reads at once.
    void poll();
    expect(read).toHaveBeenCalledTimes(3);
  });

  it("goes on reading after a read that failed", async () => {
    const ends: { resolve: () => void; reject: (err: Error) => void }[] = [];
    const read = vi.fn(() => new Promise<void>((resolve, reject) => ends.push({ resolve, reject })));
    const poll = oneAtATime(read);
    const failing = poll();
    const after = poll();
    ends.shift()?.reject(new Error("the tool quit"));
    await expect(failing).rejects.toThrow("the tool quit");
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
    ends.shift()?.resolve();
    await after;
    // Not stuck on the failed read: a new call reads again.
    void poll();
    expect(read).toHaveBeenCalledTimes(3);
  });
});

describe("saving the overlay settings", () => {
  const view = (widgetsOn: string[]): OverlayView =>
    ({
      settings: { on: true, opacity: null, scale: null, onlyWithGame: null, stream: null, streamPort: null },
      on: true,
      widgets: [
        { key: "afk", label: "AFK", group: "Host status", help: "", overlay: true, stream: false },
        { key: "killFeed", label: "Kill feed", group: "Match", help: "", overlay: true, stream: true },
      ],
      widgetsOn,
      streamWidgetsOn: [],
      opacity: 100,
      opacityRange: { default: 100, min: 20, max: 100 },
      scale: 100,
      scaleRange: { default: 100, min: 50, max: 200 },
      onlyWithGame: true,
      hotkeys: [],
      hotkeyErrors: [],
      editing: false,
      stream: false,
      streamPort: 47623,
      streamPortRange: { default: 47623, min: 1024, max: 65535 },
      streamUrl: null,
      streamError: null,
    }) as OverlayView;

  it("starts a change made while a save runs from what that save saved", async () => {
    document.body.innerHTML = /<body>([\s\S]*)<\/body>/.exec(readFileSync("index.html", "utf8"))?.[1] ?? "";
    const answers: ((next: { overlay: OverlayView }) => void)[] = [];
    invoke.mockImplementation(() => new Promise((resolve) => answers.push(resolve)));
    setupOverlaySettings({
      busy: (action) => action(),
      show: async (next) => renderOverlaySettings(next.overlay),
    });
    renderOverlaySettings(view(["afk"]));
    const box = (n: number) => document.querySelectorAll<HTMLInputElement>("#overlay-widgets input")[n]!;

    // AFK off, then the kill feed on before the first save's answer.
    box(0).checked = false;
    box(0).dispatchEvent(new Event("change"));
    box(1).checked = true;
    box(1).dispatchEvent(new Event("change"));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    const sent = (call: number) => (invoke.mock.calls[call]?.[1] as { settings: OverlaySettings }).settings.widgets;
    expect(sent(0)).toEqual({ afk: false, killFeed: false });

    answers.shift()?.({ overlay: view([]) });
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(sent(1)).toEqual({ afk: false, killFeed: true });
    answers.shift()?.({ overlay: view(["killFeed"]) });
  });

  it("goes on saving after a save that failed", async () => {
    document.body.innerHTML = /<body>([\s\S]*)<\/body>/.exec(readFileSync("index.html", "utf8"))?.[1] ?? "";
    invoke.mockReset();
    const answers: { resolve: (next: { overlay: OverlayView }) => void; reject: (err: string) => void }[] = [];
    invoke.mockImplementation(() => new Promise((resolve, reject) => answers.push({ resolve, reject })));
    const errors: string[] = [];
    setupOverlaySettings({
      // Shows the error, and passes it on: the next save must run all the same.
      busy: async (action, showError) => {
        try {
          await action();
        } catch (err) {
          errors.push(String(err));
          showError(String(err));
          throw err;
        }
      },
      show: async (next) => renderOverlaySettings(next.overlay),
    });
    renderOverlaySettings(view(["afk"]));
    const box = (n: number) => document.querySelectorAll<HTMLInputElement>("#overlay-widgets input")[n]!;

    box(0).checked = false;
    box(0).dispatchEvent(new Event("change"));
    box(1).checked = true;
    box(1).dispatchEvent(new Event("change"));
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    answers.shift()?.reject("The settings file can't be written");
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(errors).toEqual(["The settings file can't be written"]);
    // Made to the settings as last saved: the failed change isn't in it.
    const sent = (invoke.mock.calls[1]?.[1] as { settings: OverlaySettings }).settings.widgets;
    expect(sent).toEqual({ afk: true, killFeed: true });
    answers.shift()?.resolve({ overlay: view(["afk", "killFeed"]) });
    await vi.waitFor(() => expect(document.getElementById("overlay-state")?.hidden).toBe(true));
  });
});
