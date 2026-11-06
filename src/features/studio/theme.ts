export type ThemeMode = "dark" | "light" | "contrast";
export type ThemePalette = "slate" | "obsidian" | "graphite" | "sage";

export const DEFAULT_THEME_MODE: ThemeMode = "dark";
export const DEFAULT_THEME_PALETTE: ThemePalette = "slate";

export const VALID_THEME_MODES: readonly ThemeMode[] = ["dark", "light", "contrast"] as const;
export const VALID_THEME_PALETTES: readonly ThemePalette[] = [
  "slate",
  "obsidian",
  "graphite",
  "sage",
] as const;

export interface ThemePaletteDefinition {
  id: ThemePalette;
  label: string;
  description: string;
  swatches: {
    bg: string;
    surface: string;
    accent: string;
    secondary: string;
  };
}

export const THEME_PALETTES: readonly ThemePaletteDefinition[] = [
  {
    id: "slate",
    label: "Slate & Electric Blue",
    description: "Crisp cool-slate surfaces with electric blue accents",
    swatches: {
      bg: "#0f141c",
      surface: "#182130",
      accent: "#4c9aff",
      secondary: "#38bdf8",
    },
  },
  {
    id: "obsidian",
    label: "Obsidian & Indigo",
    description: "Deep neutral carbon surfaces with vibrant indigo-violet",
    swatches: {
      bg: "#101014",
      surface: "#1b1b24",
      accent: "#8b7cff",
      secondary: "#c084fc",
    },
  },
  {
    id: "graphite",
    label: "Graphite & Amber",
    description: "Warm pro-studio graphite with refined amber-copper",
    swatches: {
      bg: "#161514",
      surface: "#23201e",
      accent: "#f0a44c",
      secondary: "#fb923c",
    },
  },
  {
    id: "sage",
    label: "Forest & Sage",
    description: "Calm botanical dark studio with soft sage highlights",
    swatches: {
      bg: "#131815",
      surface: "#1c241f",
      accent: "#8fd4a4",
      secondary: "#6ee7b7",
    },
  },
] as const;

export function normalizeThemeMode(value: unknown): ThemeMode {
  if (value === "light" || value === "contrast" || value === "dark") {
    return value;
  }
  return DEFAULT_THEME_MODE;
}

export function normalizeThemePalette(value: unknown): ThemePalette {
  if (value === "slate" || value === "obsidian" || value === "graphite" || value === "sage") {
    return value;
  }
  return DEFAULT_THEME_PALETTE;
}

export function applyDocumentTheme(theme: ThemeMode, palette: ThemePalette): void {
  if (typeof document === "undefined") return;
  document.documentElement.dataset.theme = theme;
  document.documentElement.dataset.palette = palette;
}
