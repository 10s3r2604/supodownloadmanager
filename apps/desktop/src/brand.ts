import wordmarkSvg from "./assets/wordmark.svg?raw";
import logoSmallUrl from "./assets/logo-small.svg";

/** The "supo" wordmark with the letters in the given color. */
export function wordmark(color: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(wordmarkSvg.replace("#0b3b36", color))}`;
}

export const logoSmall = logoSmallUrl;
