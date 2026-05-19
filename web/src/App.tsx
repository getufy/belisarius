import { Route, Switch, Link, useLocation } from 'wouter-preact';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Home } from './routes/Home';
import { ScanView } from './routes/ScanView';
import { FleetView } from './routes/FleetView';

// One QueryClient for the whole app. Default `staleTime` is 30s — long enough
// that flipping between tabs doesn't trigger an immediate refetch, short
// enough that returning to a tab after a real change re-fetches. Retries off
// by default (Belisarius endpoints either return data or hard-fail; a retry
// just delays the error message).
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: false,
      refetchOnWindowFocus: false,
    },
  },
});

const NAV = [
  { href: '/', label: 'Overview' },
  { href: '/fleet', label: 'Fleet' },
  { href: '/scans', label: 'Scans' },
];

const VERSION = '0.1.0';

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AppInner />
    </QueryClientProvider>
  );
}

function AppInner() {
  const [path] = useLocation();
  return (
    <div class="min-h-screen">
      {/* Utility bar — Carbon slim gray ribbon (32px, surface-1, caption type). */}
      <div class="bg-ink-800 border-b border-ink-700">
        <div class="flex items-center justify-between px-6 h-8 text-[11px] text-ink-500 tracking-[0.32px]">
          <div class="flex items-center gap-4">
            <span>analysis engine</span>
            <span class="hidden md:inline text-ink-600">v{VERSION}</span>
          </div>
          <div class="flex items-center gap-4">
            <a
              href="https://github.com/getufy/belisarius"
              target="_blank"
              rel="noreferrer"
              class="hover:text-accent-500 transition"
            >
              github
            </a>
            <span class="hidden md:inline text-ink-600">MCP · HTTP · CLI</span>
          </div>
        </div>
      </div>

      {/* Top nav — 48px tall, canvas, brand mark left, route links right. */}
      <header class="bg-white border-b border-ink-700">
        <div class="flex items-stretch px-6 h-12">
          <Link
            href="/"
            class="flex items-center gap-2.5 mr-10 group"
            aria-label="Belisarius home"
          >
            <span
              class="block w-3 h-3 bg-accent-500 transition-transform group-hover:rotate-45"
              aria-hidden="true"
            />
            <span class="text-[15px] font-medium tracking-tight text-ink-300">
              Belisarius
            </span>
          </Link>

          <nav class="flex items-stretch" aria-label="Primary">
            {NAV.map((n) => {
              const active = n.href === '/' ? path === '/' : path.startsWith(n.href);
              return (
                <Link
                  key={n.href}
                  href={n.href}
                  class={`relative flex items-center px-5 h-12 text-[14px] tracking-[0.16px] transition ${
                    active
                      ? 'text-ink-300 font-medium'
                      : 'text-ink-400 hover:text-ink-300 hover:bg-ink-800/60'
                  }`}
                >
                  {n.label}
                  {active && (
                    <span
                      class="absolute inset-x-3 bottom-0 h-0.5 bg-accent-500"
                      aria-hidden="true"
                    />
                  )}
                </Link>
              );
            })}
          </nav>
        </div>
      </header>

      <main class="w-full px-6 py-6">
        <Switch>
          <Route path="/" component={Home} />
          <Route path="/fleet" component={FleetView} />
          <Route path="/scans" component={ScanView} />
          <Route>
            <p class="text-ink-500">Not found.</p>
          </Route>
        </Switch>
      </main>
    </div>
  );
}
