import { useMemo, useState } from 'preact/hooks';
import { SurfaceItem, SurfaceKind } from '../api';
import { useSurface } from '../data/queries';
import { CodeView } from './CodeView';

const KIND_LABEL: Record<SurfaceKind, string> = {
  http_route: 'HTTP routes',
  cli_command: 'CLI commands',
  function: 'Functions',
  type: 'Types',
  module: 'Modules',
  constant: 'Constants',
  re_export: 'Re-exports',
};

const METHOD_COLOR: Record<string, string> = {
  GET: 'text-green-400 border-green-400/30',
  POST: 'text-ink-warning border-ink-warning/30',
  PUT: 'text-ink-warning border-ink-warning/30',
  PATCH: 'text-ink-warning border-ink-warning/30',
  DELETE: 'text-ink-err border-ink-err/30',
  OPTIONS: 'text-ink-400 border-ink-700',
  HEAD: 'text-ink-400 border-ink-700',
};

export function SurfaceView({ path }: { path: string }) {
  const { data, error } = useSurface(path);
  const err = error ? String(error) : null;
  const [kindFilter, setKindFilter] = useState<SurfaceKind | null>(null);
  const [search, setSearch] = useState('');
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  const filtered = useMemo(() => {
    if (!data) return [];
    const needle = search.trim().toLowerCase();
    return data.items.filter((it) => {
      if (kindFilter && it.kind !== kindFilter) return false;
      if (needle) {
        const hay = `${it.name} ${it.file} ${it.method ?? ''}`.toLowerCase();
        if (!hay.includes(needle)) return false;
      }
      return true;
    });
  }, [data, kindFilter, search]);

  // Group filtered items by kind so we can render kind-specific sections.
  const grouped = useMemo(() => {
    const m = new Map<SurfaceKind, SurfaceItem[]>();
    for (const it of filtered) {
      const arr = m.get(it.kind);
      if (arr) arr.push(it);
      else m.set(it.kind, [it]);
    }
    return m;
  }, [filtered]);

  if (err) return <p class="card text-ink-err">{err}</p>;
  if (!data) return <p class="card text-ink-500">Scanning public surface…</p>;
  if (data.items.length === 0) {
    return (
      <p class="card text-sm text-ink-500">
        Nothing public found — no `pub` items, exports, or HTTP routes detected.
      </p>
    );
  }

  return (
    <div class="space-y-4">
      <div class="card flex flex-wrap items-end gap-3">
        <div class="flex-1 min-w-[240px]">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">
            Public surface
          </h2>
          <p class="text-xs text-ink-500">
            Everything this project exposes — Rust <code>pub</code> items, TS
            exports, and HTTP routes. {data.items.length} item
            {data.items.length === 1 ? '' : 's'} across {Object.keys(data.counts_by_language).length} language
            {Object.keys(data.counts_by_language).length === 1 ? '' : 's'}.
          </p>
        </div>
        <input
          class="field max-w-[280px]"
          placeholder="search name, file…"
          value={search}
          onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
        />
      </div>

      <div class="card flex flex-wrap items-center gap-1">
        <button
          class={`pill ${
            kindFilter === null
              ? 'border-accent-500 text-accent-500'
              : 'text-ink-500 hover:text-ink-300'
          }`}
          onClick={() => setKindFilter(null)}
        >
          all {data.items.length}
        </button>
        {Object.entries(data.counts_by_kind)
          .sort((a, b) => (b[1] ?? 0) - (a[1] ?? 0))
          .map(([k, n]) => (
            <button
              key={k}
              class={`pill ${
                kindFilter === (k as SurfaceKind)
                  ? 'border-accent-500 text-accent-500'
                  : 'text-ink-500 hover:text-ink-300'
              }`}
              onClick={() =>
                setKindFilter(kindFilter === (k as SurfaceKind) ? null : (k as SurfaceKind))
              }
            >
              {KIND_LABEL[k as SurfaceKind] ?? k} {n}
            </button>
          ))}
      </div>

      {(['http_route', 'cli_command', 'function', 'type', 'constant', 'module', 're_export'] as SurfaceKind[]).map(
        (kind) => {
          const rows = grouped.get(kind);
          if (!rows || rows.length === 0) return null;
          return (
            <SectionTable
              key={kind}
              kind={kind}
              rows={rows}
              onOpen={(f, line) => setPreview({ file: f, line })}
            />
          );
        }
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

function SectionTable({
  kind,
  rows,
  onOpen,
}: {
  kind: SurfaceKind;
  rows: SurfaceItem[];
  onOpen: (file: string, line: number) => void;
}) {
  return (
    <div class="card overflow-auto p-0">
      <header class="px-3 py-2 border-b border-ink-700 bg-ink-800">
        <h3 class="text-xs uppercase tracking-wider text-ink-500">
          {KIND_LABEL[kind]} — {rows.length}
        </h3>
      </header>
      <table class="w-full text-xs">
        <tbody>
          {rows.slice(0, 200).map((it, idx) => (
            <tr
              key={`${it.file}:${it.line}:${it.name}:${idx}`}
              class="border-t border-ink-700 hover:bg-ink-800 cursor-pointer"
              onClick={() => onOpen(it.file, it.line)}
            >
              {it.kind === 'http_route' ? (
                <>
                  <td class="px-2 py-1 w-20">
                    <span class={`pill ${METHOD_COLOR[it.method ?? ''] ?? 'text-ink-400 border-ink-700'}`}>
                      {it.method ?? '???'}
                    </span>
                  </td>
                  <td class="px-2 py-1 text-ink-300 font-medium">
                    <code class="text-accent-500">{it.name}</code>
                  </td>
                  <td class="px-2 py-1 text-ink-500 truncate max-w-[420px]">
                    {it.file}:{it.line}
                  </td>
                </>
              ) : (
                <>
                  <td class="px-2 py-1 w-20 text-[10px] text-ink-500">
                    {it.language}
                  </td>
                  <td class="px-2 py-1 text-ink-300 font-medium">
                    <code>{it.name}</code>
                    {it.signature && (
                      <span class="ml-2 text-ink-500 text-[10px]">
                        {ellipsize(it.signature, 80)}
                      </span>
                    )}
                  </td>
                  <td class="px-2 py-1 text-ink-500 truncate max-w-[420px]">
                    {it.file}:{it.line}
                  </td>
                </>
              )}
            </tr>
          ))}
          {rows.length > 200 && (
            <tr>
              <td colSpan={3} class="px-2 py-2 text-center text-[11px] text-ink-500">
                … {rows.length - 200} more (narrow your filter)
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

function ellipsize(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}
