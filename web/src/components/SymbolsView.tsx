import { useEffect, useState } from 'preact/hooks';
import { SymbolMatch } from '../api';
import {
  useSymbolsCallers,
  useSymbolsRefs,
  useSymbolsSearch,
  useSymbolsStatus,
} from '../data/queries';

export function SymbolsView({
  path,
  onJumpImpact,
}: {
  path: string;
  onJumpImpact?: (sym: SymbolMatch, mode: 'impact' | 'flow' | '360') => void;
}) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<SymbolMatch | null>(null);

  const { data: status, error: statusErr } = useSymbolsStatus(path);

  // Reset selection / query whenever the project path changes so we don't
  // try to render refs for a symbol that doesn't exist in the new index.
  useEffect(() => {
    setSelected(null);
    setQuery('');
  }, [path]);

  // The search hook gates on >=2 chars and on path. Throttling falls out of
  // react-query's internal request dedup — typing fast just queues the
  // latest key, prior in-flight calls are ignored.
  const enabledQuery = status?.exists ? query : '';
  const { data: matches = [], error: matchErr, isFetching: searching } = useSymbolsSearch(
    path,
    enabledQuery,
    50,
  );

  const selSym = selected?.symbol ?? '';
  const { data: refs = null } = useSymbolsRefs(path, selSym);
  const { data: callers = null } = useSymbolsCallers(path, selSym);

  if (statusErr) return <p class="card text-ink-err">{String(statusErr)}</p>;
  if (!status) return <p class="card text-ink-500">Loading symbol index status…</p>;

  // Use `in`-check rather than `status.exists` so TypeScript narrows the
  // discriminated union. `SymbolsStatusResponse` types `exists: boolean`
  // (not literal true/false) so the discriminant alone doesn't help the
  // checker — but the presence of `documents` does.
  if (!('documents' in status)) {
    return (
      <div class="card space-y-3">
        <h2 class="text-sm uppercase tracking-wider text-ink-500">
          Symbol index not built yet
        </h2>
        <p class="text-sm text-ink-400">
          Belisarius needs a SCIP index for this project before it can answer
          symbol-level questions. Build it with:
        </p>
        <pre class="bg-ink-800 p-3 text-xs text-accent-500">
          belisarius index {path}
        </pre>
        <p class="text-xs text-ink-500">
          Indexer requirements (any one is enough):
        </p>
        <ul class="text-xs text-ink-400 list-disc list-inside space-y-1">
          <li><code class="text-accent-500">rust-analyzer scip</code> for Rust (ships via <code>rustup component add rust-analyzer</code>)</li>
          <li><code class="text-accent-500">scip-typescript</code> for TS/JS (<code>npm i -g @sourcegraph/scip-typescript</code>)</li>
          <li><code class="text-accent-500">scip-python</code> / <code>scip-go</code> for the other supported languages</li>
        </ul>
      </div>
    );
  }

  return (
    <div class="space-y-4">
      <div class="card flex flex-wrap items-center gap-3">
        <input
          class="field flex-1 min-w-[240px]"
          placeholder="search symbols (e.g. Registry, HomePage)…"
          value={query}
          onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
        />
        <div class="flex gap-2 text-[10px] text-ink-500">
          <span class="pill">{status.documents} documents</span>
          <span class="pill">{status.symbols.toLocaleString()} symbols</span>
          {searching && <span class="pill">searching…</span>}
        </div>
      </div>

      {matchErr && <p class="card text-ink-err">{String(matchErr)}</p>}

      <div class="grid gap-4 lg:grid-cols-[minmax(280px,_2fr)_3fr]">
        <div class="card max-h-[680px] overflow-auto p-2">
          {matches.length === 0 ? (
            <p class="px-2 py-3 text-sm text-ink-500">
              {query.trim() ? 'no matches' : 'type to search…'}
            </p>
          ) : (
            <ul class="space-y-1">
              {matches.map((m) => {
                const active = selected?.symbol === m.symbol;
                return (
                  <li key={m.symbol}>
                    <button
                      onClick={() => setSelected(m)}
                      class={`w-full px-2 py-1 text-left transition ${
                        active
                          ? 'bg-accent-500/10 text-accent-500'
                          : 'text-ink-300 hover:bg-ink-800'
                      }`}
                    >
                      <div class="flex items-baseline justify-between gap-2">
                        <span class="truncate text-sm font-medium">
                          {m.display_name || stripSymbolTail(m.symbol)}
                        </span>
                        <span class="shrink-0 text-[10px] text-ink-500">{m.occurrences}</span>
                      </div>
                      <div class="truncate text-[10px] text-ink-500">{m.symbol}</div>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div class="card max-h-[680px] overflow-auto">
          {!selected ? (
            <p class="text-sm text-ink-500">Select a symbol to see definitions, references, and callers.</p>
          ) : (
            <div class="space-y-5">
              <header>
                <p class="text-[10px] uppercase tracking-wider text-ink-500">Selected</p>
                <p class="text-sm text-ink-300 break-all">
                  <span class="text-accent-500">{selected.display_name || '(anon)'}</span>
                  <span class="ml-2 text-[10px] text-ink-500">{selected.symbol}</span>
                </p>
                {onJumpImpact && (
                  <div class="mt-2 flex gap-2">
                    <button
                      class="btn text-xs"
                      title="Show 360° view: def + direct callers + direct callees"
                      onClick={() => onJumpImpact(selected, '360')}
                    >
                      360°
                    </button>
                    <button
                      class="btn text-xs"
                      title="Transitive callers — who reaches this symbol?"
                      onClick={() => onJumpImpact(selected, 'impact')}
                    >
                      impact ↓
                    </button>
                    <button
                      class="btn text-xs"
                      title="Transitive callees — what does this symbol reach?"
                      onClick={() => onJumpImpact(selected, 'flow')}
                    >
                      flow ↑
                    </button>
                  </div>
                )}
              </header>

              {refs == null ? (
                <p class="text-xs text-ink-500">Loading references…</p>
              ) : (
                <section>
                  <h3 class="text-xs uppercase tracking-wider text-ink-500">
                    References — {refs.total} across {refs.files} files
                  </h3>
                  <ul class="mt-2 space-y-2 text-xs">
                    {refs.groups.map((g) => (
                      <li key={g.file}>
                        <details open={refs.groups.length <= 3}>
                          <summary class="cursor-pointer text-ink-300">
                            {g.file}{' '}
                            <span class="text-ink-500">({g.refs.length})</span>
                          </summary>
                          <ul class="ml-3 mt-1 space-y-0.5">
                            {g.refs.map((r, i) => (
                              <li key={i} class="text-ink-400">
                                {r.is_definition && (
                                  <span class="mr-1 text-accent-500">def</span>
                                )}
                                {g.file}:{r.start_line + 1}:{r.start_char + 1}
                              </li>
                            ))}
                          </ul>
                        </details>
                      </li>
                    ))}
                  </ul>
                </section>
              )}

              {callers == null ? (
                <p class="text-xs text-ink-500">Loading callers…</p>
              ) : callers.callers.length === 0 ? (
                <section>
                  <h3 class="text-xs uppercase tracking-wider text-ink-500">Callers</h3>
                  <p class="mt-1 text-xs text-ink-500">
                    None found. Indexers must emit{' '}
                    <code class="text-accent-500">enclosing_range</code> on definitions for
                    Belisarius to compute callers. rust-analyzer does;
                    scip-typescript 0.4.0 does not.
                  </p>
                </section>
              ) : (
                <section>
                  <h3 class="text-xs uppercase tracking-wider text-ink-500">
                    Callers — {callers.callers_count}
                  </h3>
                  <ul class="mt-2 space-y-3 text-xs">
                    {callers.callers.map((c) => (
                      <li key={c.symbol}>
                        <div class="text-ink-300">
                          {c.display_name || '(anon)'}{' '}
                          <span class="text-ink-500">({c.call_sites.length})</span>
                        </div>
                        <div class="truncate text-[10px] text-ink-500">{c.symbol}</div>
                        <ul class="ml-3 mt-1 space-y-0.5">
                          {c.call_sites.map((s, i) => (
                            <li key={i} class="text-ink-400">
                              {s.path}:{s.start_line + 1}:{s.start_char + 1}
                            </li>
                          ))}
                        </ul>
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </div>
          )}
        </div>
      </div>

    </div>
  );
}

// `rust-analyzer cargo … prompts/registry/Registry#` → `Registry#`
function stripSymbolTail(sym: string): string {
  const parts = sym.split(/[\s/]/).filter(Boolean);
  return parts[parts.length - 1] ?? sym;
}
