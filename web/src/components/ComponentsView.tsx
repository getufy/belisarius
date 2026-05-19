import { useMemo, useState } from 'preact/hooks';
import { useComponents } from '../data/queries';
import { CodeView } from './CodeView';

export function ComponentsView({ path }: { path: string }) {
  const { data, error } = useComponents(path);
  const components = data?.components ?? null;
  const err = error ? String(error) : null;
  const [search, setSearch] = useState('');
  const [preview, setPreview] = useState<{ file: string; line: number } | null>(null);

  const filtered = useMemo(() => {
    if (!components) return [];
    const needle = search.trim().toLowerCase();
    if (!needle) return components;
    return components.filter(
      (c) =>
        c.name.toLowerCase().includes(needle) ||
        c.file.toLowerCase().includes(needle) ||
        c.props.some((p) => p.name.toLowerCase().includes(needle))
    );
  }, [components, search]);

  if (err) return <p class="card text-ink-err">{err}</p>;
  if (!components)
    return (
      <p class="card text-ink-500">
        Running react-docgen on the project's .tsx / .jsx files…
      </p>
    );
  if (components.length === 0) {
    return (
      <div class="card text-sm text-ink-400 space-y-2">
        <p>
          <strong>react-docgen didn't return any components.</strong> Most likely
          it isn't installed in the project's <code class="text-accent-500">node_modules</code>.
        </p>
        <p class="text-xs text-ink-500">
          Add it once and rerun the scan:
        </p>
        <pre class="bg-ink-800 p-3 text-[11px] text-accent-500">
          cd {path === '.' ? 'web' : path}
          pnpm add -D react-docgen
        </pre>
        <p class="text-xs text-ink-500">
          react-docgen is invoked via{' '}
          <code class="text-accent-500">npx --no-install</code>, so it must be a
          project dependency.
        </p>
      </div>
    );
  }

  return (
    <div class="space-y-4">
      <div class="card flex flex-wrap items-baseline gap-3">
        <div class="flex-1 min-w-[260px]">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">
            Design-system components
          </h2>
          <p class="text-xs text-ink-500">
            react-docgen pass over the .tsx / .jsx files. Props with their
            types, defaults, required status, and docstrings. {components.length}{' '}
            component{components.length === 1 ? '' : 's'} found.
          </p>
        </div>
        <input
          class="field max-w-[280px]"
          placeholder="search name, file, prop…"
          value={search}
          onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
        />
      </div>

      {filtered.length === 0 && (
        <p class="card text-sm text-ink-500">No components match.</p>
      )}

      <div class="grid gap-3 lg:grid-cols-2">
        {filtered.map((c) => (
          <div key={`${c.file}:${c.name}`} class="card space-y-2">
            <header
              class="flex items-baseline justify-between gap-2 cursor-pointer"
              onClick={() => setPreview({ file: c.file, line: 1 })}
              title="Preview source"
            >
              <div class="min-w-0">
                <p class="text-sm text-accent-500 font-medium truncate">
                  {c.name}
                </p>
                <p class="text-[10px] text-ink-500 truncate">{c.file}</p>
              </div>
              <span class="pill text-ink-400 shrink-0">
                {c.props.length} prop{c.props.length === 1 ? '' : 's'}
              </span>
            </header>

            {c.description && (
              <p class="text-xs text-ink-400 whitespace-pre-wrap">
                {c.description}
              </p>
            )}

            {c.props.length > 0 ? (
              <table class="w-full text-[11px]">
                <thead class="text-ink-500 uppercase tracking-wider">
                  <tr>
                    <th class="text-left py-1">prop</th>
                    <th class="text-left py-1">type</th>
                    <th class="text-left py-1">default</th>
                  </tr>
                </thead>
                <tbody>
                  {c.props.map((p) => (
                    <tr key={p.name} class="border-t border-ink-700">
                      <td class="py-0.5 pr-2">
                        <code
                          class={
                            p.required ? 'text-accent-500' : 'text-ink-300'
                          }
                          title={p.description}
                        >
                          {p.name}
                          {p.required && '*'}
                        </code>
                      </td>
                      <td class="py-0.5 pr-2 text-ink-400 truncate max-w-[180px]">
                        {p.type || '—'}
                      </td>
                      <td class="py-0.5 text-ink-500 truncate max-w-[140px]">
                        {p.default ?? '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p class="text-[11px] text-ink-500">No props detected.</p>
            )}
          </div>
        ))}
      </div>

      {preview && (
        <CodeView
          path={path}
          file={preview.file}
          line={preview.line}
          onClose={() => setPreview(null)}
        />
      )}
    </div>
  );
}
