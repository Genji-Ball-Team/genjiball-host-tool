import type { OverlayModel } from "./model";

/**
 * What a widget shows while the host places them (edit mode) and it has nothing to say yet: made
 * up, so every widget on can be seen and dragged.
 */
export const sample: OverlayModel = {
  logging: { tone: "good", title: "Recording", detail: "Round 4, 6 players" },
  uploads: { tone: "good", title: "Uploaded and counted", detail: null },
  afk: { on: true, keys: "Ctrl+Alt+A" },
  logCopies: 2,
  rankedCode: { tone: "good", title: "Ranked code copied 25 min ago", detail: null },
  roster: {
    players: [
      { name: "Sparrow", host: true, state: "found", rating: 2214, rank: 2, tier: { label: "Ascendant", color: [60, 160, 255] }, doubled: false },
      { name: "Nova", host: false, state: "found", rating: 1932, rank: 7, tier: { label: "Grandmaster", color: [255, 140, 0] }, doubled: false },
      { name: "Mochi", host: false, state: "found", rating: 1648, rank: 19, tier: { label: "Master", color: [255, 215, 0] }, doubled: false },
      { name: "Tidal", host: false, state: "found", rating: 1402, rank: null, tier: { label: "Apprentice", color: [205, 127, 50] }, doubled: false },
      { name: "Kite", host: false, state: "unknown", rating: null, rank: null, tier: null, doubled: false },
    ],
    average: 1799,
    newPlayers: 1,
    doubled: false,
  },
  eliminations: {
    round: 4,
    out: [
      { name: "Kite", by: "Nova" },
      { name: "Tidal", by: null },
    ],
  },
  standings: [
    { place: 1, name: "Sparrow", wins: 2, kills: 7, left: false },
    { place: 2, name: "Nova", wins: 1, kills: 5, left: false },
    { place: 3, name: "Mochi", wins: 0, kills: 3, left: false },
    { place: 4, name: "Tidal", wins: 0, kills: 1, left: false },
  ],
  killFeed: [
    { key: "s3", killer: "Nova", victim: "Kite", chain: ["Sparrow", "Mochi", "Nova"], speed: 142, },
    { key: "s2", killer: null, victim: "Tidal", chain: [], speed: null },
    { key: "s1", killer: "Sparrow", victim: "Mochi", chain: ["Mochi", "Sparrow"], speed: 96 },
  ],
  roundResult: {
    number: 3,
    result: "WIN",
    order: [
      { position: 1, name: "Sparrow", left: false },
      { position: 2, name: "Nova", left: false },
      { position: 3, name: "Mochi", left: false },
      { position: 4, name: "Tidal", left: false },
    ],
  },
  matchSummary: {
    rows: [
      { place: 1, name: "Sparrow", wins: 4, kills: 11, left: false, ratingBefore: 2198, ratingAfter: 2214 },
      { place: 2, name: "Nova", wins: 3, kills: 9, left: false, ratingBefore: 1925, ratingAfter: 1932 },
      { place: 3, name: "Mochi", wins: 1, kills: 6, left: false, ratingBefore: 1659, ratingAfter: 1648 },
      { place: 4, name: "Tidal", wins: 0, kills: 2, left: false, ratingBefore: 1414, ratingAfter: 1402 },
    ],
    rounds: 8,
    topSpeed: { name: "Nova", speed: 187 },
    rated: true,
  },
  session: { matches: 3, rounds: 24, players: 9, host: "Sparrow", hostChange: 31 },
  tourney: {
    name: "October Cup",
    label: "Lobby 1/2",
    round: 12,
    limit: 30,
    left: 18,
    standings: [
      { place: 1, name: "Sparrow", wins: 5, kills: 14, left: false },
      { place: 2, name: "Nova", wins: 4, kills: 12, left: false },
      { place: 3, name: "Mochi", wins: 2, kills: 9, left: false },
    ],
    ended: false,
  },
  names: [],
  matchKey: null,
};
