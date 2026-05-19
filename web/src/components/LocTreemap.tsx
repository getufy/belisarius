import { useEffect, useMemo, useState } from 'preact/hooks';
import { api, GraphNode } from '../api';
import { COLORS } from '../lib/colors';

type TreeNode = {
  name: string;
  path: string;
  value: number;
  language?: string;
  loc?: number;
  children: TreeNode[];
};

type Rect = { x: number; y: number; w: number; h: number; node: TreeNode };

const W = 720;
const H = 480;

// Carbon-aligned data-viz palette. Strong strokes for distinction.
const LANG_COLOR: Record<string, string> = {
  rust: '#d97a5d',
  typescript: '#0f62fe',
  javascript: '#bca84a',
  python: '#24a148',
  go: '#0098a6',
  default: '#525252',
};

type ColorMode = 'language' | 'complexity';

export function LocTreemap({
  nodes,
  path,
  onFileOpen,
}: {
  nodes: GraphNode[];
  path?: string;
  onFileOpen?: (file: string) => void;
}) {
  const [hovered, setHovered] = useState<TreeNode | null>(null);
  const [colorMode, setColorMode] = useState<ColorMode>('language');
  const [maxCcByFile, setMaxCcByFile] = useState<Map<string, number> | null>(null);

  const codeOnly = useMemo(
    () => nodes.filter((n) => isCode(n.language) && n.loc > 0),
    [nodes]
  );
  const tree = useMemo(() => buildTree(codeOnly), [codeOnly]);
  const rects = useMemo(() => layout(tree, 0, 0, W, H), [tree]);
  const totalLoc = codeOnly.reduce((s, n) => s + n.loc, 0);

  useEffect(() => {
    if (colorMode !== 'complexity' || maxCcByFile || !path) return;
    api
      .functions(path, { limit: 5000 })
      .then((r) => {
        const m = new Map<string, number>();
        for (const f of r.functions) {
          const prev = m.get(f.file) ?? 0;
          if (f.cyclomatic > prev) m.set(f.file, f.cyclomatic);
        }
        setMaxCcByFile(m);
      })
      .catch(() => setMaxCcByFile(new Map()));
  }, [colorMode, path, maxCcByFile]);

  const hoveredCc = hovered && maxCcByFile ? maxCcByFile.get(hovered.path) : undefined;

  return (
    <div class="card">
      <header class="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <div>
          <h2 class="text-sm uppercase tracking-wider text-ink-500">LOC by directory</h2>
          <p class="text-xs text-ink-500">
            {codeOnly.length} files · {totalLoc.toLocaleString()} loc · sized by lines of code
          </p>
        </div>
        {path && (
          <div class="flex gap-1 text-[10px]">
            <button
              class={`pill ${colorMode === 'language' ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
              onClick={() => setColorMode('language')}
            >
              by language
            </button>
            <button
              class={`pill ${colorMode === 'complexity' ? 'border-accent-500 text-accent-500' : 'text-ink-500 hover:text-ink-300'}`}
              onClick={() => setColorMode('complexity')}
            >
              by complexity
            </button>
          </div>
        )}
      </header>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="xMidYMid meet"
        class="block w-full bg-white border border-ink-700"
        style={{ aspectRatio: `${W} / ${H}` }}
      >
        {rects.map((r) => {
          const isFile = r.node.children.length === 0;
          const lang = r.node.language ?? majorityLanguage(r.node);
          let color: string;
          let fillOpacity: number;
          if (colorMode === 'complexity' && maxCcByFile) {
            const cc = isFile
              ? maxCcByFile.get(r.node.path) ?? 0
              : maxCcInSubtree(r.node, maxCcByFile);
            color = ccColor(cc);
            fillOpacity = isFile ? 0.85 : 0.2;
          } else {
            color = LANG_COLOR[lang] ?? LANG_COLOR.default;
            fillOpacity = isFile ? 0.7 : 0.15;
          }
          const showLabel = r.w > 60 && r.h > 18;
          return (
            <g
              key={r.node.path}
              onMouseEnter={() => setHovered(r.node)}
              onMouseLeave={() => setHovered(null)}
              onClick={() => {
                if (isFile && onFileOpen) onFileOpen(r.node.path);
              }}
              style={{ cursor: isFile && onFileOpen ? 'pointer' : 'default' }}
            >
              <rect
                x={r.x}
                y={r.y}
                width={Math.max(0, r.w - 1)}
                height={Math.max(0, r.h - 1)}
                fill={color}
                fill-opacity={fillOpacity}
                stroke="#e0e0e0"
                stroke-width={0.75}
              />
              {showLabel && (
                <text
                  x={r.x + 4}
                  y={r.y + 14}
                  fill={isFile ? '#161616' : '#525252'}
                  font-size="10"
                  font-family='"IBM Plex Mono", ui-monospace, Menlo, monospace'
                  style={{ pointerEvents: 'none' }}
                >
                  {ellipsize(r.node.name, Math.floor(r.w / 6))}
                </text>
              )}
            </g>
          );
        })}
      </svg>
      <p class="mt-2 text-xs text-ink-500">
        {hovered ? (
          <>
            <code class="text-accent-500">{hovered.path}</code> ·{' '}
            {hovered.value.toLocaleString()} loc
            {hoveredCc !== undefined && hoveredCc > 0 && (
              <>
                {' '}· max cc <span class={ccTextColor(hoveredCc)}>{hoveredCc}</span>
              </>
            )}
          </>
        ) : (
          colorMode === 'complexity' && !maxCcByFile
            ? 'Loading per-file complexity…'
            : 'Hover for details.'
        )}
      </p>
    </div>
  );
}

function isCode(lang: string): boolean {
  return ['rust', 'typescript', 'javascript', 'python', 'go', 'java', 'ruby'].includes(lang);
}

function buildTree(files: GraphNode[]): TreeNode {
  const root: TreeNode = { name: '/', path: '', value: 0, children: [] };
  for (const f of files) {
    const parts = f.id.split('/');
    let cur = root;
    let accum = '';
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      accum = accum ? `${accum}/${part}` : part;
      const isLeaf = i === parts.length - 1;
      let child = cur.children.find((c) => c.name === part);
      if (!child) {
        child = {
          name: part,
          path: accum,
          value: 0,
          children: [],
          ...(isLeaf ? { language: f.language, loc: f.loc } : {}),
        };
        cur.children.push(child);
      }
      child.value += f.loc;
      cur = child;
    }
    root.value += f.loc;
  }
  return root;
}

function layout(root: TreeNode, x: number, y: number, w: number, h: number): Rect[] {
  const out: Rect[] = [];
  squarify(root, x, y, w, h, out);
  return out;
}

function squarify(node: TreeNode, x: number, y: number, w: number, h: number, out: Rect[]) {
  if (w <= 0 || h <= 0) return;
  out.push({ x, y, w, h, node });
  if (node.children.length === 0) return;
  // Reserve a header strip for non-leaf nodes
  const headerH = node.path === '' ? 0 : Math.min(16, h * 0.06);
  const cy = y + headerH;
  const ch = h - headerH;
  if (ch <= 0) return;

  const sorted = [...node.children].sort((a, b) => b.value - a.value);
  squarifyRow(sorted, x, cy, w, ch, out);
}

function squarifyRow(items: TreeNode[], x: number, y: number, w: number, h: number, out: Rect[]) {
  let cx = x;
  let cy = y;
  let cw = w;
  let ch = h;
  let remaining = items.slice();
  const totalArea = remaining.reduce((s, n) => s + n.value, 0);
  if (totalArea === 0) return;
  const totalPx = cw * ch;
  const scale = totalPx / totalArea;

  while (remaining.length > 0) {
    const horiz = cw < ch;
    let row: TreeNode[] = [];
    let bestRatio = Infinity;
    let rowArea = 0;
    for (const item of remaining) {
      const trial = [...row, item];
      const area = rowArea + item.value * scale;
      const side = horiz ? cw : ch;
      const otherSide = area / side;
      const trialRatio = trial.reduce((m, n) => {
        const npx = n.value * scale;
        const long = Math.max(otherSide, npx / otherSide);
        const short = Math.min(otherSide, npx / otherSide);
        return Math.max(m, long / Math.max(1e-6, short));
      }, 0);
      if (trialRatio > bestRatio && row.length > 0) break;
      row = trial;
      rowArea = area;
      bestRatio = trialRatio;
    }
    if (row.length === 0) row = [remaining[0]];
    const side = horiz ? cw : ch;
    const otherSide = rowArea / side;
    let off = 0;
    for (const item of row) {
      const px = item.value * scale;
      const seg = px / otherSide;
      if (horiz) {
        squarify(item, cx + off, cy, seg, otherSide, out);
      } else {
        squarify(item, cx, cy + off, otherSide, seg, out);
      }
      off += seg;
    }
    if (horiz) {
      cy += otherSide;
      ch -= otherSide;
    } else {
      cx += otherSide;
      cw -= otherSide;
    }
    remaining = remaining.slice(row.length);
  }
}

function majorityLanguage(n: TreeNode): string {
  const counts: Record<string, number> = {};
  const walk = (t: TreeNode) => {
    if (t.language) counts[t.language] = (counts[t.language] ?? 0) + (t.loc ?? 0);
    for (const c of t.children) walk(c);
  };
  walk(n);
  let best = 'default';
  let bestN = -1;
  for (const [k, v] of Object.entries(counts)) {
    if (v > bestN) {
      bestN = v;
      best = k;
    }
  }
  return best;
}

function ellipsize(s: string, n: number): string {
  if (n <= 1) return '';
  if (s.length <= n) return s;
  return s.slice(0, Math.max(1, n - 1)) + '…';
}

function maxCcInSubtree(node: TreeNode, m: Map<string, number>): number {
  let best = m.get(node.path) ?? 0;
  for (const c of node.children) {
    const sub = maxCcInSubtree(c, m);
    if (sub > best) best = sub;
  }
  return best;
}

// Heatmap fills for the complexity overlay, tuned for a white canvas.
function ccColor(cc: number): string {
  if (cc === 0) return '#f4f4f4';
  if (cc <= 4) return COLORS.success; // Carbon success
  if (cc <= 9) return '#9caf3a';
  if (cc <= 14) return COLORS.warning; // Carbon warning
  if (cc <= 19) return '#ff832b';
  return COLORS.err; // Carbon error
}

function ccTextColor(cc: number): string {
  if (cc >= 20) return 'text-ink-err';
  if (cc >= 10) return 'text-orange-400';
  if (cc >= 5) return 'text-ink-warning';
  return 'text-ink-300';
}
