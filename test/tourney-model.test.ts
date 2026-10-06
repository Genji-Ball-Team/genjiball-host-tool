import { describe, expect, it } from "vitest";
import { checkScreenshot, codeState, countdown, imageType, regionWarning, screenshotState, takenBeforeStart, type LobbyView } from "../src/tourney-model";

/** The lobby of genjiball-ranked `docs/api.md`'s example, before its code window. */
function lobby(change: Partial<LobbyView> = {}): LobbyView {
  return {
    id: 7,
    label: "Lobby 1/2",
    region: "eu",
    roundLimit: 30,
    tourney: { id: 3, name: "October Cup", region: "eu", startsAt: "2026-10-10T17:00:00Z", status: "scheduled" },
    matchId: null,
    screenshot: null,
    screenshotExpired: false,
    verified: false,
    codeFrom: "2026-10-10T16:00:00Z",
    code: null,
    lobbyKey: null,
    matchUploaded: false,
    needsScreenshot: false,
    ...change,
  };
}

const values = { lobbyKey: "482913507226", roundLimit: 30, name: "October Cup", label: "Lobby 1/2" };

describe("codeState", () => {
  it("says when the code window opens", () => {
    expect(codeState(lobby(), new Date("2026-10-10T15:00:00Z"))).toEqual({ kind: "opens", at: new Date("2026-10-10T16:00:00Z") });
  });

  it("is open while the server gives the values, or once the window has opened since the list was read", () => {
    expect(codeState(lobby({ code: values }), new Date("2026-10-10T15:00:00Z"))).toEqual({ kind: "open" });
    expect(codeState(lobby(), new Date("2026-10-10T16:00:01Z"))).toEqual({ kind: "open" });
  });

  it("is closed once the lobby is done", () => {
    const now = new Date("2026-10-10T16:30:00Z");
    expect(codeState(lobby({ code: values, matchId: 812 }), now)).toEqual({ kind: "closed" });
    expect(codeState(lobby({ tourney: { ...lobby().tourney, status: "cancelled" } }), now)).toEqual({ kind: "closed" });
    expect(codeState(lobby({ codeFrom: null }), now)).toEqual({ kind: "closed" });
  });
});

describe("countdown", () => {
  it("counts down in the largest units that matter", () => {
    expect(countdown((2 * 86400 + 3 * 3600 + 59) * 1000)).toBe("in 2 d 3 h");
    expect(countdown((3600 + 5 * 60) * 1000)).toBe("in 1 h 05 min");
    expect(countdown((4 * 60 + 9) * 1000)).toBe("in 4 min 09 s");
    expect(countdown(42_400)).toBe("in 42 s");
    expect(countdown(0)).toBe("now");
    expect(countdown(-5000)).toBe("now");
  });
});

describe("regionWarning", () => {
  it("warns only when the tool uploads as another region than the lobby's", () => {
    expect(regionWarning("na", "eu")).toBe(true);
    expect(regionWarning("eu", "eu")).toBe(false);
    // Not known yet (the home region hasn't been asked): no warning that may be wrong.
    expect(regionWarning("na", null)).toBe(false);
  });
});

const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]);
const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xe0]);
const webp = new TextEncoder().encode("RIFF\0\0\0\0WEBPVP8 ");

describe("imageType", () => {
  it("knows the images the server takes by their content", () => {
    expect(imageType(png)).toBe("image/png");
    expect(imageType(jpeg)).toBe("image/jpeg");
    expect(imageType(webp)).toBe("image/webp");
    expect(imageType(new TextEncoder().encode("GIF89a"))).toBeNull();
    expect(imageType(new TextEncoder().encode("RIFF\0\0\0\0WAVE"))).toBeNull();
    expect(imageType(new Uint8Array())).toBeNull();
  });
});

describe("checkScreenshot", () => {
  const max = 8 * 1024 * 1024;

  it("takes a PNG, JPEG or WebP up to the server's limit", () => {
    expect(checkScreenshot(png, max)).toEqual({ ok: true, type: "image/png" });
    const biggest = new Uint8Array(max);
    biggest.set(jpeg);
    expect(checkScreenshot(biggest, max)).toEqual({ ok: true, type: "image/jpeg" });
  });

  it("says why it won't", () => {
    const tooBig = new Uint8Array(max + 1);
    tooBig.set(png);
    expect(checkScreenshot(tooBig, max)).toEqual({ ok: false, error: "The image is over the server's 8 MB limit" });
    expect(checkScreenshot(new Uint8Array(), max)).toEqual({ ok: false, error: "The image is empty" });
    expect(checkScreenshot(new TextEncoder().encode("<html>"), max).ok).toBe(false);
  });
});

describe("takenBeforeStart", () => {
  it("flags a screenshot older than the tourney", () => {
    expect(takenBeforeStart("2026-10-10T16:59:00Z", "2026-10-10T17:00:00Z")).toBe(true);
    expect(takenBeforeStart("2026-10-10T18:10:00Z", "2026-10-10T17:00:00Z")).toBe(false);
  });
});

describe("screenshotState", () => {
  it("asks for the screenshot once the match is uploaded, until an admin verified it", () => {
    expect(screenshotState(lobby())).toBeNull();
    expect(screenshotState(lobby({ matchId: 812, needsScreenshot: true }))?.tone).toBe("bad");
    expect(screenshotState(lobby({ needsScreenshot: true, screenshotExpired: true }))?.text).toContain("expired");
    expect(screenshotState(lobby({ matchId: 812, screenshot: "/api/screenshots/a" }))?.tone).toBe("muted");
    expect(screenshotState(lobby({ matchId: 812, screenshot: "/api/screenshots/a", verified: true }))).toEqual({ text: "Screenshot verified by an admin.", tone: "good" });
  });
});
