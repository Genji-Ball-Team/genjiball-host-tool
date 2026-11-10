/**
 * Toasts (#36): short-lived messages ("Saved", "Copied") in a corner of the window, instead of a
 * status line in the page. Errors stay next to the setting they belong to; one with no setting to
 * stay by (a failed copy) is an error toast, which stays until it's closed. They're the UI library's
 * `<gb-toast>`s, in the page's `<gb-toast-region>`.
 */
import { node } from "./dom";
import "./ui";

export function toast(text: string, detail = "", kind: "success" | "error" = "success"): void {
  const item = document.createElement("gb-toast");
  if (kind === "error") item.kind = kind;
  item.append(node("b", text));
  // The space keeps the two apart in what screen readers hear (`GbToast.message`).
  if (detail) item.append(" ", node("span", detail, "toast-detail"));
  const region = document.querySelector("gb-toast-region") ?? document.body.appendChild(document.createElement("gb-toast-region"));
  region.append(item);
}
