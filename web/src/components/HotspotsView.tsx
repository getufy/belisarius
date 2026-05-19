import { useMemo, useState } from 'preact/hooks';
import { Hotspot } from '../api';
import { useHotspots } from '../data/queries';
import { CodeView } from './CodeView';
import { ErrorState } from './states/ErrorState';
import { LoadingState } from './states/LoadingState';
import { MetricTable, Column } from './MetricTable';

export function HotspotsView({ path }: { path: string }) {
  const [days, setDays] = useState(90);
  const { data, error, refetch } = useHotspots(path, days, 50);
  const [showAuthors, setShowAuthors] = useState(false);
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  const hasOwners = useMemo(
    () => !!data?.hotspots.some((h) => (h.owners ?? []).length > 0),
    [data],
  );

  if (error) return <ErrorState error={error} onRetry={refetch} />;
  if (!data) return <LoadingState label="Walking git history…" />;

  if (!data.repo_present) {
    return (
      <div class="card text-sm text-ink-400 space-y-2">
        <p>
          <strong>No git repository found</strong> under{' '}
          <code class="text-accent-500">{path}</code>.
        </p>
        <p class="text-xs text-ink-500">
          Hotspots rank files by recent churn × cyclomatic complexity — they
          need a git history to compute. Initialize a repo and rerun the scan.
        </p>
      </div>
    );
  }

  const columns: Column<Hotspot>[] = [
    {
      key: 'score',
      header: 'score',
      numeric: true,
      render: (h) => (
        <span class={scoreClass(h.score)}>{h.score.toFixed(0)}</span>
      ),
    },
    { key: 'churn', header: 'churn', numeric: true },
    {
      key: 'complexity',
      header: 'cc',
      numeric: true,
      render: (h) => <span class={ccClass(h.complexity)}>{h.complexity}</span>,
    },
    { key: 'function_count', header: 'fns', numeric: true },
    {
      key: 'last_edited',
      header: 'last edit',
      numeric: true,
      render: (h) => (h.last_edited ? `${daysAgo(h.last_edited)}d` : '—'),
    },
    ...(showAuthors
      ? ([
          {
            key: 'last_author',
            header: 'last commit by',
            render: (h: Hotspot) => (
              <span class="text-ink-400 truncate max-w-[180px] inline-block align-middle">
                {h.last_author ?? '—'}
              </span>
            ),
          },
          {
            key: 'top_author',
            header: 'top in window',
            render: (h: Hotspot) => (
              <span class="text-ink-400 truncate max-w-[180px] inline-block align-middle">
                {h.top_author ?? '—'}
              </span>
            ),
          },
          ...(hasOwners
            ? [
                {
                  key: 'owners' as keyof Hotspot & string,
                  header: 'owners',
                  sortable: false,
                  render: (h: Hotspot) => (
                    <span class="text-accent-500 truncate max-w-[180px] inline-block align-middle">
                      {(h.owners ?? []).length > 0 ? h.owners.join(' ') : '—'}
                    </span>
                  ),
                },
              ]
            : []),
        ] as Column<Hotspot>[])
      : []),
    {
      key: 'path',
      header: 'file',
      render: (h) => (
        <span class="text-ink-300 truncate max-w-[420px] inline-block align-middle">
          {h.path}
        </span>
      ),
    },
  ];

  return (
    <div class="space-y-4">
      <div class="card flex flex-wrap items-end gap-3">
        <div class="flex-1 min-w-[200px]">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">
            Hotspots
          </h2>
          <p class="text-xs text-ink-500">
            Files ranked by <code>log2(churn + 1) × cyclomatic</code> over the
            last {data.days_window} days. The highest-leverage files to review
            — code that's both churning and complex.
          </p>
        </div>
        <div>
          <label class="label">Window (days)</label>
          <select
            class="field"
            value={days}
            onChange={(e) => setDays(parseInt((e.target as HTMLSelectElement).value, 10))}
          >
            <option value="14">14</option>
            <option value="30">30</option>
            <option value="90">90</option>
            <option value="180">180</option>
            <option value="365">365</option>
          </select>
        </div>
        <div class="flex items-center gap-2 ml-auto">
          <button
            class={`pill text-xs ${showAuthors ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
            onClick={() => setShowAuthors(!showAuthors)}
            title="Toggle author columns (last commit, top author, owners)"
          >
            authors {showAuthors ? '·' : '+'}
          </button>
          <span class="text-xs text-ink-500">{data.hotspots.length}</span>
        </div>
      </div>

      <div class="card">
        <MetricTable<Hotspot>
          rows={data.hotspots}
          columns={columns}
          rowKey={(h) => h.path}
          filter={(h, q) => h.path.toLowerCase().includes(q.toLowerCase())}
          initialSort={{ key: 'score', dir: 'desc' }}
          onRowClick={(h) => setPreview({ file: h.path, line: 1 })}
        />
      </div>

      {preview && (
        <CodeView
          path={path}
          file={preview.file}
          line={preview.line}
          onClose={() => setPreview(null)}
        />
      )}
    </div>
  );
}

function daysAgo(iso: string | null): number {
  if (!iso) return Number.MAX_SAFE_INTEGER;
  const dt = new Date(iso).getTime();
  if (!dt) return Number.MAX_SAFE_INTEGER;
  return Math.floor((Date.now() - dt) / 86_400_000);
}

function scoreClass(s: number): string {
  if (s >= 200) return 'text-ink-err font-medium';
  if (s >= 100) return 'text-orange-400';
  if (s >= 50) return 'text-ink-warning';
  return 'text-ink-400';
}
function ccClass(c: number): string {
  if (c >= 50) return 'text-ink-err';
  if (c >= 20) return 'text-orange-400';
  if (c >= 10) return 'text-ink-warning';
  return 'text-ink-400';
}

// keep this prop typed
export type _Hotspot = Hotspot;
