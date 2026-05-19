import { Link } from 'wouter-preact';
import { useRecentPaths } from '../hooks';
import {
  useBrief,
  useQuality,
  useHotspots,
  useTestGaps,
  useFunctions,
  useScan,
} from '../data/queries';
import { LoadingState } from '../components/states/LoadingState';

function grade(score: number | null | undefined): string {
  if (score == null) return '—';
  // Quality score is 0–100. Anything ≥90 = A, ≥80 = B, ≥70 = C, ≥60 = D, else F.
  if (score >= 90) return 'A';
  if (score >= 80) return 'B';
  if (score >= 70) return 'C';
  if (score >= 60) return 'D';
  return 'F';
}

function formatTimestamp(ts: string | undefined | null): string {
  if (!ts) return '—';
  try {
    const d = new Date(ts);
    if (Number.isNaN(d.getTime())) return ts;
    return d.toLocaleString();
  } catch {
    return String(ts);
  }
}

type CardProps = {
  title: string;
  value: string;
  subtitle: string;
  loading: boolean;
  error: boolean;
};

function Card({ title, value, subtitle, loading, error }: CardProps) {
  return (
    <a
      href="/scans"
      class="block bg-ink-900 border border-ink-700 hover:border-ink-brand p-4 transition-colors"
    >
      <div class="text-xs uppercase tracking-carbon text-ink-500 mb-1">{title}</div>
      {loading ? (
        <LoadingState label="Loading…" />
      ) : (
        <>
          <div class="text-3xl font-mono text-ink-300 mb-1">{error ? '—' : value}</div>
          <div class="text-xs text-ink-400">{subtitle}</div>
        </>
      )}
    </a>
  );
}

export function Home() {
  const { paths } = useRecentPaths();
  const latest = paths[0] ?? '';

  const quality = useQuality(latest);
  const hotspots = useHotspots(latest);
  const testGaps = useTestGaps(latest);
  const fnsAll = useFunctions(latest);
  const fnsHot = useFunctions(latest, { minCc: 30 });
  const scan = useScan(latest);
  const brief = useBrief(latest);

  const qScore = quality.data?.quality?.score ?? null;
  const qGrade = grade(qScore);
  const topHotspot = hotspots.data?.hotspots?.[0];
  const testGapCount = testGaps.data?.summary?.gap_files;
  const fnTotal = fnsAll.data?.total;
  const fnHotCount = fnsHot.data?.total;
  const cycles = quality.data?.cycles_count;
  const lastScan = scan.data?.scanned_at;

  return (
    <div class="space-y-8">
      <section class="grid grid-cols-1 md:grid-cols-3 gap-3 mb-6">
        <Card
          title="Quality"
          value={qScore != null ? `${qScore.toFixed(0)} ${qGrade}` : '—'}
          subtitle="Composite score"
          loading={!latest ? false : quality.isLoading}
          error={!!quality.error}
        />
        <Card
          title="Top Hotspot"
          value={topHotspot ? topHotspot.score.toFixed(1) : '—'}
          subtitle={topHotspot ? topHotspot.path : 'No hotspots'}
          loading={!latest ? false : hotspots.isLoading}
          error={!!hotspots.error}
        />
        <Card
          title="Test Gaps"
          value={testGapCount != null ? String(testGapCount) : '—'}
          subtitle="Untested source files"
          loading={!latest ? false : testGaps.isLoading}
          error={!!testGaps.error}
        />
        <Card
          title="Functions"
          value={fnTotal != null ? String(fnTotal) : '—'}
          subtitle={fnHotCount != null ? `${fnHotCount} hot (cc > 30)` : 'Total functions'}
          loading={!latest ? false : fnsAll.isLoading || fnsHot.isLoading}
          error={!!fnsAll.error || !!fnsHot.error}
        />
        <Card
          title="Cycles"
          value={cycles != null ? String(cycles) : '—'}
          subtitle="Import cycles"
          loading={!latest ? false : quality.isLoading}
          error={!!quality.error}
        />
        <Card
          title="Last Scan"
          value={formatTimestamp(lastScan)}
          subtitle={latest || 'No scan yet'}
          loading={!latest ? false : scan.isLoading}
          error={!!scan.error}
        />
      </section>

      <section>
        <h1 class="text-2xl text-ink-300">A continuous read on your code.</h1>
        <p class="mt-2 max-w-2xl text-sm text-ink-400">
          Belisarius is an analysis engine that walks your codebase, builds an import graph,
          and surfaces structural metrics — cycles, complexity, hotspots, test gaps, public
          surface. Use the Scans tab to run an analysis or Fleet to compare across projects.
        </p>
      </section>

      {latest && (
        <section>
          <div class="mb-2 flex items-baseline justify-between">
            <h2 class="text-sm uppercase tracking-wider text-ink-500">
              Latest brief
              <span class="ml-2 text-ink-400 normal-case tracking-normal">— {latest}</span>
            </h2>
            <Link
              href={`/scans#path=${encodeURIComponent(latest)}`}
              class="text-xs text-accent-500 hover:underline"
            >
              Open scan →
            </Link>
          </div>
          <div class="card">
            {brief.isLoading && <div class="text-sm text-ink-400">Composing brief…</div>}
            {brief.error && (
              <div class="text-sm text-ink-err">Brief failed: {String(brief.error)}</div>
            )}
            {brief.data && (
              <>
                <pre class="whitespace-pre-wrap break-words text-sm leading-relaxed text-ink-200">
                  {brief.data.markdown}
                </pre>
                <div class="mt-3 text-xs text-ink-500">{String(brief.data.bytes)} bytes</div>
              </>
            )}
          </div>
        </section>
      )}

      <section class="grid gap-6 md:grid-cols-2">
        <Link href="/scans" class="card hover:border-accent-500 block">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">Scan a project</h2>
          <p class="mt-2 text-sm text-ink-300">
            Walk a project, compute structural quality, and explore by file, function, cycle,
            and symbol.
          </p>
        </Link>
        <Link href="/fleet" class="card hover:border-accent-500 block">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">Compare across the fleet</h2>
          <p class="mt-2 text-sm text-ink-300">
            Cross-project surface search, hotspots, and test-gap rollups across every project
            registered in <code>~/.belisarius/fleet.toml</code>.
          </p>
        </Link>
      </section>
    </div>
  );
}
