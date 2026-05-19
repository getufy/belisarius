import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import cytoscape, { Core, ElementDefinition, EventObject } from 'cytoscape';
// @ts-expect-error - cytoscape-dagre has no published types
import dagre from 'cytoscape-dagre';
import { Graph, GraphNode, api } from '../api';

cytoscape.use(dagre);

const LANG_COLOR: Record<string, string> = {
  rust: '#d97a5d',
  typescript: '#0f62fe',
  javascript: '#bca84a',
  python: '#24a148',
  go: '#0098a6',
  java: '#8a3ffc',
  ruby: '#da1e28',
  default: '#525252',
};

const DEFAULT_MAX_NODES = 400;

type Layout = 'dagre' | 'cose' | 'concentric';

export function CytoscapeGraph({
  graph,
  cycleNodes,
  onNodeOpen,
  path,
  maxNodes = DEFAULT_MAX_NODES,
}: {
  graph: Graph;
  cycleNodes?: Set<string>;
  onNodeOpen?: (id: string) => void;
  /** Project path — needed for the right-click "blast radius" call. */
  path?: string;
  /** Cap on the number of nodes to render. The lowest-degree nodes are
   *  dropped first. Pass `Infinity` to render every node. */
  maxNodes?: number;
}) {
  // When the user clicks "Show all" we lift the cap entirely (Infinity), so
  // the warning chip disappears and the full graph paints.
  const [showAll, setShowAll] = useState(false);
  const effectiveMax = showAll ? Infinity : maxNodes;
  const containerRef = useRef<HTMLDivElement | null>(null);
  const cyRef = useRef<Core | null>(null);
  const [layout, setLayout] = useState<Layout>('dagre');
  const [hiddenLangs, setHiddenLangs] = useState<Set<string>>(new Set());
  const [highlightedFiles, setHighlightedFiles] = useState<Set<string>>(new Set());
  const [blastSource, setBlastSource] = useState<string | null>(null);

  const langStats = useMemo(() => {
    const c: Record<string, number> = {};
    for (const n of graph.nodes) c[n.language] = (c[n.language] ?? 0) + 1;
    return Object.entries(c).sort((a, b) => b[1] - a[1]);
  }, [graph]);

  const { shownNodes, shownEdges, omittedCount } = useMemo(() => {
    const filtered = graph.nodes.filter((n) => !hiddenLangs.has(n.language));
    const ranked = [...filtered].sort(
      (a, b) => b.in_degree + b.out_degree - (a.in_degree + a.out_degree),
    );
    const kept = ranked.slice(0, effectiveMax);
    const keptSet = new Set(kept.map((n) => n.id));
    const edges = graph.edges.filter((e) => keptSet.has(e.from) && keptSet.has(e.to));
    return { shownNodes: kept, shownEdges: edges, omittedCount: filtered.length - kept.length };
  }, [graph, hiddenLangs, effectiveMax]);

  const elements: ElementDefinition[] = useMemo(() => {
    const els: ElementDefinition[] = shownNodes.map((n: GraphNode) => ({
      data: {
        id: n.id,
        label: shortLabel(n.id),
        full: n.id,
        lang: n.language,
        loc: n.loc,
        inDeg: n.in_degree,
        outDeg: n.out_degree,
        entry: n.is_entry_point,
        cycle: cycleNodes?.has(n.id) ?? false,
      },
    }));
    for (const e of shownEdges) {
      els.push({ data: { id: `${e.from}->${e.to}`, source: e.from, target: e.to } });
    }
    return els;
  }, [shownNodes, shownEdges, cycleNodes]);

  // Build / rebuild the cytoscape instance only when elements change.
  useEffect(() => {
    if (!containerRef.current) return;
    if (cyRef.current) {
      cyRef.current.destroy();
      cyRef.current = null;
    }
    const cy = cytoscape({
      container: containerRef.current,
      elements,
      style: [
        {
          selector: 'node',
          style: {
            'background-color': (e: any) => LANG_COLOR[e.data('lang')] ?? LANG_COLOR.default,
            label: 'data(label)',
            color: '#c6c6c6',
            'font-size': 9,
            'text-margin-y': -4,
            'text-valign': 'top',
            'text-halign': 'center',
            'border-width': 1,
            'border-color': '#262626',
            width: (e: any) => 10 + Math.min(30, Math.sqrt(e.data('loc') ?? 0)),
            height: (e: any) => 10 + Math.min(30, Math.sqrt(e.data('loc') ?? 0)),
          },
        },
        {
          selector: 'node[?entry]',
          style: { 'border-width': 2, 'border-color': '#0f62fe' },
        },
        {
          selector: 'node[?cycle]',
          style: { 'border-color': '#da1e28', 'border-width': 3 },
        },
        {
          selector: 'edge',
          style: {
            width: 1,
            'line-color': '#404040',
            'target-arrow-color': '#525252',
            'target-arrow-shape': 'triangle',
            'arrow-scale': 0.6,
            'curve-style': 'bezier',
          },
        },
        {
          selector: '.blast',
          style: {
            'background-color': '#fa4d56',
            'border-color': '#fa4d56',
            'border-width': 3,
          },
        },
        {
          selector: '.blast-source',
          style: {
            'background-color': '#0f62fe',
            'border-color': '#0f62fe',
            'border-width': 4,
          },
        },
        {
          selector: 'edge.blast',
          style: { 'line-color': '#fa4d56', 'target-arrow-color': '#fa4d56', width: 2 },
        },
      ],
      layout: layoutOpts(layout),
      wheelSensitivity: 0.2,
    });
    cyRef.current = cy;
    cy.on('tap', 'node', (evt: EventObject) => {
      const id = evt.target.data('full');
      if (onNodeOpen) onNodeOpen(id);
    });
    cy.on('cxttap', 'node', (evt: EventObject) => {
      const id = evt.target.data('full');
      runBlastRadius(id);
    });
    return () => {
      cy.destroy();
      cyRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [elements]);

  // Re-run layout when the user switches it without rebuilding the graph.
  useEffect(() => {
    if (!cyRef.current) return;
    cyRef.current.layout(layoutOpts(layout)).run();
  }, [layout]);

  // Apply blast-radius highlight as CSS classes when the set changes.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.elements().removeClass('blast blast-source');
    if (blastSource) {
      cy.getElementById(blastSource).addClass('blast-source');
    }
    for (const f of highlightedFiles) {
      cy.getElementById(f).addClass('blast');
      cy.edges(`[target = "${cssEscape(f)}"], [source = "${cssEscape(f)}"]`).addClass('blast');
    }
  }, [highlightedFiles, blastSource]);

  async function runBlastRadius(fileId: string) {
    if (!path) return;
    // Pick a representative symbol defined in this file via symbols search;
    // for v1 we treat the *file id itself* as the impact target by looking
    // for the file's owning module symbol. If no SCIP index exists, we
    // fall back to graph-level reverse traversal.
    try {
      // Try SCIP first — look for a symbol whose display name matches the
      // file's base name. This is a heuristic; the full Impact tab gives
      // the user a real picker.
      const base = baseName(fileId);
      const matches = await api.symbolsSearch(path, base, 5).catch(() => []);
      const sym = matches[0]?.symbol;
      if (sym) {
        const impact = await api.impact(path, sym, 4);
        setBlastSource(fileId);
        setHighlightedFiles(new Set(impact.files));
        return;
      }
    } catch {
      /* fall through */
    }
    // Graph-only fallback: collect transitive in-neighbors.
    const adj = reverseAdjacency(graph);
    const seen = new Set<string>();
    const queue: string[] = [fileId];
    while (queue.length) {
      const id = queue.shift()!;
      if (seen.has(id)) continue;
      seen.add(id);
      for (const p of adj.get(id) ?? []) queue.push(p);
    }
    seen.delete(fileId);
    setBlastSource(fileId);
    setHighlightedFiles(seen);
  }

  function clearBlast() {
    setBlastSource(null);
    setHighlightedFiles(new Set());
  }

  return (
    <div class="space-y-3">
      <div class="card">
        <div class="flex flex-wrap items-center gap-3">
          <div class="flex items-center gap-2">
            <span class="text-[10px] uppercase tracking-wider text-ink-500">layout</span>
            {(['dagre', 'cose', 'concentric'] as Layout[]).map((l) => (
              <button
                key={l}
                onClick={() => setLayout(l)}
                class={`btn text-xs ${layout === l ? 'btn-primary' : ''}`}
              >
                {l}
              </button>
            ))}
          </div>
          <div class="flex items-center gap-2 flex-wrap">
            <span class="text-[10px] uppercase tracking-wider text-ink-500">languages</span>
            {langStats.map(([lang, count]) => {
              const hidden = hiddenLangs.has(lang);
              return (
                <button
                  key={lang}
                  onClick={() => {
                    const next = new Set(hiddenLangs);
                    hidden ? next.delete(lang) : next.add(lang);
                    setHiddenLangs(next);
                  }}
                  class={`btn text-xs ${hidden ? 'opacity-40' : ''}`}
                  style={{ borderLeft: `3px solid ${LANG_COLOR[lang] ?? LANG_COLOR.default}` }}
                >
                  {lang} <span class="text-ink-500">{count}</span>
                </button>
              );
            })}
          </div>
          {blastSource && (
            <button class="btn text-xs ml-auto" onClick={clearBlast}>
              clear blast radius
            </button>
          )}
        </div>
        <p class="mt-2 text-[11px] text-ink-500">
          Click a node to open its DSM. Right-click a node to highlight its blast radius
          (transitive callers via SCIP if available, else reverse graph traversal).
          {omittedCount > 0 && (
            <span class="ml-2 text-ink-600">{omittedCount} low-degree nodes omitted.</span>
          )}
        </p>
        {omittedCount > 0 && !showAll && (
          <div class="text-xs text-ink-warning bg-ink-warningSubtle border border-ink-warning px-2 py-1 rounded mb-2">
            Showing top {maxNodes} of {shownNodes.length + omittedCount} nodes (filtered by degree).{' '}
            <button class="underline" onClick={() => setShowAll(true)}>
              Show all
            </button>
          </div>
        )}
        <div class="relative">
          <div class="absolute top-2 right-2 z-10 flex gap-1">
            <button
              type="button"
              class="btn-mini"
              title="Zoom to fit"
              aria-label="Zoom to fit"
              onClick={() => cyRef.current?.fit()}
            >
              {'\u{1F50D}'}
            </button>
            <button
              type="button"
              class="btn-mini"
              title="Zoom in"
              aria-label="Zoom in"
              onClick={() => {
                const cy = cyRef.current;
                if (!cy) return;
                cy.zoom(cy.zoom() * 1.2);
                cy.center();
              }}
            >
              {'＋'}
            </button>
            <button
              type="button"
              class="btn-mini"
              title="Zoom out"
              aria-label="Zoom out"
              onClick={() => {
                const cy = cyRef.current;
                if (!cy) return;
                cy.zoom(cy.zoom() / 1.2);
                cy.center();
              }}
            >
              {'−'}
            </button>
          </div>
          <div ref={containerRef} class="card" style={{ height: '620px', padding: 0 }} />
        </div>
        <ul class="flex flex-wrap gap-2 text-xs text-ink-400 mt-2">
          {langStats.slice(0, 5).map(([lang, count]) => (
            <li key={lang} class="inline-flex items-center gap-1.5">
              <span
                aria-hidden="true"
                class="inline-block w-3 h-3"
                style={{ backgroundColor: LANG_COLOR[lang] ?? LANG_COLOR.default }}
              />
              <span>
                {lang} <span class="text-ink-500">{count}</span>
              </span>
            </li>
          ))}
          <li class="inline-flex items-center gap-1.5">
            <span
              aria-hidden="true"
              class="inline-block w-3 h-3 border-2"
              style={{ borderColor: '#0f62fe', backgroundColor: 'transparent' }}
            />
            <span>entry point</span>
          </li>
          {cycleNodes && cycleNodes.size > 0 && (
            <li class="inline-flex items-center gap-1.5">
              <span
                aria-hidden="true"
                class="inline-block w-3 h-3 border-2"
                style={{ borderColor: '#da1e28', backgroundColor: 'transparent' }}
              />
              <span>cycle member</span>
            </li>
          )}
        </ul>
      </div>
    </div>
  );
}

function layoutOpts(layout: Layout) {
  switch (layout) {
    case 'dagre':
      return {
        name: 'dagre',
        rankDir: 'LR',
        nodeSep: 30,
        rankSep: 60,
        animate: false,
        fit: true,
      } as any;
    case 'cose':
      return {
        name: 'cose',
        animate: false,
        nodeRepulsion: 6000,
        idealEdgeLength: 80,
        fit: true,
      } as any;
    case 'concentric':
      return {
        name: 'concentric',
        animate: false,
        concentric: (n: any) => n.data('inDeg') + n.data('outDeg'),
        levelWidth: () => 1,
        fit: true,
      } as any;
  }
}

function shortLabel(id: string): string {
  const parts = id.split('/');
  return parts[parts.length - 1] || id;
}

function baseName(id: string): string {
  const last = shortLabel(id);
  return last.replace(/\.(rs|ts|tsx|js|jsx|py|go|java|rb)$/, '');
}

function reverseAdjacency(graph: Graph): Map<string, string[]> {
  const m = new Map<string, string[]>();
  for (const e of graph.edges) {
    if (!m.has(e.to)) m.set(e.to, []);
    m.get(e.to)!.push(e.from);
  }
  return m;
}

function cssEscape(s: string): string {
  return s.replace(/(["\\])/g, '\\$1');
}
