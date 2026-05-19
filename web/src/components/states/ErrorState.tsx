import type { JSX } from 'preact';

interface ErrorStateProps {
  error: unknown;
  onRetry?: () => void;
  title?: string;
}

function formatError(err: unknown): string {
  if (err == null) return 'Unknown error';
  if (typeof err === 'string') return err;
  if (err instanceof Error) return err.message;
  try {
    return String(err);
  } catch {
    return 'Unknown error';
  }
}

export function ErrorState({ error, onRetry, title = 'Something went wrong' }: ErrorStateProps): JSX.Element {
  const message = formatError(error);
  return (
    <div
      role="alert"
      class="flex flex-col items-start gap-2 border border-ink-errBorder bg-ink-errSubtle p-4 text-ink-err"
    >
      <div class="flex items-center gap-2 text-sm font-medium">
        <span aria-hidden="true" class="inline-block h-2 w-2 rounded-full bg-ink-err" />
        <span>{title}</span>
      </div>
      <div class="text-xs text-ink-errBorder break-words">{message}</div>
      {onRetry ? (
        <button
          type="button"
          onClick={onRetry}
          class="mt-2 border border-ink-errBorder bg-white px-3 py-1 text-xs font-medium text-ink-err hover:bg-white/80"
        >
          Retry
        </button>
      ) : null}
    </div>
  );
}

export default ErrorState;
