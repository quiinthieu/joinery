export type ShortcutActionId =
  | "openPalette"
  | "searchObjects"
  | "recentLocations"
  | "newQuery"
  | "openSqlFile"
  | "saveSqlFile"
  | "saveSqlFileAs"
  | "toggleBookmark"
  | "preferences"
  | "closeActiveTab"
  | "closeOtherTabs"
  | "closeCleanTabs"
  | "reopenClosedTab"
  | "togglePinTab"
  | "toggleSidebar"
  | "toggleInspector";

export type ShortcutCategory = "Navigation" | "Workspace" | "SQL & Files" | "Tabs" | "Layout";

export type ShortcutScope = "global" | "activeTab" | "queryTab";

export interface ShortcutDefinition {
  id: ShortcutActionId;
  label: string;
  category: ShortcutCategory;
  defaultBinding: string;
  scope: ShortcutScope;
}

export interface NormalizedShortcut {
  canonical: string;
  display: string;
  modifiers: {
    shift: boolean;
    alt: boolean;
    mod: boolean;
  };
  key: string;
}

export interface ShortcutValidationResult {
  valid: boolean;
  canonical?: string;
  display?: string;
  conflictActionId?: ShortcutActionId;
  error?: string;
}

export const MAX_SHORTCUT_OVERRIDES = 32;
export const MAX_SHORTCUT_CHORD_LENGTH = 32;

export const SHORTCUT_DEFINITIONS: readonly ShortcutDefinition[] = [
  {
    id: "openPalette",
    label: "Open command palette",
    category: "Navigation",
    defaultBinding: "Mod+K",
    scope: "global",
  },
  {
    id: "searchObjects",
    label: "Search database objects",
    category: "Navigation",
    defaultBinding: "Shift+Mod+O",
    scope: "global",
  },
  {
    id: "recentLocations",
    label: "Recent locations",
    category: "Navigation",
    defaultBinding: "Shift+Mod+E",
    scope: "global",
  },
  {
    id: "toggleBookmark",
    label: "Toggle bookmark",
    category: "Navigation",
    defaultBinding: "Mod+D",
    scope: "activeTab",
  },
  {
    id: "newQuery",
    label: "New query tab",
    category: "Workspace",
    defaultBinding: "Mod+T",
    scope: "global",
  },
  {
    id: "preferences",
    label: "Preferences",
    category: "Workspace",
    defaultBinding: "Mod+,",
    scope: "global",
  },
  {
    id: "openSqlFile",
    label: "Open SQL file",
    category: "SQL & Files",
    defaultBinding: "Mod+O",
    scope: "global",
  },
  {
    id: "saveSqlFile",
    label: "Save SQL file",
    category: "SQL & Files",
    defaultBinding: "Mod+S",
    scope: "queryTab",
  },
  {
    id: "saveSqlFileAs",
    label: "Save SQL file as",
    category: "SQL & Files",
    defaultBinding: "Shift+Mod+S",
    scope: "queryTab",
  },
  {
    id: "closeActiveTab",
    label: "Close active tab",
    category: "Tabs",
    defaultBinding: "Mod+W",
    scope: "activeTab",
  },
  {
    id: "closeOtherTabs",
    label: "Close other tabs",
    category: "Tabs",
    defaultBinding: "Alt+Mod+W",
    scope: "activeTab",
  },
  {
    id: "closeCleanTabs",
    label: "Close unmodified tabs",
    category: "Tabs",
    defaultBinding: "Shift+Alt+Mod+W",
    scope: "global",
  },
  {
    id: "reopenClosedTab",
    label: "Reopen closed tab",
    category: "Tabs",
    defaultBinding: "Shift+Mod+T",
    scope: "global",
  },
  {
    id: "togglePinTab",
    label: "Pin or unpin active tab",
    category: "Tabs",
    defaultBinding: "Shift+Mod+P",
    scope: "activeTab",
  },
  {
    id: "toggleSidebar",
    label: "Toggle left sidebar",
    category: "Layout",
    defaultBinding: "Mod+B",
    scope: "global",
  },
  {
    id: "toggleInspector",
    label: "Toggle right inspector",
    category: "Layout",
    defaultBinding: "Mod+I",
    scope: "global",
  },
];

export const SHORTCUT_BY_ID: Readonly<Record<ShortcutActionId, ShortcutDefinition>> =
  Object.fromEntries(SHORTCUT_DEFINITIONS.map((def) => [def.id, def])) as Record<
    ShortcutActionId,
    ShortcutDefinition
  >;

export const DEFAULT_SHORTCUTS: Readonly<Record<ShortcutActionId, string>> = Object.fromEntries(
  SHORTCUT_DEFINITIONS.map((def) => [def.id, def.defaultBinding]),
) as Record<ShortcutActionId, string>;

const RESERVED_CHORDS: Readonly<Record<string, string>> = {
  "Mod+C": "Copy (⌘ C) is reserved for text and grid copying.",
  "Mod+V": "Paste (⌘ V) is reserved for text editing.",
  "Mod+X": "Cut (⌘ X) is reserved for text editing.",
  "Mod+Z": "Undo (⌘ Z) is reserved for text editing.",
  "Shift+Mod+Z": "Redo (⇧⌘ Z) is reserved for text editing.",
  "Mod+Y": "Redo (⌘ Y) is reserved for text editing.",
  "Mod+A": "Select all (⌘ A) is reserved for text and grid selection.",
  "Mod+Q": "Quit (⌘ Q) is reserved by the operating system.",
  "Mod+R": "Reload (⌘ R) is reserved by the application shell.",
};

const SPECIAL_KEY_ALIASES: Readonly<Record<string, { canonical: string; display: string }>> = {
  ",": { canonical: ",", display: "," },
  comma: { canonical: ",", display: "," },
  ".": { canonical: ".", display: "." },
  period: { canonical: ".", display: "." },
  "/": { canonical: "/", display: "/" },
  slash: { canonical: "/", display: "/" },
  ";": { canonical: ";", display: ";" },
  semicolon: { canonical: ";", display: ";" },
  "'": { canonical: "'", display: "'" },
  quote: { canonical: "'", display: "'" },
  "[": { canonical: "[", display: "[" },
  bracketleft: { canonical: "[", display: "[" },
  "]": { canonical: "]", display: "]" },
  bracketright: { canonical: "]", display: "]" },
  "\\": { canonical: "\\", display: "\\" },
  backslash: { canonical: "\\", display: "\\" },
  "-": { canonical: "-", display: "-" },
  minus: { canonical: "-", display: "-" },
  "=": { canonical: "=", display: "=" },
  equal: { canonical: "=", display: "=" },
  "`": { canonical: "`", display: "`" },
  backquote: { canonical: "`", display: "`" },
  enter: { canonical: "Enter", display: "↩" },
  return: { canonical: "Enter", display: "↩" },
  escape: { canonical: "Escape", display: "Esc" },
  esc: { canonical: "Escape", display: "Esc" },
  tab: { canonical: "Tab", display: "⇥" },
  space: { canonical: "Space", display: "Space" },
  backspace: { canonical: "Backspace", display: "⌫" },
  delete: { canonical: "Delete", display: "⌦" },
  arrowup: { canonical: "ArrowUp", display: "↑" },
  up: { canonical: "ArrowUp", display: "↑" },
  "↑": { canonical: "ArrowUp", display: "↑" },
  arrowdown: { canonical: "ArrowDown", display: "↓" },
  down: { canonical: "ArrowDown", display: "↓" },
  "↓": { canonical: "ArrowDown", display: "↓" },
  arrowleft: { canonical: "ArrowLeft", display: "←" },
  left: { canonical: "ArrowLeft", display: "←" },
  "←": { canonical: "ArrowLeft", display: "←" },
  arrowright: { canonical: "ArrowRight", display: "→" },
  right: { canonical: "ArrowRight", display: "→" },
  "→": { canonical: "ArrowRight", display: "→" },
};

function normalizeKeyToken(raw: string): { canonical: string; display: string } | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const lower = trimmed.toLowerCase();
  if (SPECIAL_KEY_ALIASES[lower]) {
    return SPECIAL_KEY_ALIASES[lower];
  }
  if (/^[a-z]$/i.test(trimmed)) {
    const upper = trimmed.toUpperCase();
    return { canonical: upper, display: upper };
  }
  if (/^[0-9]$/.test(trimmed)) {
    return { canonical: trimmed, display: trimmed };
  }
  if (/^f([1-9]|1[0-2])$/i.test(trimmed)) {
    const upper = trimmed.toUpperCase();
    return { canonical: upper, display: upper };
  }
  return null;
}

export function isFunctionKey(canonicalKey: string): boolean {
  return /^F([1-9]|1[0-2])$/.test(canonicalKey);
}

/**
 * Normalizes a shortcut string such as `"Mod+K"`, `"⌘K"`, `"⌘ K"`, `"Cmd+Shift+O"`,
 * `"Ctrl+Alt+W"`, or `"⇧⌥⌘ W"` into a canonical representation (`Shift+Alt+Mod+<Key>`)
 * and Joinery's display badge (`"⇧⌥⌘ W"`).
 */
export function normalizeShortcut(input: string): NormalizedShortcut | null {
  if (typeof input !== "string") return null;
  const trimmed = input.trim();
  if (!trimmed || trimmed.length > MAX_SHORTCUT_CHORD_LENGTH) return null;

  let shift = false;
  let alt = false;
  let mod = false;

  const expanded = trimmed
    .replace(/⇧/g, " Shift+ ")
    .replace(/⌥/g, " Alt+ ")
    .replace(/⌘/g, " Mod+ ")
    .replace(/⌃/g, " Mod+ ");

  const rawTokens = expanded
    .split(/(?:\+|\s)+/)
    .map((part) => part.trim())
    .filter(Boolean);

  if (rawTokens.length === 0) return null;

  let keyToken: { canonical: string; display: string } | null = null;

  for (const token of rawTokens) {
    const lower = token.toLowerCase();
    if (lower === "shift") {
      shift = true;
      continue;
    }
    if (lower === "alt" || lower === "option" || lower === "opt") {
      alt = true;
      continue;
    }
    if (
      lower === "mod" ||
      lower === "cmd" ||
      lower === "command" ||
      lower === "meta" ||
      lower === "ctrl" ||
      lower === "control" ||
      lower === "cmdorctrl"
    ) {
      mod = true;
      continue;
    }
    if (keyToken !== null) {
      return null;
    }
    const parsedKey = normalizeKeyToken(token);
    if (!parsedKey) return null;
    keyToken = parsedKey;
  }

  if (!keyToken) return null;

  const canonicalParts: string[] = [];
  if (shift) canonicalParts.push("Shift");
  if (alt) canonicalParts.push("Alt");
  if (mod) canonicalParts.push("Mod");
  canonicalParts.push(keyToken.canonical);
  const canonical = canonicalParts.join("+");

  const glyphPrefix = `${shift ? "⇧" : ""}${alt ? "⌥" : ""}${mod ? "⌘" : ""}`;
  const display = glyphPrefix ? `${glyphPrefix} ${keyToken.display}` : keyToken.display;

  return {
    canonical,
    display,
    modifiers: { shift, alt, mod },
    key: keyToken.canonical,
  };
}

export function formatShortcutBadge(binding: string): string {
  return normalizeShortcut(binding)?.display ?? binding;
}

export function isShortcutActionId(value: string): value is ShortcutActionId {
  return Object.prototype.hasOwnProperty.call(SHORTCUT_BY_ID, value);
}

export function effectiveShortcutBinding(
  actionId: ShortcutActionId,
  overrides?: Readonly<Record<string, string>> | null,
): string {
  const candidate = overrides?.[actionId];
  if (candidate) {
    const normalized = normalizeShortcut(candidate);
    if (normalized) return normalized.canonical;
  }
  return SHORTCUT_BY_ID[actionId].defaultBinding;
}

export function effectiveShortcutBadge(
  actionId: ShortcutActionId,
  overrides?: Readonly<Record<string, string>> | null,
): string {
  const binding = effectiveShortcutBinding(actionId, overrides);
  return formatShortcutBadge(binding);
}

export function validateShortcutOverride(
  actionId: ShortcutActionId,
  binding: string,
  overrides?: Readonly<Record<string, string>> | null,
): ShortcutValidationResult {
  const normalized = normalizeShortcut(binding);
  if (!normalized) {
    return {
      valid: false,
      error: "Enter a valid shortcut chord such as Mod+K, Shift+Mod+O, or F1–F12.",
    };
  }

  const { canonical, display, modifiers, key } = normalized;

  if (!modifiers.mod && !modifiers.alt && !isFunctionKey(key)) {
    return {
      valid: false,
      error: "Shortcuts must include ⌘/Ctrl or ⌥/Alt (unless using F1–F12).",
    };
  }

  const reservedMessage = RESERVED_CHORDS[canonical];
  if (reservedMessage) {
    return {
      valid: false,
      error: reservedMessage,
    };
  }

  for (const definition of SHORTCUT_DEFINITIONS) {
    if (definition.id === actionId) continue;
    const otherBinding = effectiveShortcutBinding(definition.id, overrides);
    if (otherBinding === canonical) {
      return {
        valid: false,
        conflictActionId: definition.id,
        error: `${display} is already assigned to "${definition.label}".`,
      };
    }
  }

  return {
    valid: true,
    canonical,
    display,
  };
}

export function sanitizeShortcutOverrides(
  raw?: Readonly<Record<string, string>> | null,
): Record<string, string> {
  if (!raw || typeof raw !== "object") return {};
  const sanitized: Record<string, string> = {};
  for (const [key, value] of Object.entries(raw)) {
    if (!isShortcutActionId(key) || typeof value !== "string") continue;
    const validation = validateShortcutOverride(key, value, sanitized);
    if (!validation.valid || !validation.canonical) continue;
    if (validation.canonical === SHORTCUT_BY_ID[key].defaultBinding) continue;
    sanitized[key] = validation.canonical;
    if (Object.keys(sanitized).length >= MAX_SHORTCUT_OVERRIDES) break;
  }
  return sanitized;
}

const MODIFIER_ONLY_KEYS = new Set(["Meta", "Control", "Shift", "Alt", "OS", "CapsLock"]);

export type ShortcutKeyboardEventLike = Pick<
  KeyboardEvent,
  "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey"
> & {
  code?: string;
};

function keyTokenFromKeyboardEvent(
  event: Pick<ShortcutKeyboardEventLike, "key" | "code">,
): { canonical: string; display: string } | null {
  if (MODIFIER_ONLY_KEYS.has(event.key)) return null;
  const direct = normalizeKeyToken(event.key);
  if (direct) return direct;
  if (event.code) {
    if (/^Key([A-Z])$/.test(event.code)) {
      const letter = event.code.slice(3);
      return { canonical: letter, display: letter };
    }
    if (/^Digit([0-9])$/.test(event.code)) {
      const digit = event.code.slice(5);
      return { canonical: digit, display: digit };
    }
    const byCode = normalizeKeyToken(event.code);
    if (byCode) return byCode;
  }
  return null;
}

export function shortcutFromKeyboardEvent(
  event: ShortcutKeyboardEventLike,
): NormalizedShortcut | null {
  const keyToken = keyTokenFromKeyboardEvent(event);
  if (!keyToken) return null;
  const parts: string[] = [];
  if (event.shiftKey) parts.push("Shift");
  if (event.altKey) parts.push("Alt");
  if (event.metaKey || event.ctrlKey) parts.push("Mod");
  parts.push(keyToken.canonical);
  return normalizeShortcut(parts.join("+"));
}

export function matchesKeyboardEvent(event: ShortcutKeyboardEventLike, binding: string): boolean {
  const expected = normalizeShortcut(binding);
  if (!expected) return false;
  const actual = shortcutFromKeyboardEvent(event);
  if (!actual) return false;
  return actual.canonical === expected.canonical;
}
