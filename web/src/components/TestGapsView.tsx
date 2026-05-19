import { useState } from 'preact/hooks';
import { useTestGaps } from '../data/queries';
import { CodeView } from './CodeView';
import { ErrorState } from './states/ErrorState';
import { LoadingState } from './states/LoadingState';
import { MetricTable, Column } from './MetricTable';
import type { TestGap } from '../api';

export function TestGapsView({ path }: { path: string }) {
  const [limit, setLimit] = useState(50);
  const { data, error, refetch } = useTestGaps(path, limit);
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  if (error) return <ErrorState error={error} onRetry={refetch} />;
  if (!data) return <LoadingState label="Mapping tests…" />;

  const s = data.summary;

  const columns: Column<TestGap>[] = [
    {
      key: 'total_cyclomatic',
      header: 'cc',
      numeric: true,
      render: (g) => (
        <span class={ccClass(g.total_cyclomatic)}>{g.total_cyclomatic}</span>
      ),
    },
    { key: 'loc', header: 'loc', numeric: true },
    { key: 'function_count', header: 'fns', numeric: true },
    {
      key: 'language',
      header: 'language',
      render: (g) => <span class="text-ink-400">{g.language}</span>,
    },
    {
      key: 'source',
      header: 'file',
      render: (g) => (
        <span class="text-ink-300 truncate max-w-[640px] inline-block align-middle">
          {g.source}
        </span>
      ),
    },
  ];

  return (
    <div class="space-y-4">
      <div class="card flex flex-wrap items-end gap-3">
        <div class="flex-1 min-w-[200px]">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">Test gaps</h2>
          <p class="text-xs text-ink-500">
            Source files with no covering test. Coverage is derived statically
            from the import graph (test files importing source files) plus
            Rust inline <code>#[cfg(test)]</code> self-tests — a lower bound,
            not a coverage tool replacement.
          </p>
        </div>
        <div>
          <label class="label">Show top</label>
          <select
            class="field"
            value={limit}
            onChange={(e) => setLimit(parseInt((e.target as HTMLSelectElement).value, 10))}
          >
            <option value="25">25</option>
            <option value="50">50</option>
            <option value="100">100</option>
            <option value="250">250</option>
            <option value="500">500</option>
          </select>
        </div>
      </div>

      <div class="card grid grid-cols-2 md:grid-cols-4 gap-3 text-xs">
        <Stat label="source files" value={s.source_files} />
        <Stat label="test files" value={s.test_files} />
        <Stat
          label="covered"
          value={`${s.covered_files} (${s.coverage_pct.toFixed(1)}%)`}
          accent={s.coverage_pct < 50 ? 'warn' : undefined}
        />
        <Stat label="gaps" value={s.gap_files} accent={s.gap_files > 0 ? 'warn' : undefined} />
      </div>

      <div class="card">
        <MetricTable<TestGap>
          rows={data.gaps}
          columns={columns}
          rowKey={(g) => g.source}
          filter={(g, q) => g.source.toLowerCase().includes(q.toLowerCase())}
          initialSort={{ key: 'total_cyclomatic', dir: 'desc' }}
          onRowClick={(g) => setPreview({ file: g.source, line: 1 })}
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

function Stat({
  label,
  value,
  accent,
}: {
  label: string;
  value: string | number;
  accent?: 'warn';
}) {
  const color = accent === 'warn' ? 'text-orange-400' : 'text-ink-300';
  return (
    <div>
      <div class="text-[10px] uppercase tracking-wider text-ink-500">{label}</div>
      <div class={`text-lg font-medium ${color}`}>{value}</div>
    </div>
  );
}

function ccClass(c: number): string {
  if (c >= 100) return 'text-ink-err font-medium';
  if (c >= 50) return 'text-orange-400';
  if (c >= 20) return 'text-ink-warning';
  return 'text-ink-400';
}
