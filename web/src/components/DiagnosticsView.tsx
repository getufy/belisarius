import { useMemo, useState } from 'preact/hooks';
import { useQuery } from '@tanstack/react-query';
import { api, DiagnosticsReport } from '../api';
import { useDiagnosticsStatus, useRunDiagnostics } from '../data/queries';
import { CodeView } from './CodeView';

const TOOL_BLURB: Record<string, string> = {
  tokei: 'in-process Rust crate',
  clippy: 'cargo clippy',
  semgrep: 'semgrep',
  ruff: 'ruff',
  eslint: 'npx eslint',
};

export function DiagnosticsView({ path }: { path: string }) {
  const { data: status, error: statusErr } = useDiagnosticsStatus(path);
  // Try to surface previously-cached diagnostics on tab open so the table
  // is non-empty even before the user clicks "run". The list endpoint is
  // tolerant — returns 404-ish if no cache; we silently ignore.
  const { data: cachedList } = useQuery({
    queryKey: ['diagnostics_list', path],
    queryFn: () =>
      api.diagnosticsList(path, { limit: 1000 }).catch(() => null),
    enabled: !!path,
  });
  const runMutation = useRunDiagnostics(path);
  const liveReport = runMutation.data?.report ?? null;
  const cached = runMutation.data?.cached ?? null;
  const report: DiagnosticsReport | null = liveReport
    ?? (cachedList
      ? {
          tools_ran: cachedList.tools_ran,
          diagnostics: cachedList.diagnostics,
          counts_by_tool: cachedList.counts_by_tool,
          counts_by_severity: cachedList.counts_by_severity,
        }
      : null);
  const busy = runMutation.isPending;
  const err = statusErr ? String(statusErr) : runMutation.error ? String(runMutation.error) : null;

  const [toolFilter, setToolFilter] = useState<string | null>(null);
  const [sevFilter, setSevFilter] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  const filtered = useMemo(() => {
    if (!report) return [];
    const needle = search.trim().toLowerCase();
    return report.diagnostics.filter((d) => {
      if (toolFilter && d.tool !== toolFilter) return false;
      if (sevFilter && d.severity !== sevFilter) return false;
      if (needle) {
        const hay =
          `${d.tool} ${d.rule_id} ${d.file} ${d.message}`.toLowerCase();
        if (!hay.includes(needle)) return false;
      }
      return true;
    });
  }, [report, toolFilter, sevFilter, search]);

  const runAll = (force: boolean) => {
    runMutation.mutate({ force });
  };

  return (
    <div class="space-y-4">
      <div class="card flex flex-wrap items-baseline gap-3">
        <div class="flex-1 min-w-[260px]">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">
            External diagnostics
          </h2>
          <p class="text-xs text-ink-500">
            Runs installed tools (clippy, semgrep, ruff, eslint, tokei) and
            surfaces every issue alongside Belisarius's own metrics.
          </p>
        </div>
        <div class="flex items-center gap-2">
          <button
            class="btn"
            disabled={busy}
            onClick={() => runAll(true)}
            title="Force a fresh run, ignoring cache"
          >
            re-run
          </button>
          <button
            class="btn btn-primary"
            disabled={busy}
            onClick={() => runAll(false)}
          >
            {busy ? 'Running…' : 'Run all'}
          </button>
        </div>
      </div>

      {err && <p class="card text-ink-err">{err}</p>}

      {status && (
        <details class="card">
          <summary class="cursor-pointer hover:text-ink-300 select-none flex items-center gap-2 text-xs text-ink-500">
            <span class="text-ink-600">▸</span>
            <span>
              {status.filter((s) => s.installed).length} of {status.length} tools available
              {' · '}
              {status.filter((s) => s.installed && s.applied).length} ran
              {report && (
                <>
                  {' · '}
                  {Object.values(report.counts_by_tool).reduce<number>((a, b) => a + (b ?? 0), 0)} issues
                </>
              )}
            </span>
            {cached !== null && (
              <span
                class={`pill ml-auto ${cached ? 'text-ink-500 border-ink-700' : 'text-accent-500 border-accent-500/30'}`}
                title={cached ? 'served from .belisarius/diagnostics/report.json' : 'just produced'}
              >
                {cached ? 'cached' : 'fresh'}
              </span>
            )}
          </summary>
          <div class="mt-2 flex flex-wrap gap-2">
            {status.map((s) => {
              const installed = s.installed;
              const applied = s.applied;
              const stateColor = !installed
                ? 'text-ink-500 border-ink-700'
                : applied
                  ? 'text-green-400 border-green-400/30'
                  : 'text-ink-warning border-ink-warning/30';
              const cnt =
                report?.counts_by_tool?.[s.name] ??
                report?.tools_ran.find((t) => t.name === s.name)?.count ??
                0;
              return (
                <span
                  key={s.name}
                  class={`pill ${stateColor}`}
                  title={
                    installed
                      ? applied
                        ? `${TOOL_BLURB[s.name] ?? ''} · ${cnt} issues`
                        : 'installed but not applicable here'
                      : 'not installed'
                  }
                >
                  {s.name}
                  {installed && applied && (
                    <span class="ml-1 text-ink-500">·</span>
                  )}
                  {installed && applied && (
                    <span class="ml-1 text-ink-400">{cnt}</span>
                  )}
                  {!installed && <span class="ml-1 text-ink-500">—</span>}
                </span>
              );
            })}
          </div>
        </details>
      )}

      {report && report.diagnostics.length > 0 && (
        <div class="card flex flex-wrap items-end gap-3">
          <div class="flex-1 min-w-[180px]">
            <label class="label">Search</label>
            <input
              class="field"
              placeholder="message, rule, or file…"
              value={search}
              onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
            />
          </div>
          <div class="flex items-end gap-1">
            <button
              class={`pill ${toolFilter === null ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
              onClick={() => setToolFilter(null)}
            >
              all tools
            </button>
            {Object.entries(report.counts_by_tool)
              .sort((a, b) => (b[1] ?? 0) - (a[1] ?? 0))
              .map(([tool, n]) => (
                <button
                  key={tool}
                  class={`pill ${toolFilter === tool ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
                  onClick={() => setToolFilter(toolFilter === tool ? null : tool)}
                >
                  {tool} {n}
                </button>
              ))}
          </div>
          <div class="flex items-end gap-1">
            {['error', 'warning', 'info', 'hint'].map((s) => {
              const n = report.counts_by_severity[s] ?? 0;
              if (n === 0) return null;
              return (
                <button
                  key={s}
                  class={`pill ${sevFilter === s ? 'border-accent-500 text-accent-500' : sevColor(s)}`}
                  onClick={() => setSevFilter(sevFilter === s ? null : s)}
                >
                  {s} {n}
                </button>
              );
            })}
          </div>
          <div class="text-xs text-ink-500 ml-auto">
            {filtered.length} / {report.diagnostics.length}
          </div>
        </div>
      )}

      {report ? (
        report.diagnostics.length === 0 ? (
          <p class="card text-sm text-ink-500">
            No diagnostics yet. {!status?.some((s) => s.installed) && 'Install at least one of clippy, semgrep, ruff, or eslint to enable.'}
          </p>
        ) : (
          <div class="card max-h-[680px] overflow-auto p-0">
            <table class="w-full text-xs">
              <thead class="sticky top-0 bg-ink-800 text-ink-500 uppercase tracking-wider z-10">
                <tr>
                  <th class="px-2 py-1 text-left w-20">sev</th>
                  <th class="px-2 py-1 text-left w-24">tool</th>
                  <th class="px-2 py-1 text-left w-64">rule</th>
                  <th class="px-2 py-1 text-left">message</th>
                  <th class="px-2 py-1 text-left">file:line</th>
                  <th class="px-2 py-1 w-px" />
                </tr>
              </thead>
              <tbody>
                {filtered.slice(0, 500).map((d, i) => (
                  <tr
                    key={`${d.file}:${d.start_line}:${d.rule_id}:${i}`}
                    class="border-t border-ink-700 hover:bg-ink-800 cursor-pointer group"
                    onClick={() => setPreview({ file: d.file, line: d.start_line })}
                  >
                    <td class={`px-2 py-1 ${sevColor(d.severity)}`}>{d.severity}</td>
                    <td class="px-2 py-1 text-ink-400">{d.tool}</td>
                    <td class="px-2 py-1 text-ink-300 truncate max-w-[180px]" title={d.rule_id}>
                      {d.rule_id}
                    </td>
                    <td class="px-2 py-1 text-ink-300 truncate max-w-[420px]" title={d.message}>
                      {d.message.split('\n')[0]}
                    </td>
                    <td class="px-2 py-1 text-ink-500 truncate max-w-[280px]">
                      {d.file}:{d.start_line}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {filtered.length > 500 && (
              <p class="text-center text-[11px] text-ink-500 py-2">
                … {filtered.length - 500} more (narrow your filter)
              </p>
            )}
          </div>
        )
      ) : !err && (
        <p class="card text-sm text-ink-500">
          No cached run yet. Click <strong>Run all</strong> to scan the project.
        </p>
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

function sevColor(s: string): string {
  switch (s) {
    case 'error':
      return 'text-ink-err border-ink-err/30';
    case 'warning':
      return 'text-orange-400 border-orange-400/30';
    case 'info':
      return 'text-ink-warning border-ink-warning/30';
    default:
      return 'text-ink-500 border-ink-700';
  }
}
