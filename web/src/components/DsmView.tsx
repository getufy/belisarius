import { useEffect, useMemo, useState } from 'preact/hooks';
import { Graph } from '../api';
import { useFileDsm } from '../data/queries';
import { CodeView } from './CodeView';

export function DsmView({
  path,
  graph,
  initialFile,
}: {
  path: string;
  graph: Graph;
  initialFile?: string;
}) {
  const allFiles = useMemo(
    () => graph.nodes.map((n) => n.id).sort(),
    [graph]
  );

  const [file, setFile] = useState<string>(initialFile ?? pickDefault(graph));
  const [search, setSearch] = useState('');
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  useEffect(() => {
    if (initialFile) setFile(initialFile);
  }, [initialFile]);

  const { data: dsm, error } = useFileDsm(path, file);
  const err = error ? String(error) : null;

  const filteredFiles = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return allFiles.slice(0, 300);
    return allFiles.filter((f) => f.toLowerCase().includes(needle)).slice(0, 300);
  }, [allFiles, search]);

  return (
    <div class="space-y-4">
      <div class="card text-xs text-ink-500">
        Pick a file on the left — the right pane shows everything that imports it
        and everything it imports, with line numbers and the function each call
        site lives in. This is the data the LLM uses to reason about call flow.
      </div>

      <div class="grid gap-3 lg:grid-cols-[minmax(260px,_2fr)_5fr]">
        <div class="card flex flex-col gap-2 max-h-[680px] p-2">
          <input
            class="field"
            placeholder="filter files…"
            value={search}
            onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
          />
          <ul class="flex-1 overflow-auto text-xs scrollbar-thin">
            {filteredFiles.map((f) => (
              <li key={f}>
                <button
                  class={`w-full text-left px-2 py-1 truncate ${
                    f === file
                      ? 'bg-accent-500/10 text-accent-500'
                      : 'text-ink-300 hover:bg-ink-800'
                  }`}
                  onClick={() => setFile(f)}
                  title={f}
                >
                  {f}
                </button>
              </li>
            ))}
            {filteredFiles.length === 300 && (
              <li class="text-[11px] text-ink-500 px-2 py-2">… narrow the filter to see more</li>
            )}
          </ul>
        </div>

        <div class="space-y-3">
          {err && <p class="card text-ink-err">{err}</p>}
          {!dsm && !err && <p class="card text-ink-500">Loading DSM for {file}…</p>}

          {dsm && (
            <>
              <div class="card flex flex-wrap items-baseline justify-between gap-3">
                <div class="min-w-0">
                  <p class="text-[10px] uppercase tracking-wider text-ink-500">
                    {dsm.language} · {dsm.loc} loc · {dsm.functions.length} functions
                  </p>
                  <p class="text-sm text-ink-300 truncate">
                    <code class="text-accent-500">{dsm.file}</code>
                  </p>
                </div>
                <div class="flex items-center gap-2">
                  <span class="pill text-ink-400">in {dsm.in_degree}</span>
                  <span class="pill text-ink-400">out {dsm.out_degree}</span>
                </div>
              </div>

              <div class="grid gap-3 md:grid-cols-2">
                <Panel label={`Inbound — ${dsm.inbound.length} edges`}>
                  {dsm.inbound.length === 0 ? (
                    <p class="text-sm text-ink-500">No file imports this.</p>
                  ) : (
                    <ul class="space-y-1">
                      {dsm.inbound.map((i, idx) => (
                        <li
                          key={idx}
                          class="text-xs hover:bg-ink-800 -mx-1 px-1 cursor-pointer"
                          onClick={() => setPreview({ file: i.from, line: i.line })}
                          title="Preview the import line"
                        >
                          <code class="text-accent-500">{i.from}</code>
                          <span class="text-ink-500">:{i.line}</span>
                          {i.in_function && (
                            <span class="text-ink-400"> · in <code class="text-ink-300">{i.in_function}</code></span>
                          )}
                        </li>
                      ))}
                    </ul>
                  )}
                </Panel>

                <Panel label={`Outbound — ${dsm.outbound.length} edges`}>
                  {dsm.outbound.length === 0 ? (
                    <p class="text-sm text-ink-500">This file imports nothing in-tree.</p>
                  ) : (
                    <ul class="space-y-1">
                      {dsm.outbound.map((o, idx) => (
                        <li
                          key={idx}
                          class="text-xs hover:bg-ink-800 -mx-1 px-1 cursor-pointer flex items-baseline gap-1"
                          onClick={() => setPreview({ file: dsm.file, line: o.line })}
                          title="Preview the import line in this file"
                        >
                          <span class="text-ink-500">{o.line || '?'}:</span>
                          <code class="text-accent-500">{o.to}</code>
                          <button
                            class="ml-auto text-[10px] text-ink-600 hover:text-accent-500"
                            onClick={(e) => {
                              e.stopPropagation();
                              setFile(o.to);
                            }}
                            title="Jump to that file's DSM"
                          >
                            →
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                </Panel>
              </div>

              {dsm.externals.length > 0 && (
                <Panel label={`External imports — ${dsm.externals.length}`}>
                  <ul class="grid gap-1 md:grid-cols-2">
                    {dsm.externals.map((e, idx) => (
                      <li
                        key={idx}
                        class="text-xs hover:bg-ink-800 -mx-1 px-1 cursor-pointer flex items-baseline gap-1"
                        onClick={() => setPreview({ file: dsm.file, line: e.line })}
                        title="Preview"
                      >
                        <span class="text-ink-500">{e.line || '?'}:</span>
                        <code class="text-ink-300 truncate">{e.spec}</code>
                      </li>
                    ))}
                  </ul>
                </Panel>
              )}

              {dsm.functions.length > 0 && (
                <Panel label={`Functions defined — ${dsm.functions.length}`}>
                  <ul class="grid gap-1 md:grid-cols-2">
                    {dsm.functions.map((f, idx) => (
                      <li
                        key={idx}
                        class="text-xs hover:bg-ink-800 -mx-1 px-1 cursor-pointer flex items-baseline gap-2"
                        onClick={() => setPreview({ file: dsm.file, line: f.start_line })}
                      >
                        <span class={`shrink-0 ${ccColor(f.cyclomatic)}`}>cc {f.cyclomatic}</span>
                        <code class="text-ink-300 truncate">{f.name}</code>
                        <span class="text-ink-500 ml-auto shrink-0">:{f.start_line}</span>
                      </li>
                    ))}
                  </ul>
                </Panel>
              )}
            </>
          )}
        </div>
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

function Panel({ label, children }: { label: string; children: preact.ComponentChildren }) {
  return (
    <div class="card">
      <h3 class="text-[10px] uppercase tracking-wider text-ink-500 mb-2">{label}</h3>
      {children}
    </div>
  );
}

function ccColor(cc: number): string {
  if (cc >= 20) return 'text-ink-err';
  if (cc >= 10) return 'text-orange-400';
  if (cc >= 5) return 'text-ink-warning';
  return 'text-ink-400';
}

function pickDefault(graph: Graph): string {
  // Pick the most-connected file as a sensible landing target.
  let best = graph.nodes[0];
  if (!best) return '';
  for (const n of graph.nodes) {
    if ((n.in_degree + n.out_degree) > (best.in_degree + best.out_degree)) best = n;
  }
  return best.id;
}
