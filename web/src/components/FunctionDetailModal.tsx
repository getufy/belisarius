import { useEffect, useRef, useState } from 'preact/hooks';
import { api, FunctionDetail } from '../api';

type Props = {
  path: string;
  file: string;
  name: string;
  onClose: () => void;
};

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

let dialogIdCounter = 0;

export function FunctionDetailModal({ path, file, name, onClose }: Props) {
  const [detail, setDetail] = useState<FunctionDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const dialogRef = useRef<HTMLDivElement | null>(null);
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);
  const previouslyFocusedRef = useRef<Element | null>(null);
  const titleIdRef = useRef<string>(`fn-detail-title-${++dialogIdCounter}`);

  useEffect(() => {
    let alive = true;
    api
      .functionDetail(path, file, name)
      .then((d) => alive && setDetail(d))
      .catch((e) => alive && setError(e?.message ?? String(e)));
    return () => {
      alive = false;
    };
  }, [path, file, name]);

  // Cache previously focused element, focus close button on mount,
  // restore focus on unmount.
  useEffect(() => {
    previouslyFocusedRef.current = document.activeElement;
    // Defer focus until DOM is painted.
    const t = window.setTimeout(() => {
      const root = dialogRef.current;
      if (!root) return;
      const focusables = root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
      const first = focusables[0] ?? closeBtnRef.current;
      first?.focus();
    }, 0);
    return () => {
      window.clearTimeout(t);
      const prev = previouslyFocusedRef.current;
      if (prev && prev instanceof HTMLElement) {
        prev.focus();
      }
    };
  }, []);

  // ESC closes; Tab key keeps focus within the modal.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key === 'Tab') {
        const root = dialogRef.current;
        if (!root) return;
        const focusables = Array.from(
          root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
        ).filter((el) => !el.hasAttribute('disabled'));
        if (focusables.length === 0) {
          e.preventDefault();
          return;
        }
        const first = focusables[0];
        const last = focusables[focusables.length - 1];
        const active = document.activeElement as HTMLElement | null;
        if (e.shiftKey) {
          if (active === first || !root.contains(active)) {
            e.preventDefault();
            last.focus();
          }
        } else {
          if (active === last || !root.contains(active)) {
            e.preventDefault();
            first.focus();
          }
        }
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div
      class="fixed inset-0 z-50 bg-black/60 flex items-start justify-center p-6 overflow-auto"
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleIdRef.current}
        class="card max-w-4xl w-full text-sm space-y-4"
        onClick={(e) => e.stopPropagation()}
      >
        <header class="flex items-center justify-between gap-3 pb-2 border-b border-ink-700">
          <div>
            <div class="text-xs uppercase tracking-wider text-ink-500">Function detail</div>
            <div id={titleIdRef.current} class="text-ink-200 font-medium">{name}</div>
            <div class="text-xs text-ink-500">{file}</div>
          </div>
          <button
            ref={closeBtnRef}
            class="text-ink-500 hover:text-ink-200 text-lg leading-none"
            onClick={onClose}
            aria-label="Close"
          >
            ×
          </button>
        </header>

        {error && <div class="text-ink-err">Failed to load: {error}</div>}
        {!detail && !error && <div class="text-ink-500">Loading detail…</div>}

        {detail && (
          <>
            <section class="grid grid-cols-2 md:grid-cols-4 gap-3 text-xs">
              <Stat label="cyclomatic" value={detail.function.cyclomatic} />
              <Stat label="cognitive" value={detail.function.cognitive} />
              <Stat label="loc" value={detail.function.loc} />
              <Stat label="params" value={detail.function.params} />
            </section>

            <section>
              <div class="label">snippet · lines {detail.snippet.start_line}-{detail.snippet.end_line}</div>
              <pre class="bg-ink-900 border border-ink-700 p-2 text-xs overflow-auto max-h-72 whitespace-pre">
                {detail.snippet.text}
              </pre>
            </section>

            {detail.file_metrics && (
              <section>
                <div class="label">file metrics</div>
                <div class="text-xs text-ink-400">
                  {detail.file_metrics.function_count} fns · total cc {detail.file_metrics.total_cyclomatic} ·
                  max cc {detail.file_metrics.max_cyclomatic} · max cog {detail.file_metrics.max_cognitive} ·
                  longest {detail.file_metrics.longest_function_loc} LOC ·
                  avg cc {detail.file_metrics.avg_cyclomatic.toFixed(1)}
                </div>
              </section>
            )}

            <section>
              <div class="label">churn (90-day)</div>
              {detail.churn ? (
                <div class="text-xs text-ink-400">
                  commits in window: {detail.churn.commits_in_window} · lifetime: {detail.churn.total_commits}
                  {detail.churn.last_author && (
                    <> · last author: <span class="text-ink-300">{detail.churn.last_author}</span></>
                  )}
                  {detail.churn.last_edited && (
                    <> · last edited: {detail.churn.last_edited.slice(0, 10)}</>
                  )}
                </div>
              ) : (
                <div class="text-xs text-ink-500">no git history (or file untouched in window)</div>
              )}
            </section>

            <section>
              <div class="label">tests touching this file</div>
              {detail.tests.covered ? (
                <ul class="text-xs text-ink-300 space-y-1">
                  {detail.tests.tests.map((t) => (
                    <li key={t} class="font-mono">{t}</li>
                  ))}
                </ul>
              ) : (
                <div class="text-xs text-ink-warning">No covering tests detected for {file}.</div>
              )}
            </section>

            <section>
              <div class="label">callers (SCIP)</div>
              {!detail.callers.available && (
                <div class="text-xs text-ink-500">{detail.callers.reason}</div>
              )}
              {detail.callers.available && detail.callers.callers.length === 0 && (
                <div class="text-xs text-ink-500">
                  {detail.callers.reason ?? 'No callers found.'}
                </div>
              )}
              {detail.callers.available && detail.callers.callers.length > 0 && (
                <ul class="text-xs text-ink-300 space-y-1">
                  {detail.callers.callers.map((c) => (
                    <li key={c.symbol}>
                      <span class="text-ink-200">{c.display_name || c.symbol}</span>
                      <span class="text-ink-500"> — {c.call_sites.length} call site{c.call_sites.length === 1 ? '' : 's'}</span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </>
        )}
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div class="border border-ink-700 p-2">
      <div class="text-[10px] uppercase tracking-wider text-ink-500">{label}</div>
      <div class="text-ink-200 text-lg">{value}</div>
    </div>
  );
}
