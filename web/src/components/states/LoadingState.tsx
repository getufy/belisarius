import type { JSX } from 'preact';

interface LoadingStateProps {
  label?: string;
}

export function LoadingState({ label = 'Loading…' }: LoadingStateProps): JSX.Element {
  return (
    <div
      role="status"
      aria-live="polite"
      class="card flex flex-col items-stretch gap-3 py-6"
    >
      <div class="flex items-center gap-2 text-xs text-ink-500">
        <span
          aria-hidden="true"
          class="inline-block h-3 w-3 animate-spin rounded-full border-2 border-ink-300 border-t-transparent"
        />
        <span>{label}</span>
      </div>
      <div class="space-y-2" aria-hidden="true">
        <div class="h-2 w-3/4 animate-pulse bg-ink-100" />
        <div class="h-2 w-5/6 animate-pulse bg-ink-100" />
        <div class="h-2 w-2/3 animate-pulse bg-ink-100" />
      </div>
    </div>
  );
}

export default LoadingState;
