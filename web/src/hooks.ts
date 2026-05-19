import { useCallback, useEffect, useRef, useState } from 'preact/hooks';

/**
 * Read/write `key=value` pairs in `window.location.hash` so the active scan
 * path and tab survive a page refresh and link sharing.
 */
export function useHashState(key: string, fallback: string): [string, (v: string) => void] {
  const read = () => {
    const params = new URLSearchParams(window.location.hash.replace(/^#/, ''));
    return params.get(key) ?? fallback;
  };
  const [value, setValue] = useState(read);

  useEffect(() => {
    const onHash = () => setValue(read());
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  const update = useCallback(
    (next: string) => {
      const params = new URLSearchParams(window.location.hash.replace(/^#/, ''));
      if (next === fallback) params.delete(key);
      else params.set(key, next);
      const hash = params.toString();
      const href = `${window.location.pathname}${window.location.search}${hash ? `#${hash}` : ''}`;
      window.history.replaceState(null, '', href);
      setValue(next);
    },
    [key, fallback]
  );

  return [value, update];
}

/**
 * LRU list of recently scanned paths, persisted in localStorage.
 */
export function useRecentPaths(key = 'belisarius:recent-paths', max = 6) {
  const read = (): string[] => {
    try {
      const raw = window.localStorage.getItem(key);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? parsed.filter((s) => typeof s === 'string') : [];
    } catch {
      return [];
    }
  };
  const [paths, setPaths] = useState<string[]>(read);

  const remember = useCallback(
    (p: string) => {
      const trimmed = p.trim();
      if (!trimmed) return;
      setPaths((prev) => {
        const next = [trimmed, ...prev.filter((x) => x !== trimmed)].slice(0, max);
        try {
          window.localStorage.setItem(key, JSON.stringify(next));
        } catch {
          /* ignore */
        }
        return next;
      });
    },
    [key, max]
  );

  const forget = useCallback(
    (p: string) => {
      setPaths((prev) => {
        const next = prev.filter((x) => x !== p);
        try {
          window.localStorage.setItem(key, JSON.stringify(next));
        } catch {
          /* ignore */
        }
        return next;
      });
    },
    [key]
  );

  return { paths, remember, forget };
}

/**
 * Fire `handler(key)` when a global key is pressed outside text inputs.
 */
export function useGlobalKeys(handler: (key: string, ev: KeyboardEvent) => void) {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    const fn = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target) {
        const tag = target.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || target.isContentEditable) {
          return;
        }
      }
      ref.current(e.key, e);
    };
    window.addEventListener('keydown', fn);
    return () => window.removeEventListener('keydown', fn);
  }, []);
}
