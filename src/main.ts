import { invoke } from "@tauri-apps/api/core";

/** Mirrors `AppInfo` in src-tauri/src/lib.rs. */
interface AppInfo {
  version: string;
  serverUrl: string;
}

function show(id: string, text: string): void {
  const el = document.getElementById(id);
  if (el) el.textContent = text;
}

async function main(): Promise<void> {
  try {
    const info = await invoke<AppInfo>("app_info");
    show("version", info.version);
    show("server", info.serverUrl);
  } catch (err) {
    show("version", `Couldn't reach the app: ${String(err)}`);
  }
}

void main();
