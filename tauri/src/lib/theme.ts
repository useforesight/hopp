import { useSyncExternalStore } from "react";
import { emit, listen } from "@tauri-apps/api/event";

export type ThemePreference = "system" | "light" | "dark";

// The inline script in each window's html reads this key too, so the first paint has the right theme.
const THEME_STORAGE_KEY = "hopp-theme";
const THEME_CHANGED_EVENT = "hopp_theme_changed";

const systemDarkQuery = window.matchMedia("(prefers-color-scheme: dark)");
const preferenceListeners = new Set<() => void>();

const readStoredPreference = (): ThemePreference => {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    return stored === "light" || stored === "dark" ? stored : "system";
  } catch {
    // This runs on load, and no theme is worth a blank incoming call card
    return "system";
  }
};

let themePreference = readStoredPreference();

const applyTheme = () => {
  const isDark = themePreference === "dark" || (themePreference === "system" && systemDarkQuery.matches);
  document.documentElement.classList.toggle("dark", isDark);
};

const updatePreference = (preference: ThemePreference) => {
  themePreference = preference;
  applyTheme();
  preferenceListeners.forEach((notify) => notify());
};

/**
 * Applies the saved theme to this window and keeps it in step with the system appearance and
 * with changes made from other windows. Call it once at each window's entry point.
 */
export const initTheme = () => {
  applyTheme();
  systemDarkQuery.addEventListener("change", applyTheme);
  void listen<ThemePreference>(THEME_CHANGED_EVENT, (event) => updatePreference(event.payload));
};

export const setThemePreference = (preference: ThemePreference) => {
  localStorage.setItem(THEME_STORAGE_KEY, preference);
  updatePreference(preference);
  void emit(THEME_CHANGED_EVENT, preference);
};

export const useThemePreference = () =>
  useSyncExternalStore(
    (notify) => {
      preferenceListeners.add(notify);
      return () => preferenceListeners.delete(notify);
    },
    () => themePreference,
  );

export const toastStyle = {
  background: "var(--raised)",
  color: "var(--foreground)",
  border: "1px solid var(--border-strong)",
  borderRadius: 12,
  boxShadow: "var(--pop-shadow)",
  fontSize: 13,
  lineHeight: "18px",
  padding: "6px 10px",
};
