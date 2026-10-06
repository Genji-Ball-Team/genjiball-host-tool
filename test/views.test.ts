// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vitest";
import { VIEWS, markView, onViewChange, savedView, setupViews, showView } from "../src/views";

describe("savedView", () => {
  it("opens the view kept from last time, else Home", () => {
    expect(savedView("uploads")).toBe("uploads");
    expect(savedView(null)).toBe("home");
    expect(savedView("an old view")).toBe("home");
  });
});

describe("the sidebar", () => {
  beforeEach(() => {
    localStorage.clear();
    document.body.innerHTML = VIEWS.map((v) => `<button id="nav-${v}" title="${v}"><i id="nav-${v}-dot" hidden></i></button><section id="view-${v}"></section>`).join("");
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

  it("says when a view needs the host, in its dot and its name", () => {
    markView("uploads", true);
    expect(document.getElementById("nav-uploads-dot")?.hidden).toBe(false);
    expect(document.getElementById("nav-uploads")?.getAttribute("aria-label")).toBe("uploads, needs attention");
    markView("uploads", false);
    expect(document.getElementById("nav-uploads-dot")?.hidden).toBe(true);
    expect(document.getElementById("nav-uploads")?.getAttribute("aria-label")).toBe("uploads");
  });
});
