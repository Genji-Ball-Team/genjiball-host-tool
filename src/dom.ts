/** The small DOM helpers the window's views share. */

/** A status line's colour. */
export type Tone = "good" | "bad" | "muted";

/** An error as text: commands fail with strings, the browser with `Error`s. */
export function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** The element with this id, which index.html must have. */
export function el<T extends Element = HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing from index.html`);
  return found as unknown as T;
}

/** A new element with its text and class. */
export function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = "", className = ""): HTMLElementTagNameMap[K] {
  const found = document.createElement(tag);
  if (text) found.textContent = text;
  if (className) found.className = className;
  return found;
}

/** Says `text` on a status line, hidden while there's nothing to say. */
export function setLine(id: string, text: string, tone: Tone): void {
  const line = el(id);
  line.hidden = !text;
  line.textContent = text;
  line.className = tone;
}

/** Fills a `<select>` with these choices, in order. */
export function fillSelect(select: HTMLSelectElement, options: { value: string; label: string }[]): void {
  select.replaceChildren(
    ...options.map((o) => {
      const option = document.createElement("option");
      option.value = o.value;
      option.textContent = o.label;
      return option;
    }),
  );
}
