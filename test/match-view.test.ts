// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { example } from "./fixture";
import { readLog } from "../src/match-model";
import { renderLog } from "../src/match-view";

function render(text: string, liveFile: string | null = null): HTMLElement {
  const box = document.createElement("div");
  box.append(...renderLog(readLog(text), liveFile));
  return box;
}

const cells = (box: HTMLElement, selector: string) => [...box.querySelectorAll(selector)].map((c) => c.textContent);

describe("the match view", () => {
  it("lays the example out like the site's match page", () => {
    const box = render(example);
    expect(box.querySelector("h3")?.textContent).toBe("Match 482913507226");
    expect(cells(box, ".facts li")).toEqual(["Workshop Island Night", "Default preset", "Game version 1.3.3R", "6 players"]);
    expect(box.querySelector(".podium b")?.textContent).toBe("Sparrow");
    expect(cells(box, ".standings tbody .place")).toEqual(["1", "2", "3", "3", "5", "5"]);
    expect(cells(box, ".grid thead th")).toEqual(["Player", "1", "2", "3"]);
    // Sparrow won rounds 1 and 3; Tidal (last row) left round 1.
    expect(cells(box, ".grid tbody tr:first-child td")).toEqual(["Sparrow", "1", "3", "1"]);
    expect(box.querySelector(".grid tbody tr:last-child td.left")?.textContent).toBe("L");
    expect(cells(box, ".outlook li")).toEqual(['Two players named "Ghost" at once: an admin checks the match before it counts.']);
    expect(cells(box, ".headline .chip")).toEqual(["Review"]);
  });

  it("shows the round in progress of the live log", () => {
    const lines = example.split("\n");
    const cut = lines.findIndex((l) => l.includes("ELIM|58.40|2|4||4"));
    const box = render(lines.slice(0, cut + 1).join("\n") + "\n", "Log-2026-10-02-20-15-33.txt");
    expect(cells(box, ".headline .chip")).toEqual(["Live", "Review"]);
    expect(box.querySelector(".live-round h4")?.textContent).toBe("Round 2 in progress");
    expect(cells(box, ".live-round dd")).toEqual(["Sparrow, Ghost, Nova", "Mochi (by Nova), Ghost"]);
    expect(box.querySelector(".podium small")?.textContent).toBe("Leading");
  });

  it("puts names from the log in as text, never as markup", () => {
    const name = '<img src=x onerror="alert(1)">';
    const text = ["GBR|1|1|1.3.3R|1", "MATCH_START|1|workshop-island-night|Default|0|", `JOIN|1|1|${name}|1`, "JOIN|1|2|B|", "ROUND_START|2|1|1,2", "ELIM|3|1|2|1|2", "ROUND_END|3|1|1|WIN", "MATCH_END|4|TIME", ""].join("\n");
    const box = render(text);
    expect(box.querySelector("img")).toBeNull();
    expect(box.querySelector(".podium b")?.textContent).toBe(name);
  });

  it("says why a match won't count", () => {
    const box = render(["GBR|1|1|1.3.3R|1", "MATCH_START|1|workshop-island-night|Default|0|", "UNRANKED|1|BOT", "MATCH_END|2|TIME", ""].join("\n"));
    expect(cells(box, ".outlook li")).toEqual(["Won't count: Unranked: a dummy bot is in the match.", "Won't count: No rated round."]);
    expect(cells(box, ".headline .chip")).toEqual(["Won't count"]);
  });

  it("says when a log has no ranked match", () => {
    expect(render("nothing of ours\n", "Log-2026-10-02-20-15-33.txt").textContent).toContain("No ranked match in the newest log");
  });
});
