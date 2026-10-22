/**
 * Toasts (#36): short-lived messages ("Saved", "Copied") in a corner of the window, instead of a
 * status line in the page. Errors stay next to the setting they belong to.
 */
import { el, node } from "./dom";

/** How long a toast stays, in milliseconds; one with a detail stays longer. */
const TOAST_MS = 3000;
const TOAST_DETAIL_MS = 6000;

export function toast(text: string, detail = ""): void {
  const item = node("div", "", "toast");
  item.append(node("b", text));
  if (detail) item.append(node("span", detail));
  el("toasts").append(item);
  const remove = () => item.remove();
  let timer = setTimeout(remove, detail ? TOAST_DETAIL_MS : TOAST_MS);
  // Kept while the pointer is on it, so a detail can be read.
  item.addEventListener("pointerenter", () => clearTimeout(timer));
  item.addEventListener("pointerleave", () => (timer = setTimeout(remove, TOAST_MS)));
  item.addEventListener("click", remove);
}
