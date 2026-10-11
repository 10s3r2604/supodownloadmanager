import { useEffect, useMemo, useState } from "react";

import type { Settings } from "./api";
import { resolveLanguage, translator } from "./i18n";

/** Applies the chosen look, color mode and language to the page. */
export function useAppearance(settings: Settings | null) {
  const language = resolveLanguage(settings?.language);
  const i18n = useMemo(() => ({ t: translator(language), language }), [language]);
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => setSystemDark(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  const skin = settings?.skin === "neon" ? "neon" : "minimal";
  const theme = settings?.theme ?? "dark";
  // Neon is always dark.
  const dark = skin === "neon" || theme === "dark" || (theme === "system" && systemDark);

  useEffect(() => {
    const root = document.documentElement;
    root.lang = language;
    root.dataset.skin = skin;
    root.dataset.theme = dark ? "dark" : "light";
  }, [language, skin, dark]);

  return { i18n, skin, dark };
}
