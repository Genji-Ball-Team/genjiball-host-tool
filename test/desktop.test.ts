import { describe, expect, it } from "vitest";
import { shortcut, type Keys } from "../src/desktop";

const press = (key: string, code: string, mods: Partial<Keys> = {}): Keys => ({ key, code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods });

describe("shortcut", () => {
  it("switches views with Ctrl and a digit, on any layout", () => {
    expect(shortcut(press("1", "Digit1", { ctrlKey: true }))).toEqual({ kind: "view", view: "home" });
    expect(shortcut(press("&", "Digit1", { ctrlKey: true }))).toEqual({ kind: "view", view: "home" });
    expect(shortcut(press("5", "Numpad5", { ctrlKey: true }))).toEqual({ kind: "view", view: "settings" });
    expect(shortcut(press("9", "Digit9", { ctrlKey: true }))).toBeNull();
    expect(shortcut(press("1", "Digit1"))).toBeNull();
  });

  it("copies the ranked code with Ctrl+Shift+C", () => {
    expect(shortcut(press("C", "KeyC", { ctrlKey: true, shiftKey: true }))).toEqual({ kind: "copyRankedCode" });
    // Plain Ctrl+C copies text as usual.
    expect(shortcut(press("c", "KeyC", { ctrlKey: true }))).toBeNull();
  });

  it("drops the browser's own shortcuts", () => {
    for (const keys of [
      press("r", "KeyR", { ctrlKey: true }),
      press("p", "KeyP", { ctrlKey: true }),
      press("f", "KeyF", { ctrlKey: true }),
      press("=", "Equal", { ctrlKey: true }),
      press("0", "Digit0", { ctrlKey: true }),
      press("F5", "F5"),
      press("F7", "F7"),
      press("ArrowLeft", "ArrowLeft", { altKey: true }),
    ]) {
      expect(shortcut(keys), keys.key).toEqual({ kind: "block" });
    }
    expect(shortcut(press("a", "KeyA", { ctrlKey: true }))).toBeNull();
    expect(shortcut(press("v", "KeyV", { ctrlKey: true }))).toBeNull();
  });
});
