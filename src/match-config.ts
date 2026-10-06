/**
 * The ranked server's rules the match view needs, to show a match as the server will read it. Each
 * mirrors a value in genjiball-ranked: keep them the same when the server's change.
 */

/** Log format versions the parser reads: genjiball-ranked `src/config.ts` `acceptedLogFormats`. */
export const acceptedLogFormats: readonly number[] = [1, 2];

/**
 * Different players a match needs in its rated rounds, or the server rejects it (`too_few_players`):
 * genjiball-ranked `src/config.ts` `minMatchPlayers`.
 */
export const minMatchPlayers = 2;

/**
 * Players a round's finishing order needs for the round to be rated: genjiball-ranked
 * `src/upload/plan.ts` `isRated`, from GenjiBall-CE `docs/ranked-log.md` ("A round is rated if at
 * least 2 players are left").
 */
export const minRatedRoundPlayers = 2;

/** Map names as the site shows them (genjiball-ranked `public/site.js` `mapNames`). */
export const mapNames: Readonly<Record<string, string>> = { "workshop-island-night": "Workshop Island Night" };

/** `UNRANKED` reasons in words (GenjiBall-CE `docs/ranked-log.md`, "Unranked matches"). */
export const unrankedReasons: Readonly<Record<string, string>> = {
  MAP: "the map isn't Workshop Island Night",
  MODE: "the mode isn't free-for-all",
  PRESET: "the preset isn't Default (Tournament in a tourney match)",
  FEEL: "a ball or player feel toggle is on",
  ADD_ON: "a gameplay add-on is on (duels, endless, sandbox or custom abilities)",
  BOT: "a dummy bot is in the match",
};
