/**
 * The team's UI library (`@genji-ball-team/ui`): the `gb-` components the window uses, defined once
 * here. Its tokens and base styles are imported in main.ts, before style.css.
 */
import "@genji-ball-team/ui/button.js";
import "@genji-ball-team/ui/chip.js";
import "@genji-ball-team/ui/field.js";
import "@genji-ball-team/ui/icon.js";
import "@genji-ball-team/ui/tabs.js";
import "@genji-ball-team/ui/toast.js";
import type { ButtonVariant, GbButton } from "@genji-ball-team/ui/button.js";
import type { GbChip } from "@genji-ball-team/ui/chip.js";
import type { GbField } from "@genji-ball-team/ui/field.js";
import type { GbIcon } from "@genji-ball-team/ui/icon.js";
import type { GbTabs } from "@genji-ball-team/ui/tabs.js";
import type { GbToast, GbToastRegion } from "@genji-ball-team/ui/toast.js";

export type { GbButton, GbChip, GbField, GbTabs, GbToast };

declare global {
  interface HTMLElementTagNameMap {
    "gb-button": GbButton;
    "gb-chip": GbChip;
    "gb-field": GbField;
    "gb-icon": GbIcon;
    "gb-tabs": GbTabs;
    "gb-toast": GbToast;
    "gb-toast-region": GbToastRegion;
  }
}

/** A `<gb-button>` with this label: secondary unless another variant is given. */
export function gbButton(text: string, variant?: ButtonVariant, className = ""): GbButton {
  const button = document.createElement("gb-button");
  button.textContent = text;
  if (variant) button.variant = variant;
  if (className) button.className = className;
  return button;
}

/** A grey `<gb-chip>`, or one in `tone`'s colour (a class in style.css). */
export function gbChip(text: string, tone = "", title = ""): GbChip {
  const chip = document.createElement("gb-chip");
  chip.textContent = text;
  if (tone) chip.className = tone;
  if (title) chip.title = title;
  return chip;
}
