import { useEffect, useMemo, useState } from 'preact/hooks';
import { QualityIssue, QualitySummary } from '../api';
import { useQuality } from '../data/queries';
import { CodeView } from './CodeView';
import { EmptyState } from './states/EmptyState';
import { ErrorState } from './states/ErrorState';
import { LoadingState } from './states/LoadingState';

type Filter = 'all' | 'hot' | 'cycle' | 'dead';

export function QualityView({ path }: { path: string }) {
  const { data, error, refetch } = useQuality(path);
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);
  const [filter, setFilter] = useState<Filter>('all');

  // Reset the filter chip when the user navigates to a different project so
  // they're not left looking at a stale `hot fns` filter on a project that
  // doesn't have any.
  useEffect(() => {
    setFilter('all');
  }, [path]);

  if (error) return <ErrorState error={error} onRetry={refetch} />;
  if (!data) return <LoadingState label="Computing quality… (first call walks the AST of every file)" />;

  const q = data.quality;
  const counts = useMemo(() => countIssues(q.top_issues), [q.top_issues]);
  const visible = filter === 'all'
    ? q.top_issues
    : q.top_issues.filter((i) => filterMatch(i, filter));

  const openFile = (file: string, line: number) => setPreview({ file, line });

  return (
    <div class="space-y-4">
      <Hero data={data} />

      <div class="grid gap-3 md:grid-cols-4">
        <Gauge
          label="complexity"
          value={q.axes.complexity}
          hint="cc & cognitive severity"
          active={filter === 'hot'}
          onClick={() => setFilter(filter === 'hot' ? 'all' : 'hot')}
        />
        <Gauge
          label="acyclicity"
          value={q.axes.acyclicity}
          hint={cyclesHint(data.cycles_count)}
          active={filter === 'cycle'}
          onClick={() => setFilter(filter === 'cycle' ? 'all' : 'cycle')}
        />
        <Gauge
          label="dead code"
          value={q.axes.dead_code}
          hint="no orphan files"
          active={filter === 'dead'}
          onClick={() => setFilter(filter === 'dead' ? 'all' : 'dead')}
        />
        <Gauge
          label="coupling"
          value={q.axes.coupling}
          hint="no god-files (out-degree)"
        />
      </div>

      {q.top_issues.length > 0 ? (
        <div class="card">
          <div class="flex flex-wrap items-center gap-2 mb-3">
            <h3 class="text-xs uppercase tracking-wider text-ink-500 mr-2">Issues</h3>
            <FilterTab label="all" count={q.top_issues.length} active={filter === 'all'} onClick={() => setFilter('all')} />
            <FilterTab label="hot fns" count={counts.hot} active={filter === 'hot'} onClick={() => setFilter('hot')} disabled={counts.hot === 0} />
            <FilterTab label="cycles" count={counts.cycle} total={data.cycles_count} active={filter === 'cycle'} onClick={() => setFilter('cycle')} disabled={counts.cycle === 0} />
            <FilterTab label="dead files" count={counts.dead} active={filter === 'dead'} onClick={() => setFilter('dead')} disabled={counts.dead === 0} />
          </div>
          {visible.length === 0 ? (
            <p class="text-sm text-ink-500 py-2">No issues in this category.</p>
          ) : (
            <ul class="space-y-1 text-sm">
              {visible.map((i, idx) => (
                <li key={idx} class="border-b border-ink-700 py-1.5">
                  <IssueRow issue={i} onOpen={openFile} />
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : (
        q.score != null && (
          <EmptyState
            title="Clean run"
            hint="No issues surfaced — every axis is clean."
          />
        )
      )}

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

function Hero({ data }: { data: QualitySummary }) {
  const q = data.quality;
  return (
    <div class="card">
      <div class="flex flex-wrap items-baseline justify-between gap-3">
        <div>
          <p class="text-[10px] uppercase tracking-wider text-ink-500">composite quality</p>
          <div class="flex items-baseline gap-3">
            <p class={`text-5xl font-medium leading-none ${scoreColor(q.score)}`}>
              {q.score != null ? q.score.toFixed(0) : '—'}
              <span class="text-ink-500 text-lg"> / 100</span>
            </p>
            <p class={`text-sm ${scoreColor(q.score)}`}>{verdict(q.score)}</p>
          </div>
          {q.score == null && (
            <p class="text-[11px] text-ink-500 mt-2 max-w-md">
              Unscored. Belisarius found no functions or resolved imports in this project — typical for empty
              folders, docs-only repos, or unsupported languages.
            </p>
          )}
        </div>
        <div class="text-right">
          <p class="text-[10px] uppercase tracking-wider text-ink-500">cycles</p>
          <p class={`text-2xl ${data.cycles_count > 0 ? 'text-orange-400' : 'text-ink-300'}`}>
            {data.cycles_count}
          </p>
        </div>
      </div>
      <details class="mt-2 text-[11px] text-ink-500">
        <summary class="cursor-pointer hover:text-ink-300 select-none w-fit">details</summary>
        <p class="mt-1">
          {data.function_count} functions · {data.file_count} files · max depth {data.max_depth}
        </p>
      </details>
    </div>
  );
}

function IssueRow({ issue, onOpen }: { issue: QualityIssue; onOpen: (file: string, line: number) => void }) {
  if (issue.kind === 'hot_function') {
    return (
      <div class="flex items-baseline gap-2 group">
        <span class={`pill ${ccPillColor(issue.cyclomatic)}`}>cc={issue.cyclomatic}</span>
        <span class={`pill ${cogPillColor(issue.cognitive)}`}>cog={issue.cognitive}</span>
        <button
          class="text-accent-500 hover:underline truncate"
          onClick={() => onOpen(issue.file, issue.start_line)}
          title="Preview function body"
        >
          {issue.name}
        </button>
        <span class="text-ink-500 text-xs truncate">{issue.file}:{issue.start_line}</span>
      </div>
    );
  }
  if (issue.kind === 'cycle') {
    return (
      <div class="space-y-1">
        <div class="flex items-baseline gap-2">
          <span class={`pill ${cyclePillColor(issue.nodes.length)}`}>cycle · {issue.nodes.length} files</span>
          {issue.nodes.length > 6 && (
            <span class="text-[10px] text-orange-400">large cycle — significant penalty</span>
          )}
        </div>
        <div class="flex flex-wrap items-center gap-1 pl-1 text-xs">
          {issue.nodes.map((node, idx) => (
            <>
              <button
                key={`${node}-${idx}`}
                class="text-accent-500 hover:underline font-mono"
                onClick={() => onOpen(node, 1)}
                title="Open file"
              >
                {shortPath(node)}
              </button>
              {idx < issue.nodes.length - 1 && <span class="text-ink-600">→</span>}
            </>
          ))}
        </div>
      </div>
    );
  }
  return (
    <div class="flex items-baseline gap-2">
      <span class="pill text-ink-warning border-ink-warning/30">dead</span>
      <button
        class="text-accent-500 hover:underline font-mono text-xs"
        onClick={() => onOpen(issue.path, 1)}
        title="Open file"
      >
        {issue.path}
      </button>
    </div>
  );
}

function Gauge({
  label,
  value,
  hint,
  active,
  onClick,
}: {
  label: string;
  value: number | null;
  hint: string;
  active?: boolean;
  onClick?: () => void;
}) {
  const pct = value != null ? Math.max(0, Math.min(100, value)) : 0;
  const interactive = !!onClick && value != null;
  const cls =
    'card transition ' +
    (interactive ? 'cursor-pointer hover:border-ink-500 ' : '') +
    (active ? 'border-accent-500/60 ring-1 ring-accent-500/30' : '');
  return (
    <div
      class={cls}
      onClick={interactive ? onClick : undefined}
      role={interactive ? 'button' : undefined}
      tabIndex={interactive ? 0 : undefined}
    >
      <div class="flex items-baseline justify-between">
        <p class="text-[10px] uppercase tracking-wider text-ink-500">{label}</p>
        <p class={`text-2xl ${scoreColor(value)}`}>{value != null ? value.toFixed(0) : '—'}</p>
      </div>
      <div class="mt-2 h-1.5 bg-ink-800 overflow-hidden">
        <div class={`h-full ${barColor(value)}`} style={{ width: `${pct}%` }} />
      </div>
      <p class="mt-1 text-[10px] text-ink-500">{value != null ? hint : 'unscored — no data'}</p>
    </div>
  );
}

function FilterTab({
  label,
  count,
  total,
  active,
  onClick,
  disabled,
}: {
  label: string;
  count: number;
  total?: number;
  active: boolean;
  onClick: () => void;
  disabled?: boolean;
}) {
  const base = 'text-xs px-2 py-0.5 border rounded-sm transition ';
  const cls = disabled
    ? base + 'border-ink-800 text-ink-600 cursor-not-allowed'
    : active
      ? base + 'border-accent-500/60 text-accent-500 bg-accent-500/10'
      : base + 'border-ink-700 text-ink-400 hover:text-ink-200 hover:border-ink-500';
  const suffix = total != null && total > count ? ` (top ${count} of ${total})` : ` · ${count}`;
  return (
    <button class={cls} onClick={onClick} disabled={disabled}>
      {label}{suffix}
    </button>
  );
}

function countIssues(issues: QualityIssue[]) {
  let hot = 0, cycle = 0, dead = 0;
  for (const i of issues) {
    if (i.kind === 'hot_function') hot++;
    else if (i.kind === 'cycle') cycle++;
    else dead++;
  }
  return { hot, cycle, dead };
}

function filterMatch(i: QualityIssue, f: Filter): boolean {
  if (f === 'hot') return i.kind === 'hot_function';
  if (f === 'cycle') return i.kind === 'cycle';
  if (f === 'dead') return i.kind === 'dead_file';
  return true;
}

function verdict(v: number | null): string {
  if (v == null) return 'unscored';
  if (v >= 90) return 'excellent';
  if (v >= 75) return 'healthy';
  if (v >= 60) return 'watch';
  if (v >= 40) return 'concerning';
  return 'at risk';
}

function cyclesHint(n: number): string {
  if (n === 0) return 'no import cycles';
  if (n === 1) return '1 import cycle';
  return `${n} import cycles`;
}

function shortPath(p: string): string {
  // crates/foo/src/bar/baz.rs → bar/baz.rs ; keeps the last 2 segments.
  const parts = p.split('/');
  if (parts.length <= 2) return p;
  return parts.slice(-2).join('/');
}

function ccPillColor(cc: number): string {
  if (cc >= 30) return 'text-ink-err border-ink-err/40';
  if (cc >= 20) return 'text-orange-400 border-orange-400/30';
  return 'text-ink-warning border-ink-warning/30';
}

function cogPillColor(cog: number): string {
  if (cog >= 50) return 'text-ink-err border-ink-err/40';
  if (cog >= 25) return 'text-orange-400 border-orange-400/30';
  return 'text-ink-400 border-ink-700';
}

function cyclePillColor(size: number): string {
  if (size >= 10) return 'text-ink-err border-ink-err/40';
  if (size >= 5) return 'text-orange-400 border-orange-400/30';
  return 'text-ink-warning border-ink-warning/30';
}

function scoreColor(v: number | null): string {
  if (v == null) return 'text-ink-500';
  if (v >= 80) return 'text-green-400';
  if (v >= 60) return 'text-ink-warning';
  if (v >= 40) return 'text-orange-400';
  return 'text-ink-err';
}

function barColor(v: number | null): string {
  if (v == null) return 'bg-ink-700';
  if (v >= 80) return 'bg-green-500/60';
  if (v >= 60) return 'bg-ink-warning/60';
  if (v >= 40) return 'bg-orange-500/60';
  return 'bg-ink-err/60';
}
