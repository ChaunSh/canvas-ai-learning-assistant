/**
 * Application settings.
 *
 * Values are applied immediately from localStorage (so the theme never flashes)
 * and mirrored into the local database, which is the source of truth shared by
 * the Windows and Android builds.
 */
import { create } from "zustand";

import { useI18nStore, type Locale } from "@/i18n";
import { api } from "@/services/api";

export type ThemeMode = "light" | "dark" | "system";

export interface AiPrivacy {
  allowImages: boolean;
  allowGrades: boolean;
  keepHistory: boolean;
}

interface SettingsState {
  theme: ThemeMode;
  locale: Locale;
  onboardingDone: boolean;
  privacy: AiPrivacy;
  hydrated: boolean;
  hydrate: () => Promise<void>;
  setTheme: (theme: ThemeMode) => void;
  setLocale: (locale: Locale) => void;
  setPrivacy: (patch: Partial<AiPrivacy>) => void;
  completeOnboarding: () => void;
}

const THEME_KEY = "ui.theme";
const LOCALE_KEY = "ui.locale";
const ONBOARDED_KEY = "app.onboarded";
const PRIVACY_KEY = "ai.privacy";

const DEFAULTS: AiPrivacy = {
  allowImages: true,
  allowGrades: false,
  keepHistory: true,
};

function readLocal<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeLocal(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Storage may be unavailable (private mode); the database mirror still works.
  }
}

export const useSettings = create<SettingsState>((set, get) => ({
  theme: readLocal<ThemeMode>(THEME_KEY, "system"),
  locale: readLocal<Locale>(LOCALE_KEY, "zh"),
  onboardingDone: readLocal<boolean>(ONBOARDED_KEY, false),
  privacy: readLocal<AiPrivacy>(PRIVACY_KEY, DEFAULTS),
  hydrated: false,

  hydrate: async () => {
    // Apply the cached theme/locale first, then reconcile with the database.
    useI18nStore.getState().setLocale(get().locale);
    applyTheme(get().theme);

    try {
      const [theme, locale, onboarded, privacy] = await Promise.all([
        api.settingsGet(THEME_KEY),
        api.settingsGet(LOCALE_KEY),
        api.settingsGet(ONBOARDED_KEY),
        api.settingsGet(PRIVACY_KEY),
      ]);
      const nextTheme = (theme as ThemeMode) ?? get().theme;
      const nextLocale = (locale as Locale) ?? get().locale;
      let nextPrivacy = get().privacy;
      if (privacy) {
        try {
          nextPrivacy = { ...DEFAULTS, ...(JSON.parse(privacy) as Partial<AiPrivacy>) };
        } catch {
          // Ignore malformed values.
        }
      }
      set({
        theme: nextTheme,
        locale: nextLocale,
        onboardingDone: onboarded === "true" || get().onboardingDone,
        privacy: nextPrivacy,
        hydrated: true,
      });
      useI18nStore.getState().setLocale(nextLocale);
      applyTheme(nextTheme);
    } catch {
      set({ hydrated: true });
    }
  },

  setTheme: (theme) => {
    set({ theme });
    writeLocal(THEME_KEY, theme);
    applyTheme(theme);
    void api.settingsSet(THEME_KEY, theme).catch(() => undefined);
  },

  setLocale: (locale) => {
    set({ locale });
    writeLocal(LOCALE_KEY, locale);
    useI18nStore.getState().setLocale(locale);
    void api.settingsSet(LOCALE_KEY, locale).catch(() => undefined);
  },

  setPrivacy: (patch) => {
    const privacy = { ...get().privacy, ...patch };
    set({ privacy });
    writeLocal(PRIVACY_KEY, privacy);
    void api.settingsSet(PRIVACY_KEY, JSON.stringify(privacy)).catch(() => undefined);
  },

  completeOnboarding: () => {
    set({ onboardingDone: true });
    writeLocal(ONBOARDED_KEY, true);
    void api.settingsSet(ONBOARDED_KEY, "true").catch(() => undefined);
  },
}));

/** Apply the theme class to `<html>` and keep `color-scheme` in sync. */
export function applyTheme(mode: ThemeMode) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const prefersDark =
    typeof window !== "undefined" && window.matchMedia
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
      : false;
  const dark = mode === "dark" || (mode === "system" && prefersDark);
  root.classList.toggle("dark", dark);
  root.style.colorScheme = dark ? "dark" : "light";
}

/** Re-apply the theme when the OS preference changes (system mode). */
export function watchSystemTheme(): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => undefined;
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const listener = () => {
    if (useSettings.getState().theme === "system") applyTheme("system");
  };
  media.addEventListener("change", listener);
  return () => media.removeEventListener("change", listener);
}
