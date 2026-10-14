// @vitest-environment happy-dom
import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setupHome, type HomeState } from "../src/home-view";
import { setupViews, showView } from "../src/views";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const state = (): HomeState => ({
  hasToken: true,
  logFolder: { exists: true },
  dryRun: false,
  settingsError: false,
  upload: null,
  tokenCheck: "ok",
  unreachable: null,
  screenshotsDue: 0,
  serverName: "genjiball.us",
  host: null,
  token: { text: "Token works", tone: "good" },
  region: "Europe",
  lastUpload: null,
  pollSecs: 1,
  lobbyLive: null,
  quietSecs: 60,
});

describe("Home's live log", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("is read only while Home shows, and an answer after it's hidden is dropped", async () => {
    vi.useFakeTimers();
    document.body.innerHTML = /<body>([\s\S]*)<\/body>/.exec(readFileSync("index.html", "utf8"))?.[1] ?? "";
    localStorage.setItem("view", "home");
    let answer: (file: { file: string; writtenAt: string }) => void = () => {};
    invoke.mockImplementation(() => new Promise((resolve) => (answer = resolve)));
    setupViews();
    setupHome(state, () => {});
    expect(invoke).toHaveBeenCalledTimes(1);

    // Another view while the read runs: its answer isn't shown, and nothing is read after it.
    showView("uploads");
    answer({ file: "Log-late.txt", writtenAt: new Date().toISOString() });
    await vi.advanceTimersByTimeAsync(5000);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(document.getElementById("home-log")?.textContent).not.toContain("Log-late.txt");

    // Back on Home: read at once, then every `pollSecs`.
    showView("home");
    expect(invoke).toHaveBeenCalledTimes(2);
    answer({ file: "Log-now.txt", writtenAt: new Date().toISOString() });
    await vi.advanceTimersByTimeAsync(1000);
    expect(document.getElementById("home-log")?.textContent).toContain("Log-now.txt");
    expect(invoke).toHaveBeenCalledTimes(3);
  });
});
