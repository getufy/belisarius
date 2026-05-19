import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import type { ComponentChildren, JSX } from 'preact';
import { EmptyState } from './states/EmptyState';

export type Column<T> = {
  key: keyof T & string;
  header: string;
  sortable?: boolean;
  className?: string;
  render?: (row: T) => ComponentChildren;
  numeric?: boolean;
};

type Props<T> = {
  rows: T[];
  columns: Column<T>[];
  filter?: (row: T, q: string) => boolean;
  initialSort?: { key: keyof T & string; dir: 'asc' | 'desc' };
  rowKey: (row: T) => string;
  onRowClick?: (row: T) => void;
  empty?: ComponentChildren;
  rowHeight?: number;
  virtualizeThreshold?: number;
  className?: string;
};

type SortState<T> = { key: keyof T & string; dir: 'asc' | 'desc' } | null;

const DEFAULT_ROW_HEIGHT = 32;
const DEFAULT_VIRTUALIZE_THRESHOLD = 200;
const OVERSCAN = 8;
const VIEWPORT_HEIGHT = 600;

export function MetricTable<T>({
  rows,
  columns,
  filter,
  initialSort,
  rowKey,
  onRowClick,
  empty,
  rowHeight = DEFAULT_ROW_HEIGHT,
  virtualizeThreshold = DEFAULT_VIRTUALIZE_THRESHOLD,
  className,
}: Props<T>): JSX.Element {
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<SortState<T>>(initialSort ?? null);
  const [range, setRange] = useState({ start: 0, end: 0 });
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const filtered = useMemo(() => {
    if (!filter || !query.trim()) return rows;
    return rows.filter((r) => filter(r, query));
  }, [rows, filter, query]);

  const sorted = useMemo(() => {
    if (!sort) return filtered;
    const { key, dir } = sort;
    const mult = dir === 'asc' ? 1 : -1;
    const copy = filtered.slice();
    copy.sort((a, b) => {
      const av = (a as Record<string, unknown>)[key];
      const bv = (b as Record<string, unknown>)[key];
      if (av == null && bv == null) return 0;
      if (av == null) return 1;
      if (bv == null) return -1;
      if (typeof av === 'number' && typeof bv === 'number') {
        return (av - bv) * mult;
      }
      const as = String(av).toLowerCase();
      const bs = String(bv).toLowerCase();
      if (as < bs) return -1 * mult;
      if (as > bs) return 1 * mult;
      return 0;
    });
    return copy;
  }, [filtered, sort]);

  const virtualize = sorted.length > virtualizeThreshold;

  // Compute initial range so the first paint isn't empty under virtualization.
  useEffect(() => {
    if (!virtualize) return;
    const visibleCount = Math.ceil(VIEWPORT_HEIGHT / rowHeight) + OVERSCAN;
    setRange({ start: 0, end: Math.min(sorted.length, visibleCount) });
  }, [virtualize, rowHeight, sorted.length]);

  const onScroll = () => {
    if (!virtualize) return;
    const el = scrollRef.current;
    if (!el) return;
    const scrollTop = el.scrollTop;
    const viewportHeight = el.clientHeight || VIEWPORT_HEIGHT;
    const start = Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN);
    const end = Math.min(
      sorted.length,
      start + Math.ceil(viewportHeight / rowHeight) + OVERSCAN * 2,
    );
    if (start !== range.start || end !== range.end) {
      setRange({ start, end });
    }
  };

  const toggleSort = (col: Column<T>) => {
    if (col.sortable === false) return;
    setSort((curr) => {
      if (!curr || curr.key !== col.key) return { key: col.key, dir: 'desc' };
      return { key: col.key, dir: curr.dir === 'asc' ? 'desc' : 'asc' };
    });
  };

  const visibleRows = virtualize ? sorted.slice(range.start, range.end) : sorted;
  const topSpacer = virtualize ? range.start * rowHeight : 0;
  const bottomSpacer = virtualize ? (sorted.length - range.end) * rowHeight : 0;

  const rootClass = className
    ? `space-y-2 ${className}`
    : 'space-y-2';

  if (rows.length === 0) {
    return (
      <div class={rootClass}>
        {empty ?? (
          <EmptyState title="No data" hint="Nothing to show here yet." />
        )}
      </div>
    );
  }

  return (
    <div class={rootClass}>
      {filter && (
        <input
          type="text"
          class="bg-ink-800 border border-ink-700 text-sm px-2 py-1 w-full mb-2"
          placeholder="Filter…"
          value={query}
          onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
          aria-label="Filter rows"
        />
      )}
      <div
        ref={scrollRef}
        onScroll={onScroll}
        class={virtualize ? 'h-[600px] overflow-auto' : 'overflow-auto'}
      >
        <table class="w-full text-xs border-collapse">
          <thead>
            <tr class="sticky top-0 bg-ink-900 z-10 border-b border-ink-700">
              {columns.map((col) => {
                const sortable = col.sortable !== false;
                const isSorted = sort?.key === col.key;
                const indicator = isSorted
                  ? sort?.dir === 'asc'
                    ? ' ▲'
                    : ' ▼'
                  : '';
                const align = col.numeric ? 'text-right' : 'text-left';
                const cursor = sortable ? 'cursor-pointer select-none' : '';
                const klass = `px-2 py-1 ${align} ${cursor} text-ink-500 uppercase tracking-wider`;
                return (
                  <th
                    key={col.key}
                    class={klass}
                    onClick={() => toggleSort(col)}
                    aria-sort={
                      isSorted
                        ? sort?.dir === 'asc'
                          ? 'ascending'
                          : 'descending'
                        : 'none'
                    }
                  >
                    {col.header}
                    {indicator}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {virtualize && topSpacer > 0 && (
              <tr aria-hidden="true" style={{ height: `${topSpacer}px` }}>
                <td colSpan={columns.length} />
              </tr>
            )}
            {visibleRows.map((row) => {
              const clickable = !!onRowClick;
              const trClass = `border-t border-ink-700 hover:bg-ink-800 ${
                clickable ? 'cursor-pointer' : ''
              }`;
              return (
                <tr
                  key={rowKey(row)}
                  class={trClass}
                  style={virtualize ? { height: `${rowHeight}px` } : undefined}
                  onClick={clickable ? () => onRowClick!(row) : undefined}
                >
                  {columns.map((col) => {
                    const align = col.numeric ? 'text-right font-mono' : 'text-left';
                    const cellKlass = `px-2 py-1 ${align} ${col.className ?? ''}`.trim();
                    const content = col.render
                      ? col.render(row)
                      : (row as Record<string, unknown>)[col.key] as ComponentChildren;
                    return (
                      <td key={col.key} class={cellKlass}>
                        {content}
                      </td>
                    );
                  })}
                </tr>
              );
            })}
            {virtualize && bottomSpacer > 0 && (
              <tr aria-hidden="true" style={{ height: `${bottomSpacer}px` }}>
                <td colSpan={columns.length} />
              </tr>
            )}
          </tbody>
        </table>
        {sorted.length === 0 && (
          <div class="py-4">
            {empty ?? (
              <EmptyState title="No matches" hint="Try a different filter." />
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default MetricTable;
