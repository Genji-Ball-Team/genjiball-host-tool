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

  it("is read on every view for the title bar's pulse, and at once when Home shows", async () => {
    vi.useFakeTimers();
    document.body.innerHTML = /<body>([\s\S]*)<\/body>/.exec(readFileSync("index.html", "utf8"))?.[1] ?? "";
    localStorage.setItem("view", "uploads");
    const answers: ((file: { file: string; writtenAt: string }) => void)[] = [];
    invoke.mockImplementation(() => new Promise((resolve) => answers.push(resolve)));
    const pulsing = () => document.getElementById("titlebar-state")?.classList.contains("playing");
    setupViews();
    setupHome(state, () => {});
    expect(invoke).toHaveBeenCalledTimes(1);

    // On Uploads, with the live lobby off: a growing live log still makes the title bar pulse.
    answers.shift()?.({ file: "Log-a.txt", writtenAt: new Date().toISOString() });
    await vi.advanceTimersByTimeAsync(0);
    expect(pulsing()).toBe(true);
    await vi.advanceTimersByTimeAsync(1000);
    expect(invoke).toHaveBeenCalledTimes(2);

    // Home shows while that read runs: read again at once, and the older answer is dropped.
    showView("home");
    expect(invoke).toHaveBeenCalledTimes(3);
    const [late, now] = answers.splice(0);
    now?.({ file: "Log-now.txt", writtenAt: new Date().toISOString() });
    late?.({ file: "Log-late.txt", writtenAt: new Date().toISOString() });
    await vi.advanceTimersByTimeAsync(0);
    expect(document.getElementById("home-log")?.textContent).toContain("Log-now.txt");

    // One loop: one read per `pollSecs`.
    await vi.advanceTimersByTimeAsync(1000);
    expect(invoke).toHaveBeenCalledTimes(4);
  });
});
