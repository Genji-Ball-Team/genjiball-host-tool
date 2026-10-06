import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["node_modules/", "dist/", "src-tauri/"] },
  js.configs.recommended,
  tseslint.configs.recommended,
  // Plain Node scripts (`scripts/`): the Node globals they use.
  {
    files: ["scripts/**/*.mjs"],
    languageOptions: {
      globals: { AbortSignal: "readonly", URL: "readonly", console: "readonly", fetch: "readonly", process: "readonly" },
    },
  },
);
