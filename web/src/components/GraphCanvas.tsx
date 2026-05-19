import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { languageColors, languageStroke } from './lang-palette';

// We model only the slice of the Cytoscape API we actually call. Loaded
// dynamically from a CDN — TypeScript can't see the real types.
type CyEvent = { target: { id: () => string } };
type CyEles = {
  removeClass: (cls: string) => CyEles;
  addClass: (cls: string) => CyEles;
  data: (key: string) => unknown;
  forEach: (fn: (el: CyEles) => void) => void;
  connectedEdges: () => CyEles;
  connectedNodes: () => CyEles;
};
type CyCollection = CyEles;
type Cy = {
  on: (event: string, selector: string | ((e: CyEvent) => void), cb?: (e: CyEvent) => void) => void;
  off: (event: string) => void;
  elements: () => CyCollection;
  getElementById: (id: string) => CyEles & { length: number; outgoers: () => CyEles; incomers: () => CyEles };
  fit: (padding?: number) => void;
  resize: () => void;
  destroy: () => void;
  batch: (fn: () => void) => void;
  zoom: () => number;
  layout: (opts: unknown) => { run: () => void };
};
type CytoscapeFactory = (opts: unknown) => Cy;

let cyPromise: Promise<CytoscapeFactory> | null = null;
function loadCytoscape(): Promise<CytoscapeFactory> {
  if (!cyPromise) {
    const dynImport = (u: string) =>
      (new Function('u', 'return import(u)') as (u: string) => Promise<any>)(u);
    // esm.sh bundles cytoscape-dagre's internal `require('dagre')` correctly;
    // jsdelivr's `+esm` ships a broken shape where `dagre.graphlib` is missing
    // at runtime ("Cannot read properties of undefined (reading 'Graph')").
    // Pinning `?deps=` keeps cytoscape-dagre on the same cytoscape instance.
    cyPromise = Promise.all([
      dynImport('https://esm.sh/cytoscape@3.30.4'),
      dynImport('https://esm.sh/cytoscape-dagre@2.5.0?deps=cytoscape@3.30.4'),
    ]).then(([cy, dagreLayout]: any[]) => {
      const cytoscape: CytoscapeFactory = cy.default;
      const ext = dagreLayout.default ?? dagreLayout;
      try {
        ext(cytoscape);
      } catch {
        /* already registered */
      }
      return cytoscape;
    });
  }
  return cyPromise;
}

export type GraphNode = {
  id: string;
  label: string;
  sublabel?: string;
  language?: string;
  is_entry?: boolean;
};

export type GraphEdge = {
  source: string;
  target: string;
  weight?: number;
};

export type GraphCanvasProps = {
  nodes: GraphNode[];
  edges: GraphEdge[];
  layout: 'dagre' | 'cose';
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  fullscreen?: boolean;
  /** Cap on the number of nodes to render. The lowest-degree nodes are
   *  dropped first. Defaults to 200. Pass `Infinity` to render every node. */
  maxNodes?: number;
};

const DEFAULT_MAX_NODES = 200;

export function GraphCanvas({
  nodes,
  edges,
  layout,
  selectedId,
  onSelect,
  fullscreen,
  maxNodes = DEFAULT_MAX_NODES,
}: GraphCanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<Cy | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // "Show all" lifts the maxNodes cap entirely for the current session.
  const [showAll, setShowAll] = useState(false);
  const effectiveMax = showAll ? Infinity : maxNodes;

  // Cap nodes by degree when the total exceeds the cap. Edges where either
  // endpoint is filtered out are dropped so cytoscape doesn't render dangling
  // arrows pointing at nothing.
  const { boundedNodes, boundedEdges, omittedCount } = useMemo(() => {
    if (nodes.length <= effectiveMax) {
      return { boundedNodes: nodes, boundedEdges: edges, omittedCount: 0 };
    }
    const degree = new Map<string, number>();
    for (const e of edges) {
      degree.set(e.source, (degree.get(e.source) ?? 0) + 1);
      degree.set(e.target, (degree.get(e.target) ?? 0) + 1);
    }
    const ranked = [...nodes].sort(
      (a, b) => (degree.get(b.id) ?? 0) - (degree.get(a.id) ?? 0),
    );
    const kept = ranked.slice(0, effectiveMax);
    const keptSet = new Set(kept.map((n) => n.id));
    const survivingEdges = edges.filter((e) => keptSet.has(e.source) && keptSet.has(e.target));
    return {
      boundedNodes: kept,
      boundedEdges: survivingEdges,
      omittedCount: nodes.length - kept.length,
    };
  }, [nodes, edges, effectiveMax]);

  // Languages present in the bounded subset, sorted by file count (top 5 in legend).
  const langStats = useMemo(() => {
    const c: Record<string, number> = {};
    for (const n of boundedNodes) {
      const lang = n.language ?? 'unknown';
      c[lang] = (c[lang] ?? 0) + 1;
    }
    return Object.entries(c).sort((a, b) => b[1] - a[1]);
  }, [boundedNodes]);

  // Cytoscape stylesheet — Carbon palette, language-tinted nodes, selection
  // and direction classes for the focus mode.
  const stylesheet = useMemo(
    () => [
      {
        selector: 'node',
        style: {
          'background-color': 'data(fill)',
          'border-color': 'data(stroke)',
          'border-width': 1,
          shape: 'round-rectangle',
          'corner-radius': 0,
          width: 'label',
          height: 'label',
          padding: '12px',
          'text-wrap': 'wrap',
          'text-max-width': 200,
          label: 'data(display)',
          color: '#161616',
          'font-family': '"IBM Plex Sans", system-ui, sans-serif',
          'font-size': 12,
          'font-weight': 400,
          'text-valign': 'center',
          'text-halign': 'center',
        },
      },
      {
        selector: 'node[?is_entry]',
        style: { 'border-width': 2.5, 'border-color': '#0f62fe' },
      },
      {
        selector: 'node:selected',
        style: { 'border-width': 3, 'border-color': '#0f62fe' },
      },
      {
        selector: 'node.dimmed',
        style: { opacity: 0.18 },
      },
      {
        selector: 'edge',
        style: {
          'line-color': '#525252',
          width: 'mapData(weight, 1, 10, 1, 4)',
          'target-arrow-color': '#525252',
          'target-arrow-shape': 'triangle',
          'arrow-scale': 1.1,
          'curve-style': 'bezier',
          'font-family': '"IBM Plex Mono", ui-monospace, monospace',
          'font-size': 10,
          color: '#161616',
          'text-background-color': '#ffffff',
          'text-background-opacity': 1,
          'text-background-padding': '2px',
          'text-border-width': 1,
          'text-border-color': '#e0e0e0',
        },
      },
      {
        selector: 'edge[?label]',
        style: { label: 'data(label)' },
      },
      {
        selector: 'edge.out',
        style: {
          'line-color': '#0f62fe',
          'target-arrow-color': '#0f62fe',
          width: 2.4,
        },
      },
      {
        selector: 'edge.in',
        style: {
          'line-color': '#24a148',
          'target-arrow-color': '#24a148',
          width: 2.4,
        },
      },
      {
        selector: 'edge.dimmed',
        style: { opacity: 0.12 },
      },
    ],
    []
  );

  // Cytoscape element format. Each node carries its language colors as data
  // attributes so the stylesheet can paint without a class explosion. Uses the
  // bounded sets so cytoscape never sees more than `effectiveMax` nodes.
  const elements = useMemo(() => {
    const nodeEls = boundedNodes.map((n) => {
      const colors = languageColors(n.language);
      const display = n.sublabel ? `${n.label}\n${n.sublabel}` : n.label;
      return {
        group: 'nodes',
        data: {
          id: n.id,
          label: n.label,
          display,
          fill: colors.fill,
          stroke: colors.stroke,
          is_entry: n.is_entry ?? false,
        },
      };
    });
    const edgeEls = boundedEdges.map((e) => ({
      group: 'edges',
      data: {
        id: `${e.source}->${e.target}`,
        source: e.source,
        target: e.target,
        weight: e.weight ?? 1,
        label: (e.weight ?? 1) > 1 ? String(e.weight) : '',
      },
    }));
    return [...nodeEls, ...edgeEls];
  }, [boundedNodes, boundedEdges]);

  // (Re)build cytoscape whenever the data changes.
  useEffect(() => {
    let cancelled = false;
    if (!containerRef.current) return;
    loadCytoscape()
      .then((cytoscape) => {
        if (cancelled || !containerRef.current) return;
        cyRef.current?.destroy();
        const cy = cytoscape({
          container: containerRef.current,
          elements,
          style: stylesheet,
          layout: layoutOptions(layout),
          wheelSensitivity: 0.2,
          maxZoom: 4,
          minZoom: 0.2,
          autoungrabify: false,
        });
        cyRef.current = cy;
        // Click on a node selects it; click on empty canvas clears.
        cy.on('tap', 'node', (e: CyEvent) => {
          onSelect(e.target.id());
        });
        cy.on('tap', (e: any) => {
          if (e.target === cy) onSelect(null);
        });
        cy.fit(40);
      })
      .catch((e) => {
        if (!cancelled) setErr(String(e));
      });
    return () => {
      cancelled = true;
      cyRef.current?.destroy();
      cyRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [elements, layout]);

  // Apply selection styling without rebuilding the graph.
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    cy.batch(() => {
      cy.elements().removeClass('dimmed in out');
      if (!selectedId) return;
      const sel = cy.getElementById(selectedId);
      if (!sel.length) return;
      const outgoers = sel.outgoers() as CyEles;
      const incomers = sel.incomers() as CyEles;
      const related: Set<string> = new Set([selectedId]);
      outgoers.forEach((el) => related.add(el.data('id') as string));
      incomers.forEach((el) => related.add(el.data('id') as string));
      cy.elements().forEach((el) => {
        const id = el.data('id') as string;
        if (!related.has(id) && !isEdgeBetween(el, related)) {
          el.addClass('dimmed');
        }
      });
      // Color edges by direction relative to the selected node.
      const outEdges = (sel.outgoers() as any).edges?.() ?? sel.outgoers();
      const inEdges = (sel.incomers() as any).edges?.() ?? sel.incomers();
      (outEdges as CyEles).addClass('out');
      (inEdges as CyEles).addClass('in');
    });
  }, [selectedId]);

  // Resize Cytoscape after fullscreen toggles (container size changes).
  useEffect(() => {
    const cy = cyRef.current;
    if (!cy) return;
    queueMicrotask(() => {
      cy.resize();
      cy.fit(40);
    });
  }, [fullscreen]);

  if (err) return <p class="card text-ink-err">{err}</p>;

  return (
    <div>
      {omittedCount > 0 && !showAll && (
        <div class="text-xs text-ink-warning bg-ink-warningSubtle border border-ink-warning px-2 py-1 mb-2">
          Showing top {boundedNodes.length} of {nodes.length} nodes by degree.
          <button class="underline ml-1" onClick={() => setShowAll(true)}>Show all</button>
        </div>
      )}
      <div
        ref={containerRef}
        class="bg-white border border-ink-700"
        style={{
          width: '100%',
          height: fullscreen ? '100%' : 560,
        }}
      />
      <ul class="flex flex-wrap gap-2 text-xs text-ink-400 mt-2">
        {langStats.slice(0, 5).map(([lang, count]) => (
          <li key={lang} class="flex items-center gap-1">
            <span class="inline-block w-3 h-3" style={{ background: languageStroke(lang) }} />
            {lang} ({count})
          </li>
        ))}
      </ul>
    </div>
  );
}

function isEdgeBetween(el: CyEles, related: Set<string>): boolean {
  // Edges with `data.source` and `data.target` survive dimming when both ends
  // are in `related`. Nodes (no source/target) take the default branch.
  const source = el.data('source') as string | undefined;
  const target = el.data('target') as string | undefined;
  if (!source || !target) return false;
  return related.has(source) && related.has(target);
}

function layoutOptions(layout: 'dagre' | 'cose') {
  if (layout === 'dagre') {
    return {
      name: 'dagre',
      rankDir: 'TB',
      nodeSep: 28,
      rankSep: 48,
      edgeSep: 12,
      animate: false,
      fit: true,
      padding: 40,
    };
  }
  return {
    name: 'cose',
    animate: false,
    nodeRepulsion: 6000,
    idealEdgeLength: 80,
    edgeElasticity: 80,
    gravity: 0.25,
    fit: true,
    padding: 40,
  };
}
