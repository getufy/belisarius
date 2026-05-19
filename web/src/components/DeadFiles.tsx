import { useMemo, useState } from 'preact/hooks';
import { Graph, GraphNode } from '../api';

type Sort = 'path' | 'loc' | 'language';

// Languages the Rust resolver actually understands. Others have no resolved
// in-edges and would always look "dead" — so we exclude them by default.
const RESOLVED_LANGS = new Set(['rust', 'typescript', 'javascript', 'python']);

export function DeadFiles({ graph }: { graph: Graph }) {
  const [includeUnresolved, setIncludeUnresolved] = useState(false);
  const [sort, setSort] = useState<Sort>('loc');

  const dead = useMemo(() => {
    let list = graph.nodes.filter(isDead);
    if (!includeUnresolved) list = list.filter((n) => RESOLVED_LANGS.has(n.language));
    list.sort((a, b) => compare(a, b, sort));
    return list;
  }, [graph, includeUnresolved, sort]);

  const total = graph.nodes.filter(
    (n) => includeUnresolved || RESOLVED_LANGS.has(n.language)
  ).length;

  return (
    <div class="card">
      <header class="mb-3 flex items-baseline justify-between">
        <div>
          <h2 class="text-sm uppercase tracking-wider text-ink-500">Dead-file candidates</h2>
          <p class="text-xs text-ink-500">
            Files with zero incoming imports that aren't recognized entry points.
            Heuristic only — verify before deleting.
          </p>
        </div>
        <div class="flex items-center gap-3">
          <label class="flex items-center gap-1.5 text-xs text-ink-400" title="Include languages the resolver doesn't understand (Go, Java, etc.) — they'll always look dead.">
            <input
              type="checkbox"
              checked={includeUnresolved}
              onChange={(e) => setIncludeUnresolved((e.target as HTMLInputElement).checked)}
            />
            include unresolved langs
          </label>
          <select
            class="field py-1 text-xs"
            value={sort}
            onChange={(e) => setSort((e.target as HTMLSelectElement).value as Sort)}
          >
            <option value="loc">sort: loc</option>
            <option value="path">sort: path</option>
            <option value="language">sort: language</option>
          </select>
        </div>
      </header>
      <p class="mb-3 text-xs text-ink-500">
        {dead.length} of {total} considered ({Math.round((dead.length / Math.max(1, total)) * 100)}%)
      </p>
      {dead.length === 0 ? (
        <p class="text-sm text-ink-500">Nothing flagged — every file is referenced.</p>
      ) : (
        <ul class="space-y-0.5 max-h-[520px] overflow-auto text-xs">
          {dead.map((n) => (
            <li
              key={n.id}
              class="flex items-baseline justify-between border-b border-ink-700 py-1"
            >
              <span class="truncate text-ink-300">{n.id}</span>
              <span class="ml-3 shrink-0 text-[10px] text-ink-500">
                {n.language} · {n.loc} loc · out {n.out_degree}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function isDead(n: GraphNode): boolean {
  return n.in_degree === 0 && !n.is_entry_point;
}
function compare(a: GraphNode, b: GraphNode, by: Sort): number {
  if (by === 'loc') return b.loc - a.loc;
  if (by === 'path') return a.id.localeCompare(b.id);
  return a.language.localeCompare(b.language) || a.id.localeCompare(b.id);
}
