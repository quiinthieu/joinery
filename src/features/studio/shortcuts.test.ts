import { describe, expect, test } from "vitest";
import {
  DEFAULT_SHORTCUTS,
  SHORTCUT_DEFINITIONS,
  effectiveShortcutBadge,
  effectiveShortcutBinding,
  formatShortcutBadge,
  matchesKeyboardEvent,
  normalizeShortcut,
  sanitizeShortcutOverrides,
  shortcutFromKeyboardEvent,
  validateShortcutOverride,
} from "./shortcuts";

describe("shortcuts registry and normalization", () => {
  test("every default shortcut binding normalizes cleanly without conflicts", () => {
    const seen = new Set<string>();
    for (const def of SHORTCUT_DEFINITIONS) {
      const normalized = normalizeShortcut(def.defaultBinding);
      expect(normalized).not.toBeNull();
      expect(normalized?.canonical).toBe(def.defaultBinding);
      expect(seen.has(def.defaultBinding)).toBe(false);
      seen.add(def.defaultBinding);
    }
  });

  test("normalizes Mod/Cmd/Ctrl, glyph strings, and punctuation keys to canonical format and badge", () => {
    expect(normalizeShortcut("mod+k")).toEqual({
      canonical: "Mod+K",
      display: "⌘ K",
      modifiers: { shift: false, alt: false, mod: true },
      key: "K",
    });
    expect(normalizeShortcut("⌘K")).toEqual({
      canonical: "Mod+K",
      display: "⌘ K",
      modifiers: { shift: false, alt: false, mod: true },
      key: "K",
    });
    expect(normalizeShortcut("Cmd+Shift+O")).toEqual({
      canonical: "Shift+Mod+O",
      display: "⇧⌘ O",
      modifiers: { shift: true, alt: false, mod: true },
      key: "O",
    });
    expect(normalizeShortcut("Ctrl+Alt+Shift+w")).toEqual({
      canonical: "Shift+Alt+Mod+W",
      display: "⇧⌥⌘ W",
      modifiers: { shift: true, alt: true, mod: true },
      key: "W",
    });
    expect(normalizeShortcut("Mod+,")).toEqual({
      canonical: "Mod+,",
      display: "⌘ ,",
      modifiers: { shift: false, alt: false, mod: true },
      key: ",",
    });
    expect(normalizeShortcut("F12")).toEqual({
      canonical: "F12",
      display: "F12",
      modifiers: { shift: false, alt: false, mod: false },
      key: "F12",
    });
    expect(formatShortcutBadge("Shift+Mod+P")).toBe("⇧⌘ P");
  });

  test("rejects empty, modifier-only, or multi-key strings", () => {
    expect(normalizeShortcut("")).toBeNull();
    expect(normalizeShortcut("Mod+Shift")).toBeNull();
    expect(normalizeShortcut("Mod+K+L")).toBeNull();
    expect(normalizeShortcut("UnknownKey")).toBeNull();
  });

  test("matches keyboard events including macOS Option-modified key codes", () => {
    expect(
      matchesKeyboardEvent(
        {
          key: "k",
          metaKey: true,
          ctrlKey: false,
          altKey: false,
          shiftKey: false,
        },
        "Mod+K",
      ),
    ).toBe(true);

    expect(
      matchesKeyboardEvent(
        {
          key: "∑",
          code: "KeyW",
          metaKey: true,
          ctrlKey: false,
          altKey: true,
          shiftKey: false,
        },
        "Alt+Mod+W",
      ),
    ).toBe(true);

    expect(
      shortcutFromKeyboardEvent({
        key: "Meta",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
        shiftKey: false,
      }),
    ).toBeNull();
  });

  test("validates overrides against bare keys, reserved chords, and duplicate bindings", () => {
    const bare = validateShortcutOverride("openPalette", "K", {});
    expect(bare.valid).toBe(false);
    expect(bare.error).toMatch(/must include/i);

    const shiftOnly = validateShortcutOverride("openPalette", "Shift+K", {});
    expect(shiftOnly.valid).toBe(false);

    const fnKey = validateShortcutOverride("openPalette", "F6", {});
    expect(fnKey).toEqual({
      valid: true,
      canonical: "F6",
      display: "F6",
    });

    for (const reserved of ["Mod+C", "Mod+V", "Mod+X", "Mod+Z", "Shift+Mod+Z", "Mod+A"]) {
      const res = validateShortcutOverride("openPalette", reserved, {});
      expect(res.valid).toBe(false);
      expect(res.error).toMatch(/reserved/i);
    }

    const conflictDefault = validateShortcutOverride("openPalette", "Mod+T", {});
    expect(conflictDefault.valid).toBe(false);
    expect(conflictDefault.conflictActionId).toBe("newQuery");
    expect(conflictDefault.error).toContain('already assigned to "New query tab"');

    const conflictCustom = validateShortcutOverride("newQuery", "Mod+J", {
      openPalette: "Mod+J",
    });
    expect(conflictCustom.valid).toBe(false);
    expect(conflictCustom.conflictActionId).toBe("openPalette");

    const freedDefault = validateShortcutOverride("newQuery", "Mod+K", {
      openPalette: "Mod+J",
    });
    expect(freedDefault.valid).toBe(true);
    expect(freedDefault.canonical).toBe("Mod+K");
  });

  test("sanitizes persisted shortcut overrides and computes effective badges", () => {
    const sanitized = sanitizeShortcutOverrides({
      openPalette: "Mod+J",
      unknownAction: "Mod+U",
      newQuery: "Mod+C",
      toggleSidebar: "Mod+B",
    });
    expect(sanitized).toEqual({ openPalette: "Mod+J" });
    expect(effectiveShortcutBinding("openPalette", sanitized)).toBe("Mod+J");
    expect(effectiveShortcutBadge("openPalette", sanitized)).toBe("⌘ J");
    expect(effectiveShortcutBinding("newQuery", sanitized)).toBe(DEFAULT_SHORTCUTS.newQuery);
  });
});
