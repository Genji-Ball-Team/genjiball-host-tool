// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vitest";
import { PANES, VIEWS, markView, onViewChange, savedPane, savedView, setupViews, showPane, showView } from "../src/views";

describe("savedView", () => {
  it("opens the view kept from last time, else Home", () => {
    expect(savedView("uploads")).toBe("uploads");
    expect(savedView(null)).toBe("home");
    expect(savedView("an old view")).toBe("home");
    expect(savedPane("advanced")).toBe("advanced");
    expect(savedPane(null)).toBe("account");
  });
});

describe("the sidebar", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = [
      ...VIEWS.map((v) => `<button id="nav-${v}" title="${v}"><i id="nav-${v}-dot" hidden></i></button><section id="view-${v}"></section>`),
      `<gb-tabs id="settings-tabs" label="Settings">`,
      ...PANES.map((p) => `<button id="pane-${p}-tab"></button>`),
      ...PANES.map((p) => `<div id="pane-${p}"></div>`),
      `</gb-tabs>`,
    ].join("");
  });

  it("shows one view, marks its button and keeps it for the next start", () => {
    const seen: string[] = [];
    onViewChange((view) => seen.push(view));
    localStorage.setItem("view", "settings");
    setupViews();
    expect(document.getElementById("view-settings")?.hidden).toBe(false);
    expect(document.getElementById("view-home")?.hidden).toBe(true);

    document.getElementById("nav-match")?.click();
    expect(document.getElementById("view-match")?.hidden).toBe(false);
    expect(document.getElementById("view-settings")?.hidden).toBe(true);
    expect(document.getElementById("nav-match")?.getAttribute("aria-current")).toBe("page");
    expect(document.getElementById("nav-settings")?.hasAttribute("aria-current")).toBe(false);
    expect(localStorage.getItem("view")).toBe("match");
    expect(seen).toEqual(["settings", "match"]);

    // Showing the view already shown changes nothing.
    showView("match");
    expect(seen).toEqual(["settings", "match"]);
  });

  it("opens a Settings pane, in the Settings view", () => {
    setupViews();
    showPane("game");
    expect(document.getElementById("view-settings")?.hidden).toBe(false);
    expect(document.getElementById("pane-game")?.hidden).toBe(false);
    expect(document.getElementById("pane-account")?.hidden).toBe(true);
    expect(localStorage.getItem("settingsPane")).toBe("game");

    expect(document.getElementById("pane-game-tab")?.getAttribute("aria-selected")).toBe("true");
    expect(document.getElementById("pane-account-tab")?.getAttribute("aria-selected")).toBe("false");

    document.getElementById("pane-lobby-tab")?.click();
    expect(document.getElementById("pane-lobby")?.hidden).toBe(false);
    expect(document.getElementById("pane-game")?.hidden).toBe(true);
    expect(localStorage.getItem("settingsPane")).toBe("lobby");

    // The arrow keys move between the panes too, and the pane is kept.
    document.getElementById("pane-lobby-tab")?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    expect(document.getElementById("pane-overlay")?.hidden).toBe(false);
    expect(document.getElementById("pane-lobby")?.hidden).toBe(true);
    expect(localStorage.getItem("settingsPane")).toBe("overlay");

    // A change inside a pane (a select, a checkbox) doesn't switch panes.
    document.getElementById("pane-overlay")?.dispatchEvent(new Event("change", { bubbles: true }));
    expect(localStorage.getItem("settingsPane")).toBe("overlay");
  });

  it("says when a view needs the host, in its dot and its name", () => {
    markView("uploads", true);
    expect(document.getElementById("nav-uploads-dot")?.hidden).toBe(false);
    expect(document.getElementById("nav-uploads")?.getAttribute("aria-label")).toBe("uploads, needs attention");
    markView("uploads", false);
    expect(document.getElementById("nav-uploads-dot")?.hidden).toBe(true);
    expect(document.getElementById("nav-uploads")?.getAttribute("aria-label")).toBe("uploads");
  });
});
