import { useEffect, useState } from 'preact/hooks';
import {
  api,
  FleetApp,
  FleetFindResponse,
  FleetHotspotRow,
  FleetTestGapRow,
} from '../api';

export function FleetView() {
  const [apps, setApps] = useState<FleetApp[] | null>(null);
  const [configPath, setConfigPath] = useState<string>('');
  const [err, setErr] = useState<string | null>(null);
  const [hotspots, setHotspots] = useState<FleetHotspotRow[] | null>(null);
  const [gaps, setGaps] = useState<FleetTestGapRow[] | null>(null);

  useEffect(() => {
    api
      .fleetList()
      .then((r) => {
        setApps(r.apps);
        setConfigPath(r.config_path);
      })
      .catch((e) => setErr(String(e)));
    api.fleetHotspots(10).then((r) => setHotspots(r.hotspots)).catch(() => setHotspots([]));
    api.fleetTestGaps(10).then((r) => setGaps(r.gaps)).catch(() => setGaps([]));
  }, []);

  if (err) return <p class="card text-ink-err">{err}</p>;
  if (!apps) return <p class="card text-ink-500">Loading fleet…</p>;

  if (apps.length === 0) {
    return (
      <div class="space-y-4">
        <div class="card space-y-3 text-sm">
          <h1 class="text-xl text-ink-300">Fleet is empty</h1>
          <p class="text-ink-400">
            The fleet registry at{' '}
            <code class="text-accent-500 text-xs">{configPath}</code> has no
            apps. Register one from the CLI to populate this view:
          </p>
          <pre class="bg-ink-800 p-3 text-xs text-accent-500">
            belisarius fleet add my-app /path/to/project
            {'\n'}belisarius fleet sync
          </pre>
          <p class="text-xs text-ink-500">
            Cross-fleet hotspots, test gaps, and surface search all read from
            the same SQLite index <code>belisarius fleet sync</code> populates.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div class="space-y-6">
      <header>
        <h1 class="text-xl text-ink-300">Fleet</h1>
        <p class="text-xs text-ink-500">
          {apps.length} registered · config{' '}
          <code class="text-ink-400">{configPath}</code>
        </p>
      </header>

      <section class="grid gap-3 md:grid-cols-2 lg:grid-cols-3">
        {apps.map((app) => (
          <AppCard key={app.name} app={app} />
        ))}
      </section>

      <FleetFindCard />

      <section class="grid gap-4 lg:grid-cols-2">
        <PortfolioHotspots rows={hotspots} />
        <PortfolioTestGaps rows={gaps} />
      </section>
    </div>
  );
}

function AppCard({ app }: { app: FleetApp }) {
  const s = app.summary;
  const lastSync = app.last_synced
    ? new Date(app.last_synced).toISOString().split('T')[0]
    : '—';
  const scanUrl = `/scans#path=${encodeURIComponent(app.path)}`;
  return (
    <a
      href={scanUrl}
      class="card hover:border-accent-500 transition-colors block space-y-2"
    >
      <div class="flex items-baseline justify-between gap-2">
        <span class="text-sm font-medium text-accent-500 truncate">
          {app.name}
        </span>
        <span class="text-[10px] uppercase tracking-wider text-ink-500">
          {s?.primary_language ?? '—'}
        </span>
      </div>
      <code class="block text-[11px] text-ink-500 truncate">{app.path}</code>
      {app.description && (
        <p class="text-xs text-ink-400">{app.description}</p>
      )}
      <div class="flex gap-4 text-[11px] text-ink-500">
        <span>
          <strong class="text-ink-300">{s?.file_count ?? '—'}</strong> files
        </span>
        <span>
          <strong class="text-ink-300">
            {s?.loc?.toLocaleString() ?? '—'}
          </strong>{' '}
          loc
        </span>
        <span class="ml-auto">synced {lastSync}</span>
      </div>
      {app.tags.length > 0 && (
        <div class="flex flex-wrap gap-1">
          {app.tags.map((t) => (
            <span
              key={t}
              class="px-1.5 py-0.5 border border-ink-700 text-[10px] text-ink-400"
            >
              {t}
            </span>
          ))}
        </div>
      )}
    </a>
  );
}

function FleetFindCard() {
  const [pattern, setPattern] = useState('');
  const [kind, setKind] = useState('');
  const [results, setResults] = useState<FleetFindResponse | null>(null);
  const [busy, setBusy] = useState(false);

  const search = () => {
    if (!pattern.trim()) return;
    setBusy(true);
    api
      .fleetFind(pattern.trim(), { kind: kind || undefined, limit: 50 })
      .then(setResults)
      .catch(() => setResults({ count: 0, results: [] }))
      .finally(() => setBusy(false));
  };

  return (
    <section class="card space-y-3">
      <div class="flex items-baseline justify-between">
        <h2 class="text-sm uppercase tracking-wider text-ink-500">
          Search across fleet
        </h2>
        <span class="text-xs text-ink-500">
          public surface — functions, types, http routes, …
        </span>
      </div>
      <div class="flex flex-wrap items-end gap-2">
        <div class="flex-1 min-w-[260px]">
          <label class="label">Pattern</label>
          <input
            class="field"
            placeholder="e.g. users  or  %User%"
            value={pattern}
            onInput={(e) => setPattern((e.target as HTMLInputElement).value)}
            onKeyDown={(e) => {
              if ((e as KeyboardEvent).key === 'Enter') search();
            }}
          />
        </div>
        <div>
          <label class="label">Kind</label>
          <select
            class="field"
            value={kind}
            onChange={(e) => setKind((e.target as HTMLSelectElement).value)}
          >
            <option value="">any</option>
            <option value="function">function</option>
            <option value="type">type</option>
            <option value="http_route">http_route</option>
            <option value="module">module</option>
            <option value="constant">constant</option>
            <option value="re_export">re_export</option>
            <option value="cli_command">cli_command</option>
          </select>
        </div>
        <button class="btn btn-primary" onClick={search} disabled={busy}>
          {busy ? 'Searching…' : 'Search'}
        </button>
      </div>
      {results && (
        <>
          <p class="text-xs text-ink-500">
            {results.count} match{results.count === 1 ? '' : 'es'}
          </p>
          {results.count > 0 && (
            <div class="overflow-auto">
              <table class="w-full text-xs">
                <thead class="text-ink-500 uppercase tracking-wider">
                  <tr>
                    <th class="text-left px-2 py-1 w-28">app</th>
                    <th class="text-left px-2 py-1 w-20">method</th>
                    <th class="text-left px-2 py-1 w-28">kind</th>
                    <th class="text-left px-2 py-1">name</th>
                    <th class="text-left px-2 py-1">file:line</th>
                  </tr>
                </thead>
                <tbody>
                  {results.results.map((r) => (
                    <tr
                      key={`${r.app}|${r.file}|${r.line}|${r.name}|${r.kind}`}
                      class="border-t border-ink-700"
                    >
                      <td class="px-2 py-1 text-accent-500">{r.app}</td>
                      <td class="px-2 py-1 text-ink-400">{r.method ?? ''}</td>
                      <td class="px-2 py-1 text-ink-400">{r.kind}</td>
                      <td class="px-2 py-1 text-ink-300 truncate max-w-[260px]">
                        {r.name}
                      </td>
                      <td class="px-2 py-1 text-ink-500 truncate max-w-[320px]">
                        {r.file}:{r.line}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </>
      )}
    </section>
  );
}

function PortfolioHotspots({ rows }: { rows: FleetHotspotRow[] | null }) {
  return (
    <section class="card space-y-2">
      <div class="flex items-baseline justify-between">
        <h2 class="text-sm uppercase tracking-wider text-ink-500">
          Top portfolio hotspots
        </h2>
        <span class="text-xs text-ink-500">churn × complexity</span>
      </div>
      {!rows ? (
        <p class="text-xs text-ink-500">Loading…</p>
      ) : rows.length === 0 ? (
        <p class="text-xs text-ink-500">
          No hotspots yet — run <code>belisarius fleet sync</code>.
        </p>
      ) : (
        <table class="w-full text-xs">
          <thead class="text-ink-500 uppercase tracking-wider">
            <tr>
              <th class="text-right px-2 py-1 w-12">score</th>
              <th class="text-left px-2 py-1 w-24">app</th>
              <th class="text-left px-2 py-1 w-28">owners</th>
              <th class="text-left px-2 py-1">file</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={`${r.app}|${r.file}`} class="border-t border-ink-700">
                <td class="px-2 py-1 text-right text-orange-400">
                  {r.score.toFixed(0)}
                </td>
                <td class="px-2 py-1 text-accent-500 truncate max-w-[100px]">
                  {r.app}
                </td>
                <td class="px-2 py-1 text-ink-400 truncate max-w-[120px]">
                  {r.owners.length > 0 ? r.owners.join(' ') : '—'}
                </td>
                <td class="px-2 py-1 text-ink-300 truncate max-w-[360px]">
                  {r.file}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

function PortfolioTestGaps({ rows }: { rows: FleetTestGapRow[] | null }) {
  return (
    <section class="card space-y-2">
      <div class="flex items-baseline justify-between">
        <h2 class="text-sm uppercase tracking-wider text-ink-500">
          Top untested files
        </h2>
        <span class="text-xs text-ink-500">no covering test, ranked by cc</span>
      </div>
      {!rows ? (
        <p class="text-xs text-ink-500">Loading…</p>
      ) : rows.length === 0 ? (
        <p class="text-xs text-ink-500">
          No data yet — run <code>belisarius fleet sync</code>.
        </p>
      ) : (
        <table class="w-full text-xs">
          <thead class="text-ink-500 uppercase tracking-wider">
            <tr>
              <th class="text-right px-2 py-1 w-12">cc</th>
              <th class="text-left px-2 py-1 w-24">app</th>
              <th class="text-left px-2 py-1 w-20">lang</th>
              <th class="text-left px-2 py-1">file</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={`${r.app}|${r.file}`} class="border-t border-ink-700">
                <td class="px-2 py-1 text-right text-orange-400">
                  {r.total_cyclomatic}
                </td>
                <td class="px-2 py-1 text-accent-500 truncate max-w-[100px]">
                  {r.app}
                </td>
                <td class="px-2 py-1 text-ink-400">{r.language}</td>
                <td class="px-2 py-1 text-ink-300 truncate max-w-[360px]">
                  {r.file}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

// Re-export to keep TS happy if upstream tools only see the route.
export const _placeholder = null;
