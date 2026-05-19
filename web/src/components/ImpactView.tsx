import { useEffect, useState } from 'preact/hooks';
import { FlowReport, ImpactReport, Symbol360, SymbolMatch } from '../api';
import { useFlow, useImpact, useSymbol360, useSymbolsSearch } from '../data/queries';

type Mode = 'impact' | 'flow' | '360';

export function ImpactView({
  path,
  initialSymbol,
  initialMode,
}: {
  path: string;
  initialSymbol?: SymbolMatch | null;
  initialMode?: Mode;
}) {
  const [query, setQuery] = useState(initialSymbol?.display_name ?? initialSymbol?.symbol ?? '');
  const [selected, setSelected] = useState<SymbolMatch | null>(initialSymbol ?? null);
  const [mode, setMode] = useState<Mode>(initialMode ?? 'impact');
  const [depth, setDepth] = useState(3);

  // Re-sync if the parent hands us a new symbol while this view is mounted.
  useEffect(() => {
    if (initialSymbol) {
      setSelected(initialSymbol);
      setQuery(initialSymbol.display_name || initialSymbol.symbol);
    }
    if (initialMode) setMode(initialMode);
  }, [initialSymbol?.symbol, initialMode]);

  // Search-as-you-type — gated to ≥2 chars inside the hook, debounced
  // implicitly by react-query's request dedup.
  const { data: candidates = [] } = useSymbolsSearch(path, query.trim(), 30);

  // Each mode hook is conditional on `enabled` — only the active one
  // actually fires.
  const sym = selected?.symbol ?? '';
  const impactQ = useImpact(mode === 'impact' ? path : '', sym, depth);
  const flowQ = useFlow(mode === 'flow' ? path : '', sym, depth);
  const sym360Q = useSymbol360(mode === '360' ? path : '', sym);
  const impact = mode === 'impact' ? impactQ.data ?? null : null;
  const flow = mode === 'flow' ? flowQ.data ?? null : null;
  const sym360 = mode === '360' ? sym360Q.data ?? null : null;
  const busy = (mode === 'impact' && impactQ.isFetching)
    || (mode === 'flow' && flowQ.isFetching)
    || (mode === '360' && sym360Q.isFetching);
  const err = (mode === 'impact' && impactQ.error)
    || (mode === 'flow' && flowQ.error)
    || (mode === '360' && sym360Q.error)
    || null;
  const errStr = err ? (err instanceof Error ? err.message : String(err)) : null;

  return (
    <div class="space-y-4">
      <div class="card space-y-3">
        <div>
          <label class="label">Symbol</label>
          <input
            class="field"
            placeholder="search symbol by name (e.g. analyze, parseScipIndex)"
            value={query}
            onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
          />
          {candidates.length > 0 && (
            <ul class="mt-1 max-h-44 overflow-auto border border-ink-700 bg-ink-900 text-sm">
              {candidates.slice(0, 30).map((c) => (
                <li
                  key={c.symbol}
                  class={`px-3 py-1 cursor-pointer hover:bg-ink-800 ${selected?.symbol === c.symbol ? 'bg-ink-800' : ''}`}
                  onClick={() => setSelected(c)}
                >
                  <span class="text-accent-500">{c.display_name || c.symbol}</span>{' '}
                  <span class="text-ink-500 text-[10px]">{c.occurrences} occ</span>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div class="flex flex-wrap items-center gap-3">
          <div class="flex items-center gap-2">
            <span class="text-[10px] uppercase tracking-wider text-ink-500">mode</span>
            {(['impact', 'flow', '360'] as Mode[]).map((m) => (
              <button
                key={m}
                class={`btn text-xs ${mode === m ? 'btn-primary' : ''}`}
                onClick={() => setMode(m)}
              >
                {m}
              </button>
            ))}
          </div>
          {mode !== '360' && (
            <div class="flex items-center gap-2">
              <span class="text-[10px] uppercase tracking-wider text-ink-500">depth</span>
              <input
                type="number"
                min={1}
                max={8}
                value={depth}
                class="field w-16"
                onInput={(e) => setDepth(Math.min(8, Math.max(1, Number((e.target as HTMLInputElement).value) || 1)))}
              />
            </div>
          )}
          {selected && (
            <span class="text-[11px] text-ink-500 truncate flex-1 text-right">
              {selected.display_name || selected.symbol}
            </span>
          )}
        </div>
      </div>

      {busy && <p class="card text-ink-400">computing {mode}…</p>}
      {errStr && (
        errStr.toLowerCase().includes('symbol index')
          ? <ScipMissingCard path={path} />
          : <p class="card text-ink-err">{errStr}</p>
      )}

      {!busy && impact && <ImpactCard report={impact} />}
      {!busy && flow && <FlowCard report={flow} />}
      {!busy && sym360 && <Sym360Card view={sym360} />}
    </div>
  );
}

/// Rendered when Impact / Flow / 360 hit a missing SCIP index. Spells out
/// the disambiguation between this index and the hybrid-search one, and
/// lists the per-language SCIP tools the user needs.
function ScipMissingCard({ path }: { path: string }) {
  return (
    <div class="card space-y-3">
      <h2 class="text-sm uppercase tracking-wider text-ink-500">
        SCIP symbol index not built yet
      </h2>
      <p class="text-sm text-ink-400">
        Impact / Flow / Symbol 360° need a <strong>SCIP</strong> symbol index
        — that's <em>different</em> from the hybrid search index (BM25 +
        embeddings) you may have already built. Build it with:
      </p>
      <pre class="bg-ink-800 p-3 text-xs text-accent-500">
        belisarius index {path}
      </pre>
      <p class="text-xs text-ink-500">
        Per-language SCIP tools are required (any combo works):
      </p>
      <ul class="text-xs text-ink-400 list-disc list-inside space-y-1">
        <li><code class="text-accent-500">rust-analyzer scip</code> — Rust (<code>rustup component add rust-analyzer</code>)</li>
        <li><code class="text-accent-500">scip-typescript</code> — TS/JS (<code>npm i -g @sourcegraph/scip-typescript</code>)</li>
        <li><code class="text-accent-500">scip-python</code> — Python (<code>pipx install scip-python</code>)</li>
        <li><code class="text-accent-500">scip-go</code> — Go (<code>go install github.com/sourcegraph/scip-go/cmd/scip-go@latest</code>)</li>
      </ul>
      <p class="text-xs text-ink-500">
        Every other tab (Quality / Hotspots / Test gaps / Search / Architecture)
        works without this index — you can ignore this if you don't need
        symbol-level call graphs.
      </p>
    </div>
  );
}

function ImpactCard({ report }: { report: ImpactReport }) {
  return (
    <div class="card space-y-3">
      <h3 class="text-sm">
        impact of <code class="text-accent-500">{report.root}</code>
        {report.truncated && <span class="ml-2 text-ink-warning text-[10px]">TRUNCATED</span>}
      </h3>
      <p class="text-[11px] text-ink-500">
        {report.nodes.length} symbols, {report.files.length} files
      </p>
      <ul class="space-y-1 text-sm">
        {report.nodes.map((n, i) => (
          <li key={i} class="flex items-baseline gap-2 border-b border-ink-800 py-1">
            <span class="text-ink-500 w-6 text-[10px]">d{n.depth}</span>
            <span class="flex-1 truncate">
              <code class="text-accent-500">{n.display_name || n.symbol}</code>
              <span class="text-ink-500 ml-2 text-[10px]">via {n.callers_of}</span>
            </span>
            <span class="text-ink-500 text-[10px]">{n.call_site_count} sites</span>
          </li>
        ))}
      </ul>
      <details>
        <summary class="text-[11px] text-ink-500 cursor-pointer">files touched ({report.files.length})</summary>
        <ul class="mt-2 text-[11px]">
          {report.files.map((f) => (
            <li key={f} class="text-ink-400">
              {f}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}

function FlowCard({ report }: { report: FlowReport }) {
  return (
    <div class="card space-y-3">
      <h3 class="text-sm">
        flow from <code class="text-accent-500">{report.root}</code>
        {report.truncated && <span class="ml-2 text-ink-warning text-[10px]">TRUNCATED</span>}
      </h3>
      <p class="text-[11px] text-ink-500">{report.nodes.length} symbols</p>
      <ul class="space-y-1 text-sm">
        {report.nodes.map((n, i) => (
          <li key={i} class="flex items-baseline gap-2 border-b border-ink-800 py-1">
            <span class="text-ink-500 w-6 text-[10px]">d{n.depth}</span>
            <code class="text-accent-500 flex-1 truncate">{n.display_name || n.symbol}</code>
            <span class="text-ink-500 text-[10px]">← {n.called_from}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Sym360Card({ view }: { view: Symbol360 }) {
  return (
    <div class="card space-y-4">
      <header>
        <p class="text-[10px] uppercase tracking-wider text-ink-500">symbol</p>
        <h3 class="text-base text-accent-500">{view.display_name || view.symbol}</h3>
        <p class="text-[11px] text-ink-500 truncate">{view.symbol}</p>
        <p class="text-[11px] text-ink-500">{view.occurrence_count} occurrences</p>
      </header>

      <section>
        <p class="text-[10px] uppercase tracking-wider text-ink-500">definitions</p>
        <ul class="text-sm">
          {view.def_sites.map((d, i) => (
            <li key={i} class="text-ink-300">
              {d.file}:{d.range.start_line + 1}
            </li>
          ))}
        </ul>
      </section>

      <section>
        <p class="text-[10px] uppercase tracking-wider text-ink-500">callers ({view.callers.length})</p>
        <ul class="text-sm">
          {view.callers.map((c) => (
            <li key={c.symbol} class="flex items-baseline gap-2 border-b border-ink-800 py-1">
              <code class="text-accent-500 flex-1 truncate">{c.display_name || c.symbol}</code>
              <span class="text-ink-500 text-[10px]">{c.call_sites} call sites</span>
            </li>
          ))}
        </ul>
      </section>

      <section>
        <p class="text-[10px] uppercase tracking-wider text-ink-500">callees ({view.callees.length})</p>
        <ul class="text-sm">
          {view.callees.map((c) => (
            <li key={c.symbol} class="border-b border-ink-800 py-1 truncate">
              <code class="text-accent-500">{c.display_name || c.symbol}</code>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
