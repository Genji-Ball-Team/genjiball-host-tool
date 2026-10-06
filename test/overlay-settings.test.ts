import { describe, expect, it } from "vitest";
import { feedQuery } from "../src/overlay/feed";
import { hotkeyFrom, withChange, type OverlayView } from "../src/overlay-settings";

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
