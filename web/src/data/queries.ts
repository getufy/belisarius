// React Query hooks — the single way components fetch Belisarius data.
//
// Why this exists: every tab used to copy the same `useState<T | null>` +
// `useState<string | null>` + `useEffect` + `api.X(path).then(setData).catch(setErr)`
// pattern, leaking requests on rapid tab switches and forcing each tab to
// reimplement caching. This module replaces that with typed `useFoo()` hooks
// that share one `QueryClient` (installed in `App.tsx`). React Query handles:
//   - request de-duplication (two tabs asking for the same path on mount =
//     one network call)
//   - automatic cache hits when revisiting a tab inside `staleTime`
//   - cancellation on unmount (the result is discarded if the component is
//     gone before the network call returns)
//   - error / loading state without per-component bookkeeping
//
// Component usage:
//   const { data, error, isLoading } = useQuality(path);
//   if (error) return <Err>{String(error)}</Err>;
//   if (!data) return <Loading />;
//   ...

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  api,
  type ArchitectureGraph,
  type ArchitectureMermaid,
  type Brief,
  type CallersResponse,
  type ComponentsResponse,
  type ContextListResponse,
  type DiagToolStatus,
  type DiagnosticsReport,
  type FileDsm,
  type FlowReport,
  type FunctionDetail,
  type FunctionsResponse,
  type HotspotsResponse,
  type ImpactReport,
  type IndexStatus,
  type MarkersResponse,
  type QualitySummary,
  type RefsResponse,
  type Scan,
  type SurfaceReport,
  type Symbol360,
  type SymbolMatch,
  type SymbolStatus,
  type TestMap,
  type WorkspaceCommands,
} from '../api';

// ─── Project-level (the bulk) ─────────────────────────────────────────────

// Every hook's `queryFn` destructures `signal` from the `QueryFunctionContext`
// react-query passes it. The signal flows through `api.X(..., signal)` and on
// to `fetch`, so unmounting / rapid path changes cancel the underlying
// network request — not just the framework-level promise.

export const useQuality = (path: string) =>
  useQuery<QualitySummary>({
    queryKey: ['quality', path],
    queryFn: ({ signal }) => api.quality(path, signal),
    enabled: !!path,
  });

export const useBrief = (path: string) =>
  useQuery<Brief>({
    queryKey: ['brief', path],
    queryFn: ({ signal }) => api.brief(path, signal),
    enabled: !!path,
  });

export const useScan = (path: string) =>
  useQuery<Scan>({
    queryKey: ['scan', path],
    queryFn: ({ signal }) => api.scan(path, signal),
    enabled: !!path,
  });

export const useFunctions = (
  path: string,
  opts: { minCc?: number; limit?: number; sortBy?: string; file?: string } = {}
) =>
  useQuery<FunctionsResponse>({
    queryKey: ['functions', path, opts],
    queryFn: ({ signal }) => api.functions(path, opts, signal),
    enabled: !!path,
  });

export const useHotspots = (path: string, days = 90, limit = 25) =>
  useQuery<HotspotsResponse>({
    queryKey: ['hotspots', path, days, limit],
    queryFn: ({ signal }) => api.hotspots(path, days, limit, signal),
    enabled: !!path,
  });

export const useTestGaps = (path: string, limit = 25) =>
  useQuery<TestMap>({
    queryKey: ['test_gaps', path, limit],
    queryFn: ({ signal }) => api.testGaps(path, limit, signal),
    enabled: !!path,
  });

export const useMarkers = (path: string, limit = 500) =>
  useQuery<MarkersResponse>({
    queryKey: ['markers', path, limit],
    queryFn: ({ signal }) => api.markers(path, limit, signal),
    enabled: !!path,
  });

export const useFileDsm = (path: string, file: string) =>
  useQuery<FileDsm>({
    queryKey: ['file_dsm', path, file],
    queryFn: ({ signal }) => api.fileDsm(path, file, signal),
    enabled: !!(path && file),
  });

export const useFunctionDetail = (path: string, file: string, name: string) =>
  useQuery<FunctionDetail>({
    queryKey: ['function_detail', path, file, name],
    queryFn: ({ signal }) => api.functionDetail(path, file, name, signal),
    enabled: !!(path && file && name),
  });

export const useSurface = (path: string) =>
  useQuery<SurfaceReport>({
    queryKey: ['surface', path],
    queryFn: ({ signal }) => api.surface(path, signal),
    enabled: !!path,
  });

export const useComponents = (path: string) =>
  useQuery<ComponentsResponse>({
    queryKey: ['components', path],
    queryFn: ({ signal }) => api.components(path, signal),
    enabled: !!path,
  });

export const useCommands = (path: string) =>
  useQuery<WorkspaceCommands>({
    queryKey: ['commands', path],
    queryFn: ({ signal }) => api.commands(path, signal),
    enabled: !!path,
  });

// ─── Symbols (SCIP) ───────────────────────────────────────────────────────

export const useSymbolsStatus = (path: string) =>
  useQuery<SymbolStatus>({
    queryKey: ['symbols_status', path],
    queryFn: ({ signal }) => api.symbolsStatus(path, signal),
    enabled: !!path,
  });

export const useSymbolsSearch = (path: string, q: string, limit = 30) =>
  useQuery<SymbolMatch[]>({
    queryKey: ['symbols_search', path, q, limit],
    queryFn: ({ signal }) => api.symbolsSearch(path, q, limit, signal),
    // Don't auto-fire until the query has at least 2 chars — avoids burning
    // the SCIP store on every keystroke from "" / "a".
    enabled: !!path && q.trim().length >= 2,
  });

export const useSymbolsRefs = (path: string, sym: string) =>
  useQuery<RefsResponse>({
    queryKey: ['symbols_refs', path, sym],
    queryFn: ({ signal }) => api.symbolsRefs(path, sym, signal),
    enabled: !!(path && sym),
  });

export const useSymbolsCallers = (path: string, sym: string) =>
  useQuery<CallersResponse>({
    queryKey: ['symbols_callers', path, sym],
    queryFn: ({ signal }) => api.symbolsCallers(path, sym, signal),
    enabled: !!(path && sym),
  });

// ─── Diagnostics ──────────────────────────────────────────────────────────

export const useDiagnostics = (path: string) =>
  useQuery<{ cached: boolean; report: DiagnosticsReport }>({
    queryKey: ['diagnostics_run', path],
    // Mutations vs cancellation: a long-running diagnostics run shouldn't be
    // aborted just because the user navigated tabs — they probably want the
    // result waiting for them. Skip `signal` for this one on purpose.
    queryFn: () => api.diagnosticsRun(path),
    enabled: !!path,
    // Diagnostics can be slow (clippy, semgrep, ruff, eslint). Don't auto-refetch.
    staleTime: Infinity,
  });

export const useDiagnosticsStatus = (path: string) =>
  useQuery<DiagToolStatus[]>({
    queryKey: ['diagnostics_status', path],
    queryFn: ({ signal }) => api.diagnosticsStatus(path, signal).then((r) => r.tools),
    enabled: !!path,
  });

/// Re-run diagnostics on demand. Components fire `mutation.mutate({ force })`
/// from a button; the report mounts back into the same cache key so
/// `useDiagnostics` consumers see the fresh report without a refetch.
export const useRunDiagnostics = (path: string) => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ force }: { force: boolean }) => api.diagnosticsRun(path, undefined, force),
    onSuccess: (data) => {
      qc.setQueryData(['diagnostics_run', path], data);
    },
  });
};

// ─── Architecture (Mermaid + Cytoscape-shape graph) ──────────────────────

export const useArchitectureMermaid = (
  path: string,
  opts: { view?: 'module' | 'file'; maxNodes?: number; groupDepth?: number } = {},
) =>
  useQuery<ArchitectureMermaid>({
    queryKey: ['arch_mermaid', path, opts],
    queryFn: ({ signal }) => api.architectureMermaid(path, opts, signal),
    enabled: !!path,
  });

export const useArchitectureGraph = (
  path: string,
  opts: { view?: 'module' | 'file'; maxNodes?: number; groupDepth?: number } = {},
) =>
  useQuery<ArchitectureGraph>({
    queryKey: ['arch_graph', path, opts],
    queryFn: ({ signal }) => api.architectureGraph(path, opts, signal),
    enabled: !!path,
  });

export const useArchitectureModule = (
  path: string,
  modulePath: string | null,
  groupDepth: number,
) =>
  useQuery({
    queryKey: ['arch_module', path, modulePath, groupDepth],
    queryFn: ({ signal }) =>
      api.architectureModule(path, modulePath!, groupDepth, signal),
    enabled: !!(path && modulePath),
  });

// ─── Transitive xref (impact / flow / symbol 360°) ───────────────────────

export const useImpact = (path: string, sym: string, depth = 3) =>
  useQuery<ImpactReport>({
    queryKey: ['impact', path, sym, depth],
    queryFn: ({ signal }) => api.impact(path, sym, depth, signal),
    enabled: !!(path && sym),
  });

export const useFlow = (path: string, sym: string, depth = 3) =>
  useQuery<FlowReport>({
    queryKey: ['flow', path, sym, depth],
    queryFn: ({ signal }) => api.flow(path, sym, depth, signal),
    enabled: !!(path && sym),
  });

export const useSymbol360 = (path: string, sym: string) =>
  useQuery<Symbol360>({
    queryKey: ['symbol_360', path, sym],
    queryFn: ({ signal }) => api.symbol360(path, sym, signal),
    enabled: !!(path && sym),
  });

// ─── Hybrid code search ──────────────────────────────────────────────────

export const useSearchStatus = (path: string) =>
  useQuery<IndexStatus>({
    queryKey: ['search_status', path],
    queryFn: ({ signal }) => api.searchStatus(path, signal),
    enabled: !!path,
    // The search index status is cheap and worth polling on tab focus so
    // a reindex kicked off elsewhere shows up.
    refetchOnWindowFocus: true,
  });

// ─── Context artifacts ───────────────────────────────────────────────────

export const useContextList = (path: string) =>
  useQuery<ContextListResponse>({
    queryKey: ['context_list', path],
    queryFn: ({ signal }) => api.contextList(path, signal),
    enabled: !!path,
  });

export const useContextGet = (path: string, name: string | null) =>
  useQuery({
    queryKey: ['context_get', path, name],
    queryFn: ({ signal }) => api.contextGet(path, name!, signal),
    enabled: !!(path && name),
  });
