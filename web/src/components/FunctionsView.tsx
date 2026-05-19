import { useMemo, useState } from 'preact/hooks';
import { useFunctions } from '../data/queries';
import { CodeView } from './CodeView';
import { FunctionDetailModal } from './FunctionDetailModal';
import { ErrorState } from './states/ErrorState';
import { LoadingState } from './states/LoadingState';
import { MetricTable, Column } from './MetricTable';
import type { FunctionInfo } from '../types/generated/FunctionInfo';

export function FunctionsView({ path }: { path: string }) {
  const { data, error, refetch } = useFunctions(path, { limit: 500 });
  const fns = data?.functions ?? null;
  const [minCc, setMinCc] = useState(0);
  const [showCog, setShowCog] = useState(false);
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);
  const [detail, setDetail] = useState<{ file: string; name: string } | null>(null);

  const filtered = useMemo(() => {
    if (!fns) return [];
    return fns.filter((f) => f.cyclomatic >= minCc);
  }, [fns, minCc]);

  if (error) return <ErrorState error={error} onRetry={refetch} />;
  if (!fns) return <LoadingState label="Analyzing functions… (first call is slow on big repos)" />;

  const columns: Column<FunctionInfo>[] = [
    {
      key: 'cyclomatic',
      header: 'cc',
      numeric: true,
      render: (f) => <span class={ccColor(f.cyclomatic)}>{f.cyclomatic}</span>,
    },
    ...(showCog
      ? ([
          {
            key: 'cognitive',
            header: 'cog',
            numeric: true,
          },
        ] as Column<FunctionInfo>[])
      : []),
    { key: 'loc', header: 'loc', numeric: true },
    { key: 'params', header: 'p', numeric: true },
    {
      key: 'name',
      header: 'name',
      render: (f) => <span class="text-ink-300 font-medium">{f.name}</span>,
    },
    {
      key: 'file',
      header: 'file:line',
      render: (f) => (
        <span class="text-ink-500 truncate max-w-[420px] inline-block align-middle">
          {f.file}:{f.start_line}
        </span>
      ),
    },
    {
      key: 'body_hash',
      header: '',
      sortable: false,
      render: (f) => (
        <button
          class="text-ink-500 hover:text-accent-500 text-xs"
          onClick={(e) => {
            e.stopPropagation();
            setDetail({ file: f.file, name: f.name });
          }}
          title="Open the function-detail bundle (snippet + callers + tests + churn)"
        >
          detail
        </button>
      ),
    },
  ];

  return (
    <div class="space-y-3">
      <div class="card flex flex-wrap items-end gap-3">
        <div>
          <label class="label">Min cyclomatic</label>
          <input
            class="field w-24"
            type="number"
            min="0"
            value={minCc}
            onInput={(e) => setMinCc(parseInt((e.target as HTMLInputElement).value || '0', 10))}
          />
        </div>
        <div class="flex items-center gap-2 ml-auto">
          <button
            class={`pill text-xs ${showCog ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
            onClick={() => setShowCog(!showCog)}
            title="Toggle the cognitive complexity column"
          >
            cog {showCog ? '·' : '+'}
          </button>
          <span class="text-xs text-ink-500">
            {filtered.length} / {fns.length} functions
          </span>
        </div>
      </div>

      <div class="card">
        <MetricTable<FunctionInfo>
          rows={filtered}
          columns={columns}
          rowKey={(f) => `${f.file}:${f.start_line}:${f.name}`}
          filter={(f, q) => {
            const needle = q.toLowerCase();
            return (
              f.name.toLowerCase().includes(needle) ||
              f.file.toLowerCase().includes(needle)
            );
          }}
          initialSort={{ key: 'cyclomatic', dir: 'desc' }}
          onRowClick={(f) => setPreview({ file: f.file, line: f.start_line })}
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
      {detail && (
        <FunctionDetailModal
          path={path}
          file={detail.file}
          name={detail.name}
          onClose={() => setDetail(null)}
        />
      )}
    </div>
  );
}

function ccColor(cc: number): string {
  if (cc >= 20) return 'text-ink-err font-medium';
  if (cc >= 10) return 'text-orange-400';
  if (cc >= 5) return 'text-ink-warning';
  return 'text-ink-400';
}
