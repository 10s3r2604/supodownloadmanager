import { createContext, useContext } from "react";

import { type Language, type Translate, translator } from "./i18n";

export interface I18n {
  t: Translate;
  language: Language;
}

export const I18nContext = createContext<I18n>({ t: translator("en"), language: "en" });

export const useI18n = () => useContext(I18nContext);

/** Copies text, falling back to a hidden text field where the clipboard API is blocked. */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const field = document.createElement("textarea");
    field.value = text;
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.append(field);
    field.select();
    document.execCommand("copy");
    field.remove();
  }
}

export function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}
