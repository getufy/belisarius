import { useState } from 'preact/hooks';
import { useMutation } from '@tanstack/react-query';
import { api, SearchHit } from '../api';
import { useContextGet, useContextList } from '../data/queries';

export function ContextView({ path }: { path: string }) {
  const [selected, setSelected] = useState<string | null>(null);
  const [q, setQ] = useState('');
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [err, setErr] = useState<string | null>(null);

  const { data: listData, error: listErr } = useContextList(path);
  const artifacts = listData?.artifacts ?? [];

  const { data: content = null } = useContextGet(path, selected);

  const searchMut = useMutation({
    mutationFn: () => api.contextSearch(path, q.trim(), 20),
    onSuccess: (r) => {
      setHits(r.hits);
      setErr(null);
    },
    onError: (e) => setErr(e instanceof Error ? e.message : String(e)),
  });

  const indexMut = useMutation({
    mutationFn: () => api.contextIndex(path),
    onSuccess: (r) => alert(`indexed ${r.indexed_chunks} artifact chunks`),
    onError: (e) => setErr(e instanceof Error ? e.message : String(e)),
  });

  const busy = searchMut.isPending || indexMut.isPending;
  const runSearch = () => {
    if (q.trim()) searchMut.mutate();
  };
  const indexNow = () => indexMut.mutate();

  // Surface the list-fetch error like the legacy code did.
  if (listErr && !err) setErr(listErr instanceof Error ? listErr.message : String(listErr));

  return (
    <div class="space-y-4">
      <div class="card">
        <div class="flex items-baseline justify-between gap-2">
          <div>
            <p class="text-[10px] uppercase tracking-wider text-ink-500">context artifacts</p>
            <p class="text-[11px] text-ink-500">
              Non-code knowledge registered in <code>.belisarius/context_artifacts.json</code>.
              Use these for schemas, runbooks, API specs.
            </p>
          </div>
          <button class="btn" onClick={indexNow} disabled={busy}>
            {busy ? '…' : 'index into search'}
          </button>
        </div>
      </div>

      {err && <p class="card text-ink-err">{err}</p>}

      <div class="card">
        <label class="label">semantic search across artifacts</label>
        <div class="flex gap-2">
          <input
            class="field flex-1"
            placeholder="e.g. database schema for sessions"
            value={q}
            onInput={(e) => setQ((e.target as HTMLInputElement).value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') runSearch();
            }}
          />
          <button class="btn btn-primary" onClick={runSearch} disabled={busy}>
            search
          </button>
        </div>
        {hits.length > 0 && (
          <ul class="mt-3 space-y-1">
            {hits.map((h) => (
              <li key={h.chunk_id} class="border-b border-ink-800 py-1">
                <p class="text-sm">
                  <code class="text-accent-500">{h.name}</code>{' '}
                  <span class="text-ink-500 text-[10px]">{h.file}</span>
                </p>
                <p class="text-[11px] text-ink-400 truncate">{h.snippet.slice(0, 160)}</p>
              </li>
            ))}
          </ul>
        )}
      </div>

      {artifacts.length === 0 ? (
        <p class="card text-ink-500">
          No artifacts registered yet. Create{' '}
          <code class="text-accent-500">.belisarius/context_artifacts.json</code> with entries like:
          <pre class="mt-2 text-[11px] text-ink-300">
{`[
  { "name": "schema",
    "description": "Postgres schema for the sessions table.",
    "paths": ["db/schema.sql"] }
]`}
          </pre>
        </p>
      ) : (
        <div class="grid gap-3 md:grid-cols-3">
          <div class="md:col-span-1 space-y-2">
            {artifacts.map((a) => (
              <button
                key={a.name}
                class={`card text-left w-full ${selected === a.name ? 'border-accent-500' : ''}`}
                onClick={() => setSelected(a.name)}
              >
                <p class="text-sm text-accent-500">{a.name}</p>
                <p class="text-[11px] text-ink-500">{a.description}</p>
                <p class="text-[10px] text-ink-600 mt-1">{a.paths.length} path(s)</p>
              </button>
            ))}
          </div>
          <div class="md:col-span-2">
            {content ? (
              <div class="card space-y-2">
                <h3 class="text-base text-accent-500">{content.artifact.name}</h3>
                <p class="text-[11px] text-ink-500">{content.artifact.description}</p>
                {content.files.map((f) => (
                  <details key={f.path}>
                    <summary class="cursor-pointer text-sm text-ink-300">{f.path}</summary>
                    <pre class="mt-1 text-[11px] text-ink-300 max-h-80 overflow-auto">{f.content}</pre>
                  </details>
                ))}
              </div>
            ) : (
              <p class="card text-ink-500">Select an artifact to view.</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
