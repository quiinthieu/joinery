import type { KeyboardEvent } from "react";

/** Automatic activation for horizontal tablists; auxiliary buttons keep their keys. */
export function tabNavigation(event: KeyboardEvent<HTMLElement>) {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const current = event.target as HTMLElement;
  if (current.getAttribute("role") !== "tab") return;
  const root = event.currentTarget;
  const tabs = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')].filter(
    (tab) => !tab.disabled && tab.closest('[role="tablist"]') === root,
  );
  const index = tabs.indexOf(current as HTMLButtonElement);
  if (index < 0) return;
  let next: number;
  switch (event.key) {
    case "ArrowRight":
      next = (index + 1) % tabs.length;
      break;
    case "ArrowLeft":
      next = (index - 1 + tabs.length) % tabs.length;
      break;
    case "Home":
      next = 0;
      break;
    case "End":
      next = tabs.length - 1;
      break;
    default:
      return;
  }
  event.preventDefault();
  tabs[next].focus();
  tabs[next].click();
}
