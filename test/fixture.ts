import { readFileSync } from "node:fs";
import { resolve } from "node:path";

/**
 * The spec's example log (GenjiBall-CE `docs/ranked-log-example.txt`), shared with the Rust tests.
 * From the repo root, where Vitest runs: a DOM test environment has its own `URL`.
 */
export const example = readFileSync(resolve("src-tauri/tests/fixtures/ranked-log-example.txt"), "utf8");
