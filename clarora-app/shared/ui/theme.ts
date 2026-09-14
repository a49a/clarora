import { ColorSchemeName } from "react-native";

export type ThemeName = "paper" | "aurora" | "ember" | "plum" | "night";

// ── Paper: ink on parchment (cream / ink-brown / vermilion) ────────────────

const paperLight = {
  bg: "#f7f1e4",
  surface: "#fffcf4",
  sidebar: "#ede2ca",
  sidebarActive: "#fff8e9",
  surfaceHover: "#f1e5cd",
  border: "#e0d1b0",
  text: "#2b241c",
  textSecondary: "#6d6151",
  textMuted: "#9b8f7a",
  sidebarText: "#2b241c",
  sidebarTextMuted: "#7d715d",
  sidebarAccent: "#b8452c",
  onSidebarAccent: "#fff8ec",
  accent: "#b8452c",
  accentHover: "#9a3a24",
  danger: "#a83527",
  cardShadow: {
    shadowColor: "#1a140c",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.08,
    shadowRadius: 12,
    elevation: 4,
  },
  toastBg: "#2b241c",
  toastText: "#fffcf4",
};

const paperDark: typeof paperLight = {
  bg: "#1c1712",
  surface: "#26201a",
  sidebar: "#130f0b",
  sidebarActive: "#2f271e",
  surfaceHover: "#332a20",
  border: "#473b2c",
  text: "#f2e8d6",
  textSecondary: "#bcae95",
  textMuted: "#867a63",
  sidebarText: "#f2e8d6",
  sidebarTextMuted: "#b1a488",
  sidebarAccent: "#e8825e",
  onSidebarAccent: "#1c1712",
  accent: "#e8825e",
  accentHover: "#f29d7c",
  danger: "#ef7a66",
  cardShadow: {
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.32,
    shadowRadius: 16,
    elevation: 6,
  },
  toastBg: "#f2e8d6",
  toastText: "#1c1712",
};

// ── Aurora: deep forest (evergreen / mint) ──────────────────────────────────

const auroraLight: typeof paperLight = {
  bg: "#edf3ee",
  surface: "#fbfdfa",
  sidebar: "#14301f",
  sidebarActive: "#1e4230",
  surfaceHover: "#e6f0e9",
  border: "#cfe0d5",
  text: "#10231a",
  textSecondary: "#4c665a",
  textMuted: "#7b9488",
  sidebarText: "#f0f7f1",
  sidebarTextMuted: "#a7c3b2",
  sidebarAccent: "#5fd6a4",
  onSidebarAccent: "#062318",
  accent: "#059669",
  accentHover: "#047857",
  danger: "#dc2626",
  cardShadow: {
    shadowColor: "#0d2117",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.1,
    shadowRadius: 14,
    elevation: 4,
  },
  toastBg: "#10231a",
  toastText: "#edf3ee",
};

const auroraDark: typeof paperLight = {
  bg: "#0d1512",
  surface: "#13201a",
  sidebar: "#081009",
  sidebarActive: "#1d2f26",
  surfaceHover: "#1a2b22",
  border: "#2b4034",
  text: "#e6f2ea",
  textSecondary: "#a8bfb2",
  textMuted: "#6f8a7c",
  sidebarText: "#eef7f0",
  sidebarTextMuted: "#93ab9e",
  sidebarAccent: "#52e0a2",
  onSidebarAccent: "#062318",
  accent: "#34d399",
  accentHover: "#6ee7b7",
  danger: "#f87171",
  cardShadow: {
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.4,
    shadowRadius: 18,
    elevation: 6,
  },
  toastBg: "#e6f2ea",
  toastText: "#0d1512",
};

// ── Ember: warm sandstone (ochre / terracotta / charcoal) ──────────────────

const emberLight: typeof paperLight = {
  bg: "#f6efe3",
  surface: "#fffaf2",
  sidebar: "#34251e",
  sidebarActive: "#4a3428",
  surfaceHover: "#f1e2cf",
  border: "#decab1",
  text: "#34251e",
  textSecondary: "#725a49",
  textMuted: "#9b806a",
  sidebarText: "#fff5e8",
  sidebarTextMuted: "#d9bda0",
  sidebarAccent: "#f2ae4b",
  onSidebarAccent: "#34251e",
  accent: "#b9532c",
  accentHover: "#963f22",
  danger: "#b83e32",
  cardShadow: {
    shadowColor: "#34251e",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.1,
    shadowRadius: 14,
    elevation: 4,
  },
  toastBg: "#34251e",
  toastText: "#fff5e8",
};

const emberDark: typeof paperLight = {
  bg: "#211916",
  surface: "#2c211c",
  sidebar: "#17100d",
  sidebarActive: "#3a2921",
  surfaceHover: "#382a22",
  border: "#554137",
  text: "#f8ecdc",
  textSecondary: "#d4bda9",
  textMuted: "#a98d79",
  sidebarText: "#fff3e3",
  sidebarTextMuted: "#d8bca2",
  sidebarAccent: "#f4b45b",
  onSidebarAccent: "#2c1d15",
  accent: "#ef8f43",
  accentHover: "#f5ad65",
  danger: "#f17b67",
  cardShadow: {
    shadowColor: "#000000",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.38,
    shadowRadius: 18,
    elevation: 6,
  },
  toastBg: "#f8ecdc",
  toastText: "#211916",
};

// ── Plum: warm porcelain / mulberry / charcoal ────────────────────────────

const plumLight: typeof paperLight = {
  bg: "#f4efef",
  surface: "#fffbfa",
  sidebar: "#32232e",
  sidebarActive: "#4b3544",
  surfaceHover: "#eee3e9",
  border: "#ddcdd5",
  text: "#30252d",
  textSecondary: "#715d69",
  textMuted: "#87737e",
  sidebarText: "#fff5f8",
  sidebarTextMuted: "#d1b8c6",
  sidebarAccent: "#e6adc9",
  onSidebarAccent: "#32232e",
  accent: "#8c406d",
  accentHover: "#743359",
  danger: "#b53e45",
  cardShadow: {
    shadowColor: "#32232e",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.07,
    shadowRadius: 14,
    elevation: 4,
  },
  toastBg: "#32232e",
  toastText: "#fff5f8",
};

const plumDark: typeof paperLight = {
  bg: "#181216",
  surface: "#21191e",
  sidebar: "#120d10",
  sidebarActive: "#392831",
  surfaceHover: "#30232b",
  border: "#4d3945",
  text: "#f8edf2",
  textSecondary: "#cdb7c3",
  textMuted: "#a58c9a",
  sidebarText: "#fff3f8",
  sidebarTextMuted: "#c6aab9",
  sidebarAccent: "#e6adc9",
  onSidebarAccent: "#32232e",
  accent: "#a75b88",
  accentHover: "#a95d8a",
  danger: "#f18c91",
  cardShadow: {
    shadowColor: "#000000",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.32,
    shadowRadius: 18,
    elevation: 6,
  },
  toastBg: "#f8edf2",
  toastText: "#21191e",
};

export type Theme = typeof paperLight;

// A dedicated night palette, independent of the system appearance.
const night: Theme = {
  bg: "#141a23",
  surface: "#1c2430",
  sidebar: "#10151d",
  sidebarActive: "#29364a",
  surfaceHover: "#293447",
  border: "#3b485d",
  text: "#e3e7ee",
  textSecondary: "#b6c0cf",
  textMuted: "#98a6bb",
  sidebarText: "#e3e7ee",
  sidebarTextMuted: "#a6b3c6",
  sidebarAccent: "#a5bbed",
  onSidebarAccent: "#141a23",
  accent: "#7089c7",
  accentHover: "#829bd9",
  danger: "#ed9696",
  cardShadow: {
    shadowColor: "#000000",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.18,
    shadowRadius: 12,
    elevation: 4,
  },
  toastBg: "#303d52",
  toastText: "#e3e7ee",
};

const THEMES: Record<ThemeName, { light: Theme; dark: Theme }> = {
  paper: { light: paperLight, dark: paperDark },
  aurora: { light: auroraLight, dark: auroraDark },
  ember: { light: emberLight, dark: emberDark },
  plum: { light: plumLight, dark: plumDark },
  night: { light: night, dark: night },
};

export const THEME_ORDER: ThemeName[] = ["paper", "aurora", "ember", "plum", "night"];

export function isThemeName(value: unknown): value is ThemeName {
  return THEME_ORDER.some((name) => name === value);
}

export const THEME_LABELS: Record<ThemeName, string> = {
  paper: "纸张",
  aurora: "深林",
  ember: "余烬",
  plum: "暮紫",
  night: "夜航",
};

export const THEME_DETAILS: Record<ThemeName, string> = {
  paper: "Ink on parchment",
  aurora: "Deep forest",
  ember: "Warm sandstone",
  plum: "暖灰 · 梅紫",
  night: "蓝灰 · 固定深色",
};

export function getTheme(name: ThemeName, scheme: ColorSchemeName): Theme {
  return scheme === "dark" ? THEMES[name].dark : THEMES[name].light;
}
