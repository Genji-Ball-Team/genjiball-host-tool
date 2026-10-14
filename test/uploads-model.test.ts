import { describe, expect, it } from "vitest";
import { answerRegions, describeAnswer, describeQueued, describeUploads, onSite, type Answer, type UploadedMatch } from "../src/uploads-model";

const match = (change: Partial<UploadedMatch> = {}): UploadedMatch => ({
  matchKey: "482913507226",
  matchId: null,
  lineCount: 120,
  region: "eu",
  action: "insert",
  status: "accepted",
  rejection: null,
  reviewReasons: [],
  ...change,
});

const answered = (matches: UploadedMatch[], result: "stored" | "unchanged" | "duplicate" = "stored"): Answer => ({ kind: "answered", result, region: "eu", matches });

describe("the Uploads status line", () => {
  it("says a problem first, then the dry run, then what waits", () => {
    expect(describeUploads({ problem: { kind: "noFolder" }, dryRun: true, waiting: 2 })).toBe("Waiting for the Workshop log folder.");
    expect(describeUploads({ problem: { kind: "tokenRejected", revoked: true }, dryRun: false, waiting: 0 })).toBe("Paused: this token was revoked. Ask an admin for a new one.");
    expect(describeUploads({ problem: { kind: "local", message: "uploads.json can't be read" }, dryRun: false, waiting: 0 })).toBe("uploads.json can't be read");
    expect(describeUploads({ problem: null, dryRun: true, waiting: 2 })).toMatch(/^Dry run/);
    expect(describeUploads({ problem: null, dryRun: false, waiting: 1 })).toBe("Watching. 1 ranked log to upload.");
    expect(describeUploads({ problem: null, dryRun: false, waiting: 3 })).toBe("Watching. 3 ranked logs to upload.");
    expect(describeUploads({ problem: null, dryRun: false, waiting: 0 })).toBe("Watching for ranked matches.");
  });
});

describe("an upload's answer", () => {
  it("says each match's status, coloured by the worst", () => {
    expect(describeAnswer(answered([match()]))).toEqual({ text: "Accepted", tone: "good" });
    expect(describeAnswer(answered([match(), match({ status: "rejected", rejection: { code: "too_short", message: "too few rounds" } })]))).toEqual({
      text: "Accepted; rejected: too few rounds",
      tone: "bad",
    });
    expect(describeAnswer(answered([match({ status: "void" })])).tone).toBe("muted");
    expect(describeAnswer(answered([match({ status: "rejected" })])).text).toBe("Rejected: no reason given");
  });

  it("names why a match waits for an admin, but not an untrusted host", () => {
    expect(describeAnswer(answered([match({ status: "review", reviewReasons: ["duplicate_name", "untrusted_host"] })])).text).toBe("Waiting for an admin: two players with the same name");
    expect(describeAnswer(answered([match({ status: "review", reviewReasons: ["untrusted_host"] })])).text).toBe("Waiting for an admin");
    expect(describeAnswer(answered([match({ status: "review", reviewReasons: ["something_new"] })])).text).toBe("Waiting for an admin: something_new");
  });

  it("tells a duplicate, an empty file and a refusal apart", () => {
    expect(describeAnswer(answered([match()], "duplicate"))).toEqual({ text: "Already uploaded", tone: "muted" });
    expect(describeAnswer(answered([]))).toEqual({ text: "No match in it", tone: "muted" });
    expect(describeAnswer({ kind: "refused", error: "bad_log", message: "Not a ranked log" })).toEqual({ text: "Not a ranked log", tone: "bad" });
  });

  it("lists the regions its matches are stored in, else the upload's", () => {
    expect(answerRegions(answered([match(), match({ region: "na" }), match()]))).toBe("EU, NA");
    expect(answerRegions(answered([match({ region: null })]))).toBe("EU");
    expect(answerRegions({ kind: "refused", error: "bad_log", message: "" })).toBe("");
  });
});

describe("a queued upload", () => {
  it("says why it waits", () => {
    expect(describeQueued({ kind: "playing" }).tone).toBe("muted");
    expect(describeQueued({ kind: "due" }).text).toBe("Waiting to upload");
    expect(describeQueued({ kind: "failed", error: "offline" })).toEqual({ text: "Upload failed: offline. Retrying automatically.", tone: "bad" });
  });
});

describe("onSite", () => {
  it("links only accepted and voided matches with an id", () => {
    expect(onSite(match({ matchId: 7 }))).toBe(true);
    expect(onSite(match({ matchId: 7, status: "void" }))).toBe(true);
    expect(onSite(match({ matchId: 7, status: "review" }))).toBe(false);
    expect(onSite(match())).toBe(false);
  });
});
