import React, { createContext, useCallback, useContext, useEffect, useState } from "react";
import { ColorSchemeName, useColorScheme } from "react-native";
import { getTheme, isThemeName, Theme, ThemeName } from "./theme";
import { getSetting, setSetting } from "../data/database";

const THEME_SETTING_KEY = "ui_theme";

type ThemeContextValue = {
  theme: Theme;
  themeName: ThemeName;
  scheme: ColorSchemeName;
  systemScheme: ColorSchemeName;
  setThemeName: (name: ThemeName) => void;
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const systemScheme = useColorScheme();
  const [themeName, setThemeNameState] = useState<ThemeName>("paper");

  // Restore the persisted theme choice on mount.
  useEffect(() => {
    getSetting(THEME_SETTING_KEY)
      .then((value) => {
        if (isThemeName(value)) setThemeNameState(value);
      })
      .catch(() => {
        // database not ready — keep default
      });
  }, []);

  const setThemeName = useCallback((name: ThemeName) => {
    setThemeNameState(name);
    setSetting(THEME_SETTING_KEY, name).catch(() => {
      // persistence is best-effort
    });
  }, []);

  const scheme = themeName === "night" ? "dark" : systemScheme;
  const theme = getTheme(themeName, scheme);

  return (
    <ThemeContext.Provider value={{ theme, themeName, scheme, systemScheme, setThemeName }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useAppTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useAppTheme must be used within ThemeProvider");
  return ctx;
}
