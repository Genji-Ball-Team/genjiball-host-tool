import { describe, expect, it } from "vitest";
import { describeOutcome, eventParts, prettyBody } from "../src/debug-view";

describe("eventParts", () => {
  it("splits an event line into its time, event and fields", () => {
    expect(eventParts("[00:00:02] JOIN|2.38|1|Sparrow|1")).toEqual({ time: "00:00:02", event: "JOIN", fields: ["2.38", "1", "Sparrow", "1"] });
  });

  it("keeps a line that isn't an event as its text", () => {
    expect(eventParts("[00:00:01] some other inspector line, ignored")).toEqual({ time: "00:00:01", event: null, fields: ["some other inspector line, ignored"] });
    expect(eventParts("no prefix")).toEqual({ time: "", event: null, fields: ["no prefix"] });
  });
});

describe("describeOutcome", () => {
  it("tells a dry run, a refusal and a failure apart", () => {
    expect(describeOutcome({ kind: "dryRun" }).tone).toBe("muted");
    expect(describeOutcome({ kind: "answered", answer: { status: 200, body: "{}" } })).toEqual({ text: "The server answered 200", tone: "good" });
    expect(describeOutcome({ kind: "answered", answer: { status: 422, body: "" } }).tone).toBe("bad");
    expect(describeOutcome({ kind: "unanswered", message: "Couldn't reach the server" }).text).toBe("No answer: Couldn't reach the server");
  });
});

describe("prettyBody", () => {
  it("indents JSON and leaves anything else as it is", () => {
    expect(prettyBody('{"result":"stored"}')).toBe('{\n  "result": "stored"\n}');
    expect(prettyBody("Bad Gateway")).toBe("Bad Gateway");
  });
});
