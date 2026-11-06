import { useCallback } from 'react';
import { translate, type Lang, type MessageKey } from './i18n';
import { useApp } from './store';

/** Current UI language. */
export function useLang(): Lang {
  return useApp((state) => state.lang);
}

/**
 * Translator hook.
 *
 * Returns a stable `(key, vars?) => string` bound to the active language, so
 * components re-render when the language changes and nothing else.
 */
export function useT() {
  const lang = useLang();
  return useCallback(
    (key: MessageKey | string, vars?: Record<string, string | number>) => translate(lang, key, vars),
    [lang],
  );
}
