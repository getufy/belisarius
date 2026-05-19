import { useRef, useState } from 'preact/hooks';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api, IndexStatus, SearchHit } from '../api';
import { EmptyState } from './states/EmptyState';
import { ErrorState } from './states/ErrorState';
import { LoadingState } from './states/LoadingState';

export function SearchView({ path, onOpenSnippet }: { path: string; onOpenSnippet?: (file: string, line: number) => void }) {
  const [q, setQ] = useState('');
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [langFilter, setLangFilter] = useState<string>('');
  const [kindFilter, setKindFilter] = useState<string>('');
  const inputRef = useRef<HTMLInputElement>(null);
  const qc = useQueryClient();

  // Status — react-query polls while the indexer is running, then stops.
  const { data: status } = useQuery<IndexStatus | null>({
    queryKey: ['search_status', path],
    queryFn: () => api.searchStatus(path).catch(() => null),
    enabled: !!path,
    refetchInterval: (query) =>
      query.state.data?.state === 'indexing' ? 1500 : false,
  });

  // Search is button-triggered, not state-driven (the input changes while
  // the user is typing — we don't want to fire a request on each keystroke).
  // useMutation is the right shape: imperative, with built-in pending state.
  const searchMut = useMutation({
    mutationFn: () =>
      api.searchCode(path, q.trim(), {
        limit: 30,
        lang: langFilter || undefined,
        kind: kindFilter || undefined,
      }),
    onSuccess: (r) => {
      setHits(r.hits);
      setErr(null);
    },
    onError: (e) => setErr(e instanceof Error ? e.message : String(e)),
  });
  const busy = searchMut.isPending;
  const submit = () => {
    if (q.trim()) searchMut.mutate();
  };

  const reindexMut = useMutation({
    mutationFn: ({ full }: { full: boolean }) => api.searchReindex(path, { full }),
    onSuccess: (s) => {
      qc.setQueryData(['search_status', path], s);
      setErr(null);
    },
    onError: (e) => setErr(e instanceof Error ? e.message : String(e)),
  });
  const reindexing = reindexMut.isPending;
  const reindex = (full: boolean) => reindexMut.mutate({ full });

  return (
    <div class="space-y-4">
      <div class="card">
        <label class="label">Hybrid search (semantic + BM25 fused via RRF)</label>
        <div class="flex gap-2">
          <input
            ref={inputRef}
            class="field flex-1"
            placeholder="e.g. where do we parse SCIP indexes"
            value={q}
            onInput={(e) => setQ((e.target as HTMLInputElement).value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit();
            }}
          />
          <select class="field" value={langFilter} onChange={(e) => setLangFilter((e.target as HTMLSelectElement).value)}>
            <option value="">all langs</option>
            <option value="rust">rust</option>
            <option value="typescript">typescript</option>
            <option value="javascript">javascript</option>
            <option value="python">python</option>
            <option value="go">go</option>
          </select>
          <select class="field" value={kindFilter} onChange={(e) => setKindFilter((e.target as HTMLSelectElement).value)}>
            <option value="">all kinds</option>
            <option value="function">function</option>
            <option value="window">window</option>
            <option value="artifact">artifact</option>
          </select>
          <button class="btn btn-primary" onClick={submit} disabled={busy}>
            {busy ? 'searching…' : 'search'}
          </button>
        </div>
        <p class="mt-2 text-[11px] text-ink-500">
          Returns ranked code chunks across the whole repo. Prefer this over opening files blindly.
        </p>
      </div>

      <div class="card flex flex-wrap items-center gap-3 text-[11px]">
        <span class="text-ink-500">index</span>
        {status ? (
          <>
            <span class={statusColor(status.state)}>{status.state}</span>
            <span class="text-ink-500">
              {status.chunk_count} chunks · model {status.model}
            </span>
            {status.state === 'indexing' && (
              <span class="text-ink-500">
                {status.processed}/{status.total} files
              </span>
            )}
            {status.last_error && (
              <span class="text-ink-err">err: {status.last_error}</span>
            )}
          </>
        ) : (
          <span class="text-ink-500">unknown</span>
        )}
        <span class="ml-auto flex gap-2">
          <button class="btn" disabled={reindexing} onClick={() => reindex(false)}>
            {reindexing ? '…' : 'reindex'}
          </button>
          <button class="btn" disabled={reindexing} onClick={() => reindex(true)}>
            full rebuild
          </button>
        </span>
      </div>

      {err && (
        <ErrorState error={err} onRetry={() => { setErr(null); if (q.trim()) searchMut.mutate(); }} />
      )}

      {busy && hits.length === 0 && !err && (
        <LoadingState label="Searching…" />
      )}

      {!busy && !err && hits.length === 0 && searchMut.isSuccess && (
        <EmptyState
          title="No matches"
          hint="Try a broader query or remove language/kind filters."
        />
      )}

      {hits.length > 0 && (
        <ul class="space-y-2">
          {hits.map((h, i) => (
            <li key={h.chunk_id} class="card">
              <div class="flex items-baseline justify-between gap-2">
                <div class="min-w-0 flex-1">
                  <p class="text-sm text-ink-300 truncate">
                    <span class="text-ink-500">{i + 1}.</span>{' '}
                    <code class="text-accent-500">{h.name}</code>{' '}
                    <span class="text-ink-500">— {h.file}:{h.start_line}-{h.end_line}</span>
                  </p>
                </div>
                <div class="text-[10px] text-ink-500 whitespace-nowrap">
                  <span class="text-ink-400">{h.score.toFixed(4)}</span>{' '}
                  · bm25 {h.bm25_rank ?? '—'} · dense {h.dense_rank ?? '—'} · {h.lang}/{h.kind}
                </div>
              </div>
              <pre class="mt-2 text-[11px] text-ink-300 overflow-auto max-h-48 cursor-pointer hover:bg-ink-800"
                   onClick={() => onOpenSnippet?.(h.file, h.start_line)}>
                {h.snippet}
              </pre>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function statusColor(s: IndexStatus['state']): string {
  switch (s) {
    case 'idle':
      return 'text-green-400';
    case 'indexing':
      return 'text-ink-warning';
    case 'error':
      return 'text-ink-err';
  }
}
