import { describe, expect, it } from "vitest";
import { ago, headline, matchInProgress, problems, type HomeInput } from "../src/home-model";

const fine: HomeInput = {
  hasToken: true,
  logFolder: { exists: true },
  dryRun: false,
  settingsError: false,
  upload: { problem: null, waiting: 0, retrying: null },
  tokenCheck: "ok",
  unreachable: null,
  screenshotsDue: 0,
};

const labels = (input: HomeInput) => problems(input).map((p) => p.fix?.label);

describe("Home's headline", () => {
  it("says uploads go on, wait or are paused", () => {
    expect(headline(fine, "genjiball.us")).toEqual({ text: "Uploading to genjiball.us", detail: "Watching for ranked matches.", tone: "good" });
    expect(headline({ ...fine, upload: { problem: null, waiting: 2, retrying: null } }, "genjiball.us").detail).toBe("2 ranked logs to upload.");
    expect(headline({ ...fine, upload: { problem: null, waiting: 0, retrying: "Log.txt" } }, "genjiball.us").tone).toBe("warn");
    expect(headline({ ...fine, dryRun: true }, "genjiball.us").tone).toBe("warn");
    expect(headline({ ...fine, upload: { problem: { kind: "noFolder" }, waiting: 0, retrying: null } }, "genjiball.us")).toMatchObject({ text: "Uploads are paused", tone: "bad" });
  });
});

describe("Home's problems", () => {
  it("lists none when all is well", () => {
    expect(problems(fine)).toEqual([]);
  });

  it("sends a bad token to the Account pane", () => {
    const rejected = problems({ ...fine, upload: { problem: { kind: "tokenRejected", revoked: true }, waiting: 0, retrying: null } });
    expect(rejected).toEqual([{ text: "Your host token was revoked. Ask an admin for a new one.", fix: { kind: "changeToken", label: "Change token" } }]);
    expect(problems({ ...fine, tokenCheck: "unknown" })[0]?.text).toBe("The server doesn't know your host token.");
    expect(labels({ ...fine, tokenCheck: "unreachable", unreachable: "offline" })).toEqual(["Check again"]);
  });

  it("sends a missing log folder and a missing region to the Game pane", () => {
    expect(problems({ ...fine, logFolder: { exists: false } })[0]?.fix).toEqual({ kind: "pane", pane: "game", label: "Choose folder" });
    expect(problems({ ...fine, upload: { problem: { kind: "noRegion" }, waiting: 0, retrying: null } })[0]?.fix).toMatchObject({ pane: "game" });
  });

  it("turns the dry run off, and points at failed uploads and due screenshots", () => {
    expect(labels({ ...fine, dryRun: true, upload: { problem: null, waiting: 0, retrying: "Log.txt: offline" }, screenshotsDue: 2 })).toEqual(["Turn it off", "Uploads", "Tourneys"]);
    expect(problems({ ...fine, screenshotsDue: 2 })[0]?.text).toBe("2 tourney lobbies need your verify screenshot.");
  });
});

describe("ago", () => {
  it("says how long ago, shortly", () => {
    const now = Date.parse("2026-10-06T12:00:00Z");
    expect(ago("2026-10-06T11:59:48Z", now)).toBe("12 s ago");
    expect(ago("2026-10-06T11:57:00Z", now)).toBe("3 min ago");
    expect(ago("2026-10-06T09:00:00Z", now)).toBe("3 h ago");
  });
});

describe("matchInProgress", () => {
  const now = Date.parse("2026-10-06T12:00:00Z");

  it("takes the live lobby's word while it's on", () => {
    expect(matchInProgress("playing", null, 60, now)).toBe(true);
    // An unranked match grows the log too, but isn't one.
    expect(matchInProgress("unranked", "2026-10-06T11:59:58Z", 60, now)).toBe(false);
  });

  it("else goes by the live log growing within the quiet time", () => {
    expect(matchInProgress(null, "2026-10-06T11:59:30Z", 60, now)).toBe(true);
    expect(matchInProgress(null, "2026-10-06T11:58:00Z", 60, now)).toBe(false);
    expect(matchInProgress(null, null, 60, now)).toBe(false);
  });
});
