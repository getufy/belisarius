import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { lazy, Suspense } from 'preact/compat';
import { api, Graph, QualitySummary, Scan } from '../api';
import { useGlobalKeys, useHashState, useRecentPaths } from '../hooks';
import { DeadFiles } from '../components/DeadFiles';
import { QualityView } from '../components/QualityView';
import { ComponentsView } from '../components/ComponentsView';
import { CommandsView } from '../components/CommandsView';
import { SurfaceView } from '../components/SurfaceView';
import { FunctionsView } from '../components/FunctionsView';
import { ContextView } from '../components/ContextView';
import { FindingsView } from '../components/FindingsView';

// Heavy tabs are split into separate chunks so the initial bundle drops the
// cytoscape / mermaid / treemap weight unless the user actually opens those
// tabs. Each tab's chunk loads on demand the first time the user navigates
// to it; subsequent visits hit cache instantly.
const CytoscapeGraph = lazy(() =>
  import('../components/CytoscapeGraph').then((m) => ({ default: m.CytoscapeGraph })),
);
const LocTreemap = lazy(() =>
  import('../components/LocTreemap').then((m) => ({ default: m.LocTreemap })),
);
const DsmView = lazy(() =>
  import('../components/DsmView').then((m) => ({ default: m.DsmView })),
);
const ArchitectureView = lazy(() =>
  import('../components/ArchitectureView').then((m) => ({ default: m.ArchitectureView })),
);
const ImpactView = lazy(() =>
  import('../components/ImpactView').then((m) => ({ default: m.ImpactView })),
);
const SymbolsView = lazy(() =>
  import('../components/SymbolsView').then((m) => ({ default: m.SymbolsView })),
);
const SearchView = lazy(() =>
  import('../components/SearchView').then((m) => ({ default: m.SearchView })),
);

function TabFallback() {
  return <p class="card text-ink-500">Loading tab…</p>;
}

type Tab =
  | 'overview'
  | 'search'
  | 'architecture'
  | 'graph'
  | 'treemap'
  | 'dsm'
  | 'dead'
  | 'functions'
  | 'components'
  | 'surface'
  | 'quality'
  | 'hotspots'
  | 'test-gaps'
  | 'findings'
  | 'impact'
  | 'commands'
  | 'diagnostics'
  | 'markers'
  | 'symbols'
  | 'context';

type TabDef = { id: Tab; label: string; help: string; badge?: () => string | null };

export function ScanView() {
  const recent = useRecentPaths();
  const [hashPath, setHashPath] = useHashState('path', '');
  const [hashTab, setHashTab] = useHashState('tab', 'overview');

  const [path, setPath] = useState(hashPath || recent.paths[0] || '.');
  const [scan, setScan] = useState<Scan | null>(null);
  const [graph, setGraph] = useState<Graph | null>(null);
  const [scanPath, setScanPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [quality, setQuality] = useState<QualitySummary | null>(null);
  const [markersCount, setMarkersCount] = useState<number | null>(null);
  const [showRecent, setShowRecent] = useState(false);
  const [dsmFile, setDsmFile] = useState<string | undefined>(undefined);
  const [diagCount, setDiagCount] = useState<number | null>(null);
  const [impactSeed, setImpactSeed] = useState<{ sym: import('../api').SymbolMatch; mode: 'impact' | 'flow' | '360' } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const tab = (hashTab as Tab) || 'overview';
  const setTab = (t: Tab) => setHashTab(t);

  const cycleNodes = useMemo(() => {
    if (!quality) return undefined;
    const out = new Set<string>();
    for (const i of quality.quality.top_issues) {
      if (i.kind === 'cycle') for (const n of i.nodes) out.add(n);
    }
    return out;
  }, [quality]);

  // Auto-run a scan when the URL hash carries a path (deep link).
  useEffect(() => {
    if (hashPath && hashPath !== scanPath && !busy) {
      setPath(hashPath);
      runScan(hashPath);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hashPath]);

  // Lazy background-fetch quality (cycle nodes + score badge) once the scan resolves.
  useEffect(() => {
    if (!scan) {
      setQuality(null);
      return;
    }
    api.quality(scan.root).then(setQuality).catch(() => setQuality(null));
  }, [scan]);

  // Lazy background-fetch marker count for the tab badge.
  useEffect(() => {
    if (!scan) {
      setMarkersCount(null);
      return;
    }
    api
      .markers(scan.root, 1000)
      .then((r) => setMarkersCount(r.total))
      .catch(() => setMarkersCount(null));
  }, [scan]);

  // Cached diagnostics count for the diagnostics-tab badge.
  useEffect(() => {
    if (!scan) {
      setDiagCount(null);
      return;
    }
    api
      .diagnosticsList(scan.root, { limit: 1 })
      .then((r) => setDiagCount(r.total_cached))
      .catch(() => setDiagCount(null));
  }, [scan, tab]);

  useGlobalKeys((key, ev) => {
    if (key === '/' && inputRef.current) {
      ev.preventDefault();
      inputRef.current.focus();
      inputRef.current.select();
      return;
    }
    if (!scan) return;
    const idx = '123456789'.indexOf(key);
    if (idx >= 0 && idx < tabs.length) {
      ev.preventDefault();
      setTab(tabs[idx].id);
    }
  });

  const runScan = async (target: string) => {
    const t = target.trim() || '.';
    setBusy(true);
    setErr(null);
    setScan(null);
    setGraph(null);
    setScanPath(t);
    setShowRecent(false);
    try {
      const [s, g] = await Promise.all([api.scan(t), api.graph(t)]);
      setScan(s);
      setGraph(g);
      recent.remember(t);
      setHashPath(t);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const onScan = () => runScan(path);

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Enter') onScan();
    if (e.key === 'Escape') setShowRecent(false);
  };

  // Tab definitions — badges compute when data is loaded.
  const tabs: TabDef[] = [
    { id: 'overview', label: 'Overview', help: 'Stats + raw JSON' },
    {
      id: 'search',
      label: 'Search',
      help: 'Hybrid semantic + BM25 search across the repo (RRF fusion)',
    },
    {
      id: 'architecture',
      label: 'Architecture',
      help: 'Mermaid diagram + directory-level coupling summary',
    },
    {
      id: 'graph',
      label: 'Graph',
      help: 'Force-directed file dependency map',
    },
    { id: 'treemap', label: 'Treemap', help: 'LOC by directory or complexity' },
    {
      id: 'dsm',
      label: 'DSM',
      help: 'Per-file dependency structure with line numbers',
    },
    {
      id: 'dead',
      label: 'Dead',
      help: 'Files no one imports',
      badge: () =>
        graph
          ? `${graph.nodes.filter((n) => n.in_degree === 0 && !n.is_entry_point && ['rust', 'typescript', 'javascript', 'python', 'go'].includes(n.language)).length}`
          : null,
    },
    {
      id: 'functions',
      label: 'Functions',
      help: 'Per-function complexity (tree-sitter AST)',
      badge: () => (quality ? `${quality.function_count}` : null),
    },
    {
      id: 'components',
      label: 'Components',
      help: 'Design-system component inventory (react-docgen)',
    },
    {
      id: 'surface',
      label: 'Surface',
      help: 'Public API surface — HTTP routes, exposed functions and types',
    },
    {
      id: 'quality',
      label: 'Quality',
      help: 'Composite 0-100 score across 4 axes',
      badge: () => (quality && quality.quality.score != null ? quality.quality.score.toFixed(0) : null),
    },
    {
      id: 'findings',
      label: 'Findings',
      help: 'Hotspots · Test gaps · Markers · Diagnostics — ranked risks across one view',
      badge: () => {
        const m = markersCount ?? 0;
        const d = diagCount ?? 0;
        const total = m + d;
        return total > 0 ? `${total}` : null;
      },
    },
    {
      id: 'impact',
      label: 'Impact',
      help: 'Backward (blast radius) / forward (flow) call traversal + symbol 360°',
    },
    {
      id: 'commands',
      label: 'Commands',
      help: 'Runnable commands: package.json scripts, Justfile, Makefile, workflows',
    },
    { id: 'symbols', label: 'Symbols', help: 'SCIP-indexed symbol search + refs + callers' },
    {
      id: 'context',
      label: 'Context',
      help: 'Non-code knowledge registry (schemas, runbooks)',
    },
  ];

  return (
    <div class="space-y-4">
      <div class="card">
        <div class="flex items-end gap-3">
          <div class="flex-1 relative">
            <label class="label flex items-center justify-between">
              <span>Project path</span>
              <span class="text-[10px] text-ink-500 font-normal normal-case tracking-normal">
                press <kbd class="kbd">/</kbd> to focus
              </span>
            </label>
            <input
              ref={inputRef}
              class="field"
              value={path}
              placeholder="."
              onInput={(e) => setPath((e.target as HTMLInputElement).value)}
              onKeyDown={onKey}
              onFocus={() => setShowRecent(recent.paths.length > 0)}
              onBlur={() => setTimeout(() => setShowRecent(false), 120)}
            />
            {showRecent && recent.paths.length > 0 && (
              <ul class="absolute z-20 mt-1 w-full border border-ink-700 bg-ink-900 shadow-lg text-sm overflow-hidden">
                {recent.paths.map((p) => (
                  <li
                    key={p}
                    class="flex items-center px-3 py-1.5 hover:bg-ink-800 cursor-pointer"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      setPath(p);
                      setShowRecent(false);
                      runScan(p);
                    }}
                  >
                    <code class="flex-1 text-ink-300">{p}</code>
                    <button
                      class="text-[10px] text-ink-600 hover:text-ink-err"
                      onMouseDown={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        recent.forget(p);
                      }}
                      title="Forget this path"
                    >
                      ✕
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <button class="btn btn-primary" disabled={busy} onClick={onScan}>
            {busy ? 'Scanning…' : 'Scan'}
          </button>
        </div>
        <p class="mt-2 text-[11px] text-ink-500">
          Resolved against the server's CWD. Use <code class="text-accent-500">..</code> or an
          absolute path like <code class="text-accent-500">/Users/you/project</code>.
        </p>
      </div>

      {busy && scanPath && (
        <p class="card text-ink-400">
          Scanning <code class="text-accent-500">{scanPath}</code>…
        </p>
      )}
      {err && (
        <p class="card text-ink-err">
          <strong>Error scanning {scanPath}:</strong> {err}
        </p>
      )}

      {scan && graph && !busy && (
        <div class="space-y-4">
          <header class="card flex flex-wrap items-baseline justify-between gap-3">
            <div class="min-w-0">
              <p class="text-[10px] uppercase tracking-wider text-ink-500">scanned</p>
              <p class="text-sm text-ink-300 truncate">
                <code class="text-accent-500">{scan.root}</code>
              </p>
            </div>
            <div class="flex items-baseline gap-3 text-[11px] text-ink-500">
              {quality && (
                <span class={qualityColor(quality.quality.score)} title="composite quality">
                  {quality.quality.score != null ? `${quality.quality.score.toFixed(0)} / 100` : '— / 100'}
                </span>
              )}
              <span>{scan.files.length} files</span>
              <span>{graph.edges.length} edges</span>
              <span>{new Date(scan.scanned_at).toLocaleTimeString()}</span>
            </div>
          </header>

          <nav class="flex gap-1 overflow-x-auto border-b border-ink-700 -mb-px scrollbar-thin">
            {tabs.map((t, i) => {
              const badge = t.badge ? t.badge() : null;
              const active = tab === t.id;
              return (
                <button
                  key={t.id}
                  onClick={() => setTab(t.id)}
                  title={`${t.help} · ${i + 1}`}
                  class={`group relative -mb-px shrink-0 px-3 py-2 text-sm transition flex items-center gap-1.5 ${
                    active
                      ? 'border-b-2 border-accent-500 text-accent-500'
                      : 'border-b-2 border-transparent text-ink-400 hover:text-ink-300'
                  }`}
                >
                  <span>{t.label}</span>
                  {badge && (
                    <span
                      class={`text-[10px] px-1.5 py-0.5 ${
                        active
                          ? 'bg-accent-500/10 text-accent-500'
                          : 'bg-ink-800 text-ink-500 group-hover:text-ink-400'
                      }`}
                    >
                      {badge}
                    </span>
                  )}
                </button>
              );
            })}
          </nav>

          <Suspense fallback={<TabFallback />}>
          {tab === 'overview' && <Overview scan={scan} graph={graph} />}
          {tab === 'architecture' && <ArchitectureView path={scan.root} />}
          {tab === 'graph' && (
            <CytoscapeGraph
              graph={graph}
              path={scan.root}
              cycleNodes={cycleNodes}
              onNodeOpen={(id) => {
                setDsmFile(id);
                setTab('dsm');
              }}
            />
          )}
          {tab === 'search' && (
            <SearchView
              path={scan.root}
              onOpenSnippet={(file) => {
                setDsmFile(file);
                setTab('dsm');
              }}
            />
          )}
          {tab === 'impact' && (
            <ImpactView
              path={scan.root}
              initialSymbol={impactSeed?.sym}
              initialMode={impactSeed?.mode}
            />
          )}
          {tab === 'context' && <ContextView path={scan.root} />}
          {tab === 'treemap' && (
            <LocTreemap
              nodes={graph.nodes}
              path={scan.root}
              onFileOpen={(id) => {
                setDsmFile(id);
                setTab('dsm');
              }}
            />
          )}
          {tab === 'dsm' && <DsmView path={scan.root} graph={graph} initialFile={dsmFile} />}
          {tab === 'dead' && <DeadFiles graph={graph} />}
          {tab === 'functions' && <FunctionsView path={scan.root} />}
          {tab === 'components' && <ComponentsView path={scan.root} />}
          {tab === 'surface' && <SurfaceView path={scan.root} />}
          {tab === 'quality' && <QualityView path={scan.root} />}
          {(tab === 'findings' ||
            tab === 'hotspots' ||
            tab === 'test-gaps' ||
            tab === 'markers' ||
            tab === 'diagnostics') && (
            <FindingsView
              path={scan.root}
              markersCount={markersCount}
              diagCount={diagCount}
            />
          )}
          {tab === 'commands' && <CommandsView path={scan.root} />}
          {tab === 'symbols' && (
            <SymbolsView
              path={scan.root}
              onJumpImpact={(sym, mode) => {
                setImpactSeed({ sym, mode });
                setTab('impact');
              }}
            />
          )}
          </Suspense>
        </div>
      )}
    </div>
  );
}

function qualityColor(score: number | null): string {
  if (score == null) return 'text-ink-500';
  if (score >= 80) return 'text-green-400';
  if (score >= 60) return 'text-ink-warning';
  if (score >= 40) return 'text-orange-400';
  return 'text-ink-err';
}

function Overview({ scan, graph }: { scan: Scan; graph: Graph }) {
  return (
    <div class="space-y-4">
      <div class="grid gap-3 md:grid-cols-4">
        <Stat label="files" value={scan.files.length} />
        <Stat
          label="resolved edges"
          value={graph.edges.length}
          hint={`${graph.unresolved} external`}
        />
        <Stat label="languages" value={Object.keys(scan.language_summary).length} />
        <Stat label="loc" value={scan.files.reduce((a, f) => a + f.loc, 0)} />
      </div>
      {scan.files.length === 0 ? (
        <p class="card text-ink-500">
          Scan returned 0 files. The path probably doesn't exist or doesn't contain any files
          in a language Belisarius recognizes.
        </p>
      ) : (
        <>
          <div class="card">
            <h2 class="text-xs uppercase tracking-wider text-ink-500">Languages</h2>
            <ul class="mt-2 grid gap-1 text-sm md:grid-cols-2">
              {Object.entries(scan.language_summary).map(([lang, s]) => (
                <li
                  key={lang}
                  class="flex justify-between border-b border-ink-700 py-1"
                >
                  <span class="text-ink-300">{lang}</span>
                  <span class="text-ink-500">{s?.files ?? 0} files · {s?.loc ?? 0} loc</span>
                </li>
              ))}
            </ul>
          </div>
          <details class="card">
            <summary class="cursor-pointer text-sm text-ink-500 hover:text-ink-300 select-none flex items-center gap-2">
              <span class="text-ink-600">▸</span>
              Files ({scan.files.length})
              <span class="text-ink-600 text-xs">— flat dump; prefer Graph or DSM for navigation</span>
            </summary>
            <ul class="mt-2 max-h-[400px] overflow-auto text-xs">
              {scan.files.slice(0, 500).map((f) => (
                <li
                  key={f.path}
                  class="flex justify-between border-b border-ink-700 py-0.5"
                >
                  <span class="truncate text-ink-300">{f.path}</span>
                  <span class="text-ink-500">{f.language} · {f.loc} loc</span>
                </li>
              ))}
              {scan.files.length > 500 && (
                <li class="py-2 text-center text-ink-500">… {scan.files.length - 500} more</li>
              )}
            </ul>
          </details>
          <div class="pt-2 mt-2 border-t border-ink-800">
            <p class="text-[10px] uppercase tracking-wider text-ink-600 mb-1">Advanced</p>
            <details class="card">
              <summary class="cursor-pointer text-sm text-ink-500 hover:text-ink-300 select-none flex items-center gap-2">
                <span class="text-ink-600">▸</span>
                Raw scan JSON
              </summary>
              <pre class="mt-2 max-h-[480px] overflow-auto text-[11px]">
                {JSON.stringify(scan, null, 2)}
              </pre>
            </details>
          </div>
        </>
      )}
    </div>
  );
}

function Stat({
  label,
  value,
  hint,
}: {
  label: string;
  value: number;
  hint?: string;
}) {
  return (
    <div class="card">
      <p class="text-[10px] uppercase tracking-wider text-ink-500">{label}</p>
      <p class="mt-1 text-2xl text-accent-500">{value.toLocaleString()}</p>
      {hint && <p class="mt-0.5 text-[10px] text-ink-500">{hint}</p>}
    </div>
  );
}
