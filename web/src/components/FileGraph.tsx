import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { Graph, GraphEdge, GraphNode } from '../api';

type Pos = { x: number; y: number };
type Force = { fx: number; fy: number };

// Carbon-aligned palette — same hues as the Treemap + Architecture views.
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

const W = 720;
const H = 520;
const MAX_NODES = 200;
const ITERATIONS = 260;

export function FileGraph({
  graph,
  cycleNodes,
  onNodeOpen,
}: {
  graph: Graph;
  cycleNodes?: Set<string>;
  onNodeOpen?: (id: string) => void;
}) {
  const [hiddenLangs, setHiddenLangs] = useState<Set<string>>(new Set());
  const [hovered, setHovered] = useState<GraphNode | null>(null);
  const [showAll, setShowAll] = useState(false);
  const effectiveMax = showAll ? Infinity : MAX_NODES;

  // All languages present in the (unfiltered) graph, sorted by file count.
  const langStats = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const n of graph.nodes) counts[n.language] = (counts[n.language] ?? 0) + 1;
    return Object.entries(counts).sort((a, b) => b[1] - a[1]);
  }, [graph]);

  // Visible subset: filter by language, cap at effectiveMax by total degree.
  const { shownNodes, shownEdges, omittedCount } = useMemo(() => {
    const filtered = graph.nodes.filter((n) => !hiddenLangs.has(n.language));
    const ranked = [...filtered].sort(
      (a, b) => b.in_degree + b.out_degree - (a.in_degree + a.out_degree)
    );
    const kept = ranked.slice(0, effectiveMax);
    const keptSet = new Set(kept.map((n) => n.id));
    const edges = graph.edges.filter((e) => keptSet.has(e.from) && keptSet.has(e.to));
    return {
      shownNodes: kept,
      shownEdges: edges,
      omittedCount: filtered.length - kept.length,
    };
  }, [graph, hiddenLangs, effectiveMax]);

  // Precompute the layout synchronously. No animation loop, no per-frame re-renders.
  const positions = useMemo(
    () => runForceLayout(shownNodes, shownEdges),
    [shownNodes, shownEdges]
  );

  // DOM refs for direct mutation during hover/drag — avoids re-rendering 250 vnodes per event.
  const nodeRefs = useRef(new Map<string, SVGCircleElement>());
  const edgeRefs = useRef(new Map<string, SVGLineElement>());
  const positionsRef = useRef(positions);
  positionsRef.current = positions;
  const dragRef = useRef<{ id: string; offX: number; offY: number } | null>(null);

  // Imperatively restyle nodes + edges when hover changes.
  useEffect(() => {
    if (!hovered) {
      for (const [, el] of edgeRefs.current) {
        el.setAttribute('stroke', '#525252');
        el.setAttribute('stroke-width', '0.7');
        el.setAttribute('stroke-opacity', '0.5');
      }
      for (const n of shownNodes) {
        const el = nodeRefs.current.get(n.id);
        if (el) el.setAttribute('fill-opacity', n.is_entry_point ? '0.95' : '0.75');
      }
      return;
    }
    const related = new Set<string>([hovered.id]);
    for (const e of shownEdges) {
      if (e.from === hovered.id || e.to === hovered.id) {
        related.add(e.from);
        related.add(e.to);
      }
    }
    for (const e of shownEdges) {
      const el = edgeRefs.current.get(edgeKey(e));
      if (!el) continue;
      const on = e.from === hovered.id || e.to === hovered.id;
      el.setAttribute('stroke', on ? '#0f62fe' : '#c6c6c6');
      el.setAttribute('stroke-width', on ? '1.6' : '0.5');
      el.setAttribute('stroke-opacity', on ? '0.95' : '0.4');
    }
    for (const n of shownNodes) {
      const el = nodeRefs.current.get(n.id);
      if (!el) continue;
      el.setAttribute(
        'fill-opacity',
        related.has(n.id) ? (n.is_entry_point ? '0.95' : '0.9') : '0.15'
      );
    }
  }, [hovered, shownEdges, shownNodes]);

  const onMouseDown = (e: MouseEvent, id: string) => {
    const pos = positionsRef.current.get(id);
    if (!pos) return;
    const pt = svgPoint(e);
    dragRef.current = { id, offX: pt.x - pos.x, offY: pt.y - pos.y };
  };
  const onMouseMove = (e: MouseEvent) => {
    const drag = dragRef.current;
    if (!drag) return;
    const pt = svgPoint(e);
    const nx = clamp(pt.x - drag.offX, 16, W - 16);
    const ny = clamp(pt.y - drag.offY, 16, H - 16);
    const pos = positionsRef.current.get(drag.id);
    if (!pos) return;
    pos.x = nx;
    pos.y = ny;
    const node = nodeRefs.current.get(drag.id);
    if (node) {
      node.setAttribute('cx', String(nx));
      node.setAttribute('cy', String(ny));
    }
    for (const edge of shownEdges) {
      if (edge.from !== drag.id && edge.to !== drag.id) continue;
      const el = edgeRefs.current.get(edgeKey(edge));
      if (!el) continue;
      if (edge.from === drag.id) {
        el.setAttribute('x1', String(nx));
        el.setAttribute('y1', String(ny));
      }
      if (edge.to === drag.id) {
        el.setAttribute('x2', String(nx));
        el.setAttribute('y2', String(ny));
      }
    }
  };
  const onMouseUp = () => {
    dragRef.current = null;
  };

  const toggleLang = (lang: string) => {
    setHiddenLangs((prev) => {
      const next = new Set(prev);
      if (next.has(lang)) next.delete(lang);
      else next.add(lang);
      return next;
    });
  };

  return (
    <div class="card">
      {omittedCount > 0 && !showAll && (
        <div class="text-xs text-ink-warning bg-ink-warningSubtle border border-ink-warning px-2 py-1 mb-2">
          Showing top {shownNodes.length} of {graph.nodes.length} nodes by degree.
          <button class="underline ml-1" onClick={() => setShowAll(true)}>Show all</button>
        </div>
      )}
      <header class="mb-3 flex flex-wrap items-baseline justify-between gap-3">
        <div>
          <h2 class="text-sm uppercase tracking-wider text-ink-500">File dependency graph</h2>
          <p class="text-xs text-ink-500">
            {shownNodes.length} of {graph.nodes.length} nodes · {shownEdges.length} edges
            {omittedCount > 0 && ` · ${omittedCount} omitted (lowest connectivity)`}
            {graph.unresolved > 0 && ` · ${graph.unresolved} external/unresolved imports`}
          </p>
        </div>
        <div class="flex flex-wrap gap-1.5">
          {langStats.map(([lang, count]) => {
            const hidden = hiddenLangs.has(lang);
            const color = LANG_COLOR[lang] ?? LANG_COLOR.default;
            return (
              <button
                key={lang}
                type="button"
                onClick={() => toggleLang(lang)}
                class="pill cursor-pointer select-none transition hover:opacity-100"
                style={{
                  borderColor: hidden ? '#e0e0e0' : color,
                  color: hidden ? '#8c8c8c' : color,
                  opacity: hidden ? 0.55 : 1,
                  textDecoration: hidden ? 'line-through' : 'none',
                }}
                title={hidden ? `Show ${lang}` : `Hide ${lang}`}
              >
                {lang} · {count}
              </button>
            );
          })}
        </div>
      </header>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="xMidYMid meet"
        class="block w-full select-none bg-white border border-ink-700"
        style={{ aspectRatio: `${W} / ${H}` }}
        onMouseMove={onMouseMove}
        onMouseUp={onMouseUp}
        onMouseLeave={onMouseUp}
      >
        <g stroke-opacity="0.6">
          {shownEdges.map((e) => {
            const a = positions.get(e.from);
            const b = positions.get(e.to);
            if (!a || !b) return null;
            const inCycle =
              cycleNodes && cycleNodes.has(e.from) && cycleNodes.has(e.to);
            return (
              <line
                key={edgeKey(e)}
                ref={(el) => {
                  if (el) edgeRefs.current.set(edgeKey(e), el);
                  else edgeRefs.current.delete(edgeKey(e));
                }}
                x1={a.x}
                y1={a.y}
                x2={b.x}
                y2={b.y}
                stroke={inCycle ? '#da1e28' : '#525252'}
                stroke-width={inCycle ? 1.6 : 0.6}
                stroke-opacity={inCycle ? 0.95 : 0.6}
              />
            );
          })}
        </g>
        <g>
          {shownNodes.map((n) => {
            const pos = positions.get(n.id)!;
            const color = LANG_COLOR[n.language] ?? LANG_COLOR.default;
            const inCycle = cycleNodes?.has(n.id) ?? false;
            const ringColor = inCycle ? '#da1e28' : n.is_entry_point ? '#0f62fe' : 'none';
            const ringWidth = inCycle ? 2 : n.is_entry_point ? 1.5 : 0;
            return (
              <circle
                key={n.id}
                ref={(el) => {
                  if (el) nodeRefs.current.set(n.id, el);
                  else nodeRefs.current.delete(n.id);
                }}
                cx={pos.x}
                cy={pos.y}
                r={nodeRadius(n)}
                fill={color}
                fill-opacity={n.is_entry_point ? 0.95 : 0.7}
                stroke={ringColor}
                stroke-width={ringWidth}
                style={{ cursor: 'grab' }}
                onMouseEnter={() => setHovered(n)}
                onMouseLeave={() => setHovered((cur) => (cur?.id === n.id ? null : cur))}
                onMouseDown={(e) => onMouseDown(e, n.id)}
                onDblClick={() => onNodeOpen?.(n.id)}
              />
            );
          })}
        </g>
        <Tooltip node={hovered} positions={positions} />
      </svg>
      <p class="mt-2 text-[10px] text-ink-500">
        Click a language pill to hide it · hover for details · drag to reposition
        {onNodeOpen && <> · double-click a node to open its DSM</>} · golden ring = entry point
        {cycleNodes && cycleNodes.size > 0 && (
          <> · <span class="text-orange-400">orange ring = cycle member</span></>
        )}.
      </p>
    </div>
  );
}

function Tooltip({
  node,
  positions,
}: {
  node: GraphNode | null;
  positions: Map<string, Pos>;
}) {
  if (!node) return null;
  const pos = positions.get(node.id);
  if (!pos) return null;
  const x = Math.min(pos.x + 10, W - 280);
  const y = Math.min(pos.y + 10, H - 70);
  return (
    <g style={{ pointerEvents: 'none' }}>
      <rect
        x={x}
        y={y}
        width="280"
        height={node.is_entry_point ? 62 : 50}
        fill="#ffffff"
        stroke="#e0e0e0"
      />
      <text
        x={x + 8}
        y={y + 16}
        fill="#161616"
        font-size="11"
        font-family='"IBM Plex Mono", ui-monospace, Menlo, monospace'
      >
        {ellipsize(node.id, 38)}
      </text>
      <text
        x={x + 8}
        y={y + 34}
        fill="#525252"
        font-size="10"
        font-family='"IBM Plex Mono", ui-monospace, Menlo, monospace'
      >
        {node.language} · {node.loc} loc · in {node.in_degree} · out {node.out_degree}
      </text>
      {node.is_entry_point && (
        <text
          x={x + 8}
          y={y + 50}
          fill="#0f62fe"
          font-size="9"
          font-family='"IBM Plex Mono", ui-monospace, Menlo, monospace'
        >
          entry point
        </text>
      )}
    </g>
  );
}

function runForceLayout(nodes: GraphNode[], edges: GraphEdge[]): Map<string, Pos> {
  const pos: Map<string, Pos & Force> = new Map();
  const cx = W / 2;
  const cy = H / 2;
  nodes.forEach((n, i) => {
    const t = (i / Math.max(1, nodes.length)) * Math.PI * 2;
    pos.set(n.id, {
      x: cx + Math.cos(t) * 160 + (rand() - 0.5) * 40,
      y: cy + Math.sin(t) * 160 + (rand() - 0.5) * 40,
      fx: 0,
      fy: 0,
    });
  });

  const repulsion = 420;
  const springK = 0.06;
  const targetLen = nodes.length > 80 ? 60 : 80;
  const center = 0.014;
  const damping = 0.82;
  const arr = nodes.map((n) => pos.get(n.id)!) as Array<Pos & Force & { vx?: number; vy?: number }>;

  for (let iter = 0; iter < ITERATIONS; iter++) {
    for (const p of arr) {
      p.fx = (cx - p.x) * center;
      p.fy = (cy - p.y) * center;
    }
    for (let i = 0; i < arr.length; i++) {
      const a = arr[i];
      for (let j = i + 1; j < arr.length; j++) {
        const b = arr[j];
        const dx = a.x - b.x;
        const dy = a.y - b.y;
        const d2 = dx * dx + dy * dy + 0.01;
        const d = Math.sqrt(d2);
        const f = repulsion / d2;
        const fx = (dx / d) * f;
        const fy = (dy / d) * f;
        a.fx += fx;
        a.fy += fy;
        b.fx -= fx;
        b.fy -= fy;
      }
    }
    for (const e of edges) {
      const a = pos.get(e.from);
      const b = pos.get(e.to);
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const d = Math.sqrt(dx * dx + dy * dy) + 0.01;
      const force = springK * (d - targetLen);
      const fx = (dx / d) * force;
      const fy = (dy / d) * force;
      (a as Pos & Force).fx += fx;
      (a as Pos & Force).fy += fy;
      (b as Pos & Force).fx -= fx;
      (b as Pos & Force).fy -= fy;
    }
    for (const p of arr) {
      p.vx = ((p.vx ?? 0) + p.fx) * damping;
      p.vy = ((p.vy ?? 0) + p.fy) * damping;
      p.x += p.vx;
      p.y += p.vy;
      p.x = clamp(p.x, 16, W - 16);
      p.y = clamp(p.y, 16, H - 16);
    }
  }

  const out = new Map<string, Pos>();
  for (const [k, v] of pos) out.set(k, { x: v.x, y: v.y });
  return out;
}

function nodeRadius(n: GraphNode): number {
  const total = n.in_degree + n.out_degree;
  return 4 + Math.min(10, Math.sqrt(total) * 1.6) + Math.min(3, Math.log2(1 + n.loc) * 0.3);
}

function edgeKey(e: GraphEdge): string {
  return `${e.from}\u0001${e.to}`;
}

function ellipsize(s: string, n: number): string {
  if (s.length <= n) return s;
  return '…' + s.slice(s.length - n + 1);
}

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

let seed = 1;
function rand(): number {
  seed = (seed * 9301 + 49297) % 233280;
  return seed / 233280;
}

function svgPoint(e: MouseEvent): Pos {
  const t = e.currentTarget as Element | null;
  const svg = t && (t.closest('svg') as SVGSVGElement | null);
  if (!svg) return { x: e.offsetX, y: e.offsetY };
  const rect = svg.getBoundingClientRect();
  return {
    x: ((e.clientX - rect.left) / rect.width) * W,
    y: ((e.clientY - rect.top) / rect.height) * H,
  };
}
