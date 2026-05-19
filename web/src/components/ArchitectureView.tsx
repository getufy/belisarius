import { useEffect, useMemo, useState } from 'preact/hooks';
import { ArchitectureGraph, ArchitectureModule } from '../api';
import { useArchitectureGraph, useArchitectureModule } from '../data/queries';
import { CodeView } from './CodeView';
import { GraphCanvas } from './GraphCanvas';

export function ArchitectureView({ path }: { path: string }) {
  const [view, setView] = useState<'module' | 'file'>('module');
  const [groupDepth, setGroupDepth] = useState(2);
  const [maxNodes, setMaxNodes] = useState(60);
  const [fullscreen, setFullscreen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  const { data, error } = useArchitectureGraph(path, { view, maxNodes, groupDepth });
  const err = error ? String(error) : null;

  // Stable id → display path map so we can fetch /api/architecture/module.
  const idToPath = useMemo(() => {
    const m = new Map<string, string>();
    for (const n of data?.nodes ?? []) m.set(n.id, n.label);
    return m;
  }, [data]);

  // Stable path → id (for jumping between modules via the side panel).
  const pathToId = useMemo(() => {
    const m = new Map<string, string>();
    for (const n of data?.nodes ?? []) m.set(n.label, n.id);
    return m;
  }, [data]);

  // Reset selection on path/view change so we don't show a stale node detail.
  useEffect(() => {
    setSelectedId(null);
  }, [path, view, maxNodes, groupDepth]);

  // Module detail fetch — only in module view, only when a node is selected.
  const modulePathForFetch =
    selectedId && view === 'module' ? idToPath.get(selectedId) ?? null : null;
  const { data: moduleDetailData } = useArchitectureModule(path, modulePathForFetch, groupDepth);
  const moduleDetail = moduleDetailData as ArchitectureModule | undefined ?? null;

  // Esc exits fullscreen → clears selection.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      if (selectedId) setSelectedId(null);
      else if (fullscreen) setFullscreen(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [fullscreen, selectedId]);

  const selectedModulePath =
    selectedId && view === 'module' ? idToPath.get(selectedId) ?? null : null;

  return (
    <>
      <div class={fullscreen ? 'fixed inset-0 z-40 bg-white flex flex-col' : 'space-y-4'}>
        <div class={fullscreen ? 'p-3 border-b border-ink-700' : ''}>
          <Controls
            view={view}
            setView={setView}
            groupDepth={groupDepth}
            setGroupDepth={setGroupDepth}
            maxNodes={maxNodes}
            setMaxNodes={setMaxNodes}
            data={data ?? null}
            fullscreen={fullscreen}
            setFullscreen={setFullscreen}
          />
        </div>

        {err && <p class="card text-ink-err">{err}</p>}
        {!data && !err && (
          <p class={fullscreen ? 'p-3 text-ink-500' : 'card text-ink-500'}>
            Loading architecture…
          </p>
        )}

        {data && (
          <div class={`flex ${fullscreen ? 'flex-1 min-h-0' : 'gap-3'}`}>
            <div class={fullscreen ? 'flex-1 min-h-0' : 'flex-1'}>
              <GraphCanvas
                nodes={data.nodes}
                edges={data.edges}
                layout={view === 'module' ? 'dagre' : 'cose'}
                selectedId={selectedId}
                onSelect={setSelectedId}
                fullscreen={fullscreen}
              />
            </div>
            {selectedModulePath && (
              <DetailPanel
                module={selectedModulePath}
                detail={moduleDetail}
                onClose={() => setSelectedId(null)}
                onJumpModule={(p) => {
                  const id = pathToId.get(p);
                  if (id) setSelectedId(id);
                }}
                onPreviewFile={(file) => setPreview({ file, line: 1 })}
                fullscreen={fullscreen}
              />
            )}
          </div>
        )}

        {!fullscreen && data && data.directory_summary.length > 0 && (
          <div class="card">
            <h3 class="text-xs uppercase tracking-wider text-ink-500 mb-2">
              Directory groups — sorted by cross-module coupling
            </h3>
            <table class="w-full text-xs">
              <thead class="text-ink-500 uppercase tracking-wider">
                <tr>
                  <th class="text-left px-2 py-1">group</th>
                  <th class="text-right px-2 py-1">files</th>
                  <th class="text-right px-2 py-1">loc</th>
                  <th class="text-right px-2 py-1" title="Edges crossing module boundaries — high = leaky module">cross</th>
                </tr>
              </thead>
              <tbody>
                {data.directory_summary.map((d) => (
                  <tr
                    key={d.path}
                    class="border-t border-ink-700 hover:bg-ink-800 cursor-pointer"
                    onClick={() => {
                      const id = pathToId.get(d.path);
                      if (id) setSelectedId(id);
                    }}
                    title={`in: ${d.in_edges} · out: ${d.out_edges}`}
                  >
                    <td class="px-2 py-1 text-ink-300 font-medium">
                      <code class="text-accent-500">{d.path}</code>
                    </td>
                    <td class="px-2 py-1 text-right text-ink-400">{d.files}</td>
                    <td class="px-2 py-1 text-right text-ink-400">{d.loc.toLocaleString()}</td>
                    <td
                      class={`px-2 py-1 text-right ${
                        d.cross_edges > 0 ? 'text-orange-400' : 'text-ink-500'
                      }`}
                    >
                      {d.cross_edges}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
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
    </>
  );
}

function Controls({
  view,
  setView,
  groupDepth,
  setGroupDepth,
  maxNodes,
  setMaxNodes,
  data,
  fullscreen,
  setFullscreen,
}: {
  view: 'module' | 'file';
  setView: (v: 'module' | 'file') => void;
  groupDepth: number;
  setGroupDepth: (n: number) => void;
  maxNodes: number;
  setMaxNodes: (n: number) => void;
  data: ArchitectureGraph | null;
  fullscreen: boolean;
  setFullscreen: (v: boolean) => void;
}) {
  return (
    <div class={fullscreen ? 'flex flex-wrap items-end gap-3' : 'card flex flex-wrap items-end gap-3'}>
      <div class="flex-1 min-w-[200px]">
        <h2 class="text-sm uppercase tracking-wider text-ink-500">
          Architecture overview
        </h2>
        <p class="text-xs text-ink-500">
          {view === 'module'
            ? 'Files aggregated into directory modules — edge labels show import count. Click a module for details.'
            : 'Per-file dependency graph. Click any node to inspect.'}
          {data && (
            <>
              {' '}{data.nodes_total} files · {data.edges_total} edges.
            </>
          )}
        </p>
      </div>
      <div class="flex gap-1">
        <button
          class={`pill ${view === 'module' ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
          onClick={() => setView('module')}
        >
          module view
        </button>
        <button
          class={`pill ${view === 'file' ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
          onClick={() => setView('file')}
        >
          file view
        </button>
      </div>
      <div>
        <label class="label">Depth</label>
        <select
          class="field"
          value={groupDepth}
          onChange={(e) => setGroupDepth(parseInt((e.target as HTMLSelectElement).value, 10))}
        >
          <option value="1">top-level</option>
          <option value="2">two levels</option>
          <option value="3">three levels</option>
        </select>
      </div>
      {view === 'file' && (
        <div>
          <label class="label">Max nodes</label>
          <input
            class="field w-20"
            type="number"
            min="20"
            max="500"
            value={maxNodes}
            onInput={(e) => setMaxNodes(parseInt((e.target as HTMLInputElement).value, 10) || 60)}
          />
        </div>
      )}
      <button
        class="btn text-xs"
        onClick={() => setFullscreen(!fullscreen)}
        title={fullscreen ? 'Exit fullscreen (esc)' : 'Expand to fullscreen'}
      >
        {fullscreen ? 'exit' : 'fullscreen'}
      </button>
    </div>
  );
}

function DetailPanel({
  module,
  detail,
  onClose,
  onJumpModule,
  onPreviewFile,
  fullscreen,
}: {
  module: string;
  detail: ArchitectureModule | null;
  onClose: () => void;
  onJumpModule: (path: string) => void;
  onPreviewFile: (file: string) => void;
  fullscreen: boolean;
}) {
  return (
    <div
      class={`flex flex-col border border-ink-700 bg-white ${
        fullscreen ? 'w-[360px] shrink-0 overflow-auto' : 'card w-[360px] max-h-[560px] overflow-auto p-0'
      }`}
    >
      <header class="flex items-baseline justify-between gap-2 border-b border-ink-700 px-3 py-2">
        <div class="min-w-0">
          <p class="text-[10px] uppercase tracking-wider text-ink-500">module</p>
          <code class="text-sm text-accent-500 break-all">{module}</code>
        </div>
        <button
          class="border border-ink-700 px-2 py-0.5 text-[11px] text-ink-300 hover:bg-ink-800"
          onClick={onClose}
        >
          ✕
        </button>
      </header>

      {!detail ? (
        <p class="p-3 text-sm text-ink-500">Loading details…</p>
      ) : (
        <div class="p-3 space-y-4 text-xs">
          <p class="text-ink-500">
            {detail.file_count} files · {detail.total_loc.toLocaleString()} loc
          </p>

          {detail.outgoing.length > 0 && (
            <section>
              <h4 class="text-[10px] uppercase tracking-wider text-ink-500 mb-1">
                imports → ({detail.outgoing.length})
              </h4>
              <ul class="space-y-0.5">
                {detail.outgoing.map((o) => (
                  <li
                    key={o.path}
                    class="cursor-pointer hover:bg-ink-800 px-1 py-0.5 flex items-center gap-2"
                    onClick={() => onJumpModule(o.path)}
                    title="Jump to this module"
                  >
                    <code class="text-accent-500 truncate flex-1">{o.path}</code>
                    <span class="pill text-ink-400 shrink-0">×{o.weight}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {detail.incoming.length > 0 && (
            <section>
              <h4 class="text-[10px] uppercase tracking-wider text-ink-500 mb-1">
                ← imported by ({detail.incoming.length})
              </h4>
              <ul class="space-y-0.5">
                {detail.incoming.map((o) => (
                  <li
                    key={o.path}
                    class="cursor-pointer hover:bg-ink-800 px-1 py-0.5 flex items-center gap-2"
                    onClick={() => onJumpModule(o.path)}
                  >
                    <code class="text-accent-500 truncate flex-1">{o.path}</code>
                    <span class="pill text-ink-400 shrink-0">×{o.weight}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {detail.outgoing.length === 0 && detail.incoming.length === 0 && (
            <p class="text-ink-500 italic">This module has no cross-module edges.</p>
          )}

          {detail.files.length > 0 && (
            <section>
              <h4 class="text-[10px] uppercase tracking-wider text-ink-500 mb-1">
                files ({detail.files.length})
              </h4>
              <ul class="space-y-0.5">
                {detail.files.map((f) => (
                  <li
                    key={f.path}
                    class="cursor-pointer hover:bg-ink-800 px-1 py-0.5 flex items-center gap-2"
                    onClick={() => onPreviewFile(f.path)}
                  >
                    <code class="text-ink-300 truncate flex-1">{f.path}</code>
                    <span class="text-ink-500 shrink-0">{f.loc}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>
      )}
    </div>
  );
}
