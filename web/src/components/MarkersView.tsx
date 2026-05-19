import { useMemo, useState } from 'preact/hooks';
import { useMarkers } from '../data/queries';
import { CodeView } from './CodeView';

const KIND_COLORS: Record<string, string> = {
  TODO: 'text-ink-warning border-ink-warning/30',
  FIXME: 'text-ink-err border-ink-err/30',
  HACK: 'text-orange-400 border-orange-400/30',
  XXX: 'text-orange-400 border-orange-400/30',
  NOTE: 'text-ink-400 border-ink-700',
};

export function MarkersView({ path }: { path: string }) {
  const { data, error } = useMarkers(path);
  const markers = data?.markers ?? null;
  const limited = data?.limited ?? false;
  const err = error ? String(error) : null;
  const [kindFilter, setKindFilter] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  const kindCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const m of markers ?? []) counts[m.kind] = (counts[m.kind] ?? 0) + 1;
    return counts;
  }, [markers]);

  const filtered = useMemo(() => {
    if (!markers) return [];
    const needle = search.trim().toLowerCase();
    return markers
      .filter((m) => !kindFilter || m.kind === kindFilter)
      .filter(
        (m) =>
          !needle ||
          m.text.toLowerCase().includes(needle) ||
          m.file.toLowerCase().includes(needle)
      );
  }, [markers, kindFilter, search]);

  if (err) return <p class="card text-ink-err">{err}</p>;
  if (!markers) return <p class="card text-ink-500">Scanning for TODO / FIXME / HACK / XXX / NOTE…</p>;

  return (
    <div class="space-y-3">
      <div class="card flex flex-wrap items-end gap-3">
        <div class="flex-1 min-w-[200px]">
          <label class="label">Search</label>
          <input
            class="field"
            placeholder="text or file…"
            value={search}
            onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
          />
        </div>
        <div class="flex items-end gap-1">
          <button
            class={`pill ${kindFilter === null ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
            onClick={() => setKindFilter(null)}
          >
            all {markers.length}
          </button>
          {Object.entries(kindCounts)
            .sort((a, b) => b[1] - a[1])
            .map(([kind, n]) => (
              <button
                key={kind}
                class={`pill ${kindFilter === kind ? 'border-accent-500 text-accent-500' : KIND_COLORS[kind] ?? 'text-ink-500 hover:text-ink-300'}`}
                onClick={() => setKindFilter(kindFilter === kind ? null : kind)}
              >
                {kind} {n}
              </button>
            ))}
        </div>
        <div class="text-xs text-ink-500 ml-auto">
          {filtered.length} / {markers.length}
          {limited && <span class="ml-2 text-ink-warning">limit hit</span>}
        </div>
      </div>

      <div class="card max-h-[640px] overflow-auto">
        {filtered.length === 0 ? (
          <p class="text-sm text-ink-500">No markers match.</p>
        ) : (
          <ul class="space-y-1 text-xs">
            {filtered.slice(0, 1000).map((m, i) => (
              <li
                key={`${m.file}:${m.line}:${i}`}
                class="border-b border-ink-700 py-1 cursor-pointer hover:bg-ink-800 -mx-1 px-1 flex items-baseline gap-2"
                onClick={() => setPreview({ file: m.file, line: m.line })}
                title="Click to preview"
              >
                <span class={`pill ${KIND_COLORS[m.kind] ?? 'text-ink-500'}`}>{m.kind}</span>
                <span class="text-ink-500 shrink-0">
                  {m.file}:{m.line}
                </span>
                <span class="text-ink-300 truncate">{m.text || <em class="text-ink-500">(no body)</em>}</span>
              </li>
            ))}
          </ul>
        )}
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
