import type { ComponentChildren, JSX } from 'preact';

interface EmptyStateProps {
  title: string;
  hint?: string;
  action?: ComponentChildren;
  icon?: ComponentChildren;
}

export function EmptyState({ title, hint, action, icon }: EmptyStateProps): JSX.Element {
  return (
    <div class="card flex flex-col items-center justify-center gap-2 py-10 text-center">
      {icon ? <div class="mb-1 text-ink-400">{icon}</div> : null}
      <div class="text-sm font-medium text-ink-300">{title}</div>
      {hint ? <div class="max-w-md text-xs text-ink-500">{hint}</div> : null}
      {action ? <div class="mt-2">{action}</div> : null}
    </div>
  );
}

export default EmptyState;
