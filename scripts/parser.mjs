// The match view parses logs with the ranked server's own parser (genjiball-ranked
// `src/parser/parse.ts` and `types.ts`), copied into `src/parser/` unchanged. This keeps the copy
// the same as genjiball-ranked `main` on GitHub (not a sibling clone, which may be on another
// branch):
//
//   node scripts/parser.mjs check   fails, naming the file, when a copy differs (`npm run check:parser`)
//   node scripts/parser.mjs sync    copies the files in from GitHub (`npm run sync:parser`)
//
// Line endings don't count: Git may check the files out with either.
import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const SOURCE = "https://raw.githubusercontent.com/Genji-Ball-Team/genjiball-ranked/main/src/parser/";
const FILES = ["parse.ts", "types.ts"];
const TARGET = new URL("../src/parser/", import.meta.url);
/** How long GitHub may take to answer, in milliseconds. */
const TIMEOUT_MS = 15_000;

const normalise = (text) => text.replace(/\r\n/g, "\n");

async function upstream(file) {
  const url = SOURCE + file;
  let response;
  try {
    response = await fetch(url, { signal: AbortSignal.timeout(TIMEOUT_MS) });
  } catch (error) {
    throw new Error(`Couldn't fetch ${url} (${error.cause?.message ?? error.message}). The parser check needs the network: try again once you're online.`, { cause: error });
  }
  if (!response.ok) throw new Error(`Couldn't fetch ${url}: GitHub answered ${response.status} ${response.statusText}.`);
  return response.text();
}

async function local(file) {
  try {
    return await readFile(new URL(file, TARGET), "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
}

async function check() {
  const differ = [];
  for (const file of FILES) {
    const [theirs, ours] = await Promise.all([upstream(file), local(file)]);
    if (ours === null || normalise(ours) !== normalise(theirs)) differ.push(file);
  }
  if (differ.length) {
    const paths = differ.map((f) => `src/parser/${f}`).join(", ");
    console.error(
      `${paths} ${differ.length === 1 ? "differs" : "differ"} from genjiball-ranked main (${SOURCE}).\n` +
        "The match view must parse logs exactly as the server does. Don't edit the copy: change the parser in genjiball-ranked first, " +
        "then run `npm run sync:parser` here to copy it over, and update the match view if the types changed.",
    );
    process.exitCode = 1;
    return;
  }
  console.log(`src/parser/ matches genjiball-ranked main (${FILES.join(", ")}).`);
}

async function sync() {
  for (const file of FILES) {
    await writeFile(new URL(file, TARGET), await upstream(file));
    console.log(`Copied ${file} into ${fileURLToPath(new URL(file, TARGET))}`);
  }
}

const commands = { check, sync };
const command = commands[process.argv[2]];
if (!command) {
  console.error("Usage: node scripts/parser.mjs check|sync");
  process.exitCode = 2;
} else {
  command().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
