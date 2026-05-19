import { useEffect, useState } from 'preact/hooks';
import { lazy, Suspense } from 'preact/compat';

const HotspotsView = lazy(() =>
  import('./HotspotsView').then((m) => ({ default: m.HotspotsView })),
);
const TestGapsView = lazy(() =>
  import('./TestGapsView').then((m) => ({ default: m.TestGapsView })),
);
const MarkersView = lazy(() =>
  import('./MarkersView').then((m) => ({ default: m.MarkersView })),
);
const DiagnosticsView = lazy(() =>
  import('./DiagnosticsView').then((m) => ({ default: m.DiagnosticsView })),
);

type Facet = 'hotspots' | 'test-gaps' | 'markers' | 'diagnostics';

type FacetDef = {
  id: Facet;
  label: string;
  hint: string;
  badge?: number | null;
};

const HASH_KEY = 'findings';

function readFacet(): Facet {
  if (typeof window === 'undefined') return 'hotspots';
  const m = window.location.hash.match(new RegExp(`${HASH_KEY}=([a-z-]+)`));
  const v = (m?.[1] ?? 'hotspots') as Facet;
  return ['hotspots', 'test-gaps', 'markers', 'diagnostics'].includes(v)
    ? v
    : 'hotspots';
}

function writeFacet(v: Facet) {
  if (typeof window === 'undefined') return;
  const hash = window.location.hash.replace(/^#/, '');
  const parts = hash.split('&').filter((p) => p && !p.startsWith(`${HASH_KEY}=`));
  parts.push(`${HASH_KEY}=${v}`);
  window.location.hash = parts.join('&');
}

export function FindingsView({
  path,
  markersCount,
  diagCount,
}: {
  path: string;
  markersCount?: number | null;
  diagCount?: number | null;
}) {
  const [facet, setFacet] = useState<Facet>(readFacet);

  useEffect(() => {
    const onHash = () => setFacet(readFacet());
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);

  const select = (v: Facet) => {
    setFacet(v);
    writeFacet(v);
  };

  const facets: FacetDef[] = [
    {
      id: 'hotspots',
      label: 'Hotspots',
      hint: 'Files ranked by churn × complexity (git history window)',
    },
    {
      id: 'test-gaps',
      label: 'Test gaps',
      hint: 'Source files with no covering test, ranked by complexity',
    },
    {
      id: 'markers',
      label: 'Markers',
      hint: 'TODO / FIXME / HACK / XXX across the project',
      badge: markersCount ?? null,
    },
    {
      id: 'diagnostics',
      label: 'Diagnostics',
      hint: 'External lint / security tool runs (clippy, semgrep, eslint, …)',
      badge: diagCount ?? null,
    },
  ];

  return (
    <div class="space-y-4">
      <nav
        class="flex flex-wrap gap-1 border-b border-ink-700"
        role="tablist"
        aria-label="Findings categories"
      >
        {facets.map((f) => {
          const active = facet === f.id;
          return (
            <button
              key={f.id}
              type="button"
              role="tab"
              aria-selected={active}
              title={f.hint}
              onClick={() => select(f.id)}
              class={`group px-3 py-1.5 -mb-px border-b-2 text-sm transition-colors ${
                active
                  ? 'border-accent-500 text-accent-500'
                  : 'border-transparent text-ink-400 hover:text-ink-200'
              }`}
            >
              {f.label}
              {f.badge != null && (
                <span
                  class={`ml-1.5 inline-flex items-center justify-center min-w-[1.25rem] px-1 text-[10px] leading-4 rounded ${
                    active
                      ? 'bg-accent-500/10 text-accent-500'
                      : 'bg-ink-800 text-ink-500 group-hover:text-ink-400'
                  }`}
                >
                  {f.badge}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <Suspense fallback={<p class="card text-ink-500">Loading…</p>}>
        {facet === 'hotspots' && <HotspotsView path={path} />}
        {facet === 'test-gaps' && <TestGapsView path={path} />}
        {facet === 'markers' && <MarkersView path={path} />}
        {facet === 'diagnostics' && <DiagnosticsView path={path} />}
      </Suspense>
    </div>
  );
}
