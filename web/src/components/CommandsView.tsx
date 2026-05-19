import { NamedCommand } from '../api';
import { useCommands } from '../data/queries';

export function CommandsView({ path }: { path: string }) {
  const { data, error } = useCommands(path);
  const err = error ? String(error) : null;

  if (err) return <p class="card text-ink-err">{err}</p>;
  if (!data) return <p class="card text-ink-500">Discovering commands…</p>;

  const s = data.suggested;
  const sections: { label: string; rows: NamedCommand[] }[] = [
    { label: 'package.json scripts', rows: data.package_scripts },
    { label: 'Cargo', rows: data.cargo },
    { label: 'Justfile', rows: data.just },
    { label: 'Makefile', rows: data.make },
    { label: 'Python', rows: data.python },
    { label: '.github/workflows', rows: data.workflows },
  ].filter((sec) => sec.rows.length > 0);

  const total = sections.reduce((n, sec) => n + sec.rows.length, 0);

  return (
    <div class="space-y-4">
      <section class="card space-y-2">
        <div class="flex items-baseline justify-between">
          <h2 class="text-sm uppercase tracking-wider text-ink-500">
            Suggested commands
          </h2>
          <span class="text-xs text-ink-500">
            {total} commands discovered across {sections.length} source
            {sections.length === 1 ? '' : 's'}
          </span>
        </div>
        <p class="text-xs text-ink-500">
          One-liners we'd recommend for the most common workflows. Heuristic
          — verify before relying on them in CI scripts.
        </p>
        <div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-5 text-xs">
          <SuggestedTile label="run" value={s.run} />
          <SuggestedTile label="build" value={s.build} />
          <SuggestedTile label="test" value={s.test} />
          <SuggestedTile label="lint" value={s.lint} />
          <SuggestedTile label="format" value={s.format} />
        </div>
      </section>

      {sections.length === 0 ? (
        <p class="card text-sm text-ink-500">
          No runnable commands found — no <code>package.json</code>,{' '}
          <code>Justfile</code>, <code>Makefile</code>, <code>Cargo.toml</code>{' '}
          (with detectable targets), <code>pyproject.toml</code> scripts, or
          <code>.github/workflows</code> in this project.
        </p>
      ) : (
        sections.map((sec) => (
          <CommandSection key={sec.label} label={sec.label} rows={sec.rows} />
        ))
      )}
    </div>
  );
}

function SuggestedTile({ label, value }: { label: string; value: string | null }) {
  return (
    <div class="border border-ink-700 p-2">
      <div class="text-[10px] uppercase tracking-wider text-ink-500">{label}</div>
      {value ? (
        <code class="block text-accent-500 text-[11px] break-all mt-1">
          {value}
        </code>
      ) : (
        <span class="text-ink-500 text-[11px] italic">not found</span>
      )}
    </div>
  );
}

function CommandSection({ label, rows }: { label: string; rows: NamedCommand[] }) {
  return (
    <details class="card p-0">
      <summary class="cursor-pointer hover:bg-ink-800 select-none px-3 py-2 border-b border-ink-700 flex items-center gap-2">
        <span class="text-ink-600">▸</span>
        <h3 class="text-sm uppercase tracking-wider text-ink-500">{label}</h3>
        <span class="text-xs text-ink-500 ml-auto">{rows.length} commands</span>
      </summary>
      <table class="w-full text-xs">
        <thead class="text-ink-500 uppercase tracking-wider">
          <tr>
            <th class="text-left px-3 py-2 w-44">name</th>
            <th class="text-left px-3 py-2 w-24">purpose</th>
            <th class="text-left px-3 py-2">command</th>
            <th class="text-left px-3 py-2 w-56">source</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr
              key={`${r.source}|${r.name}|${i}`}
              class="border-t border-ink-700"
            >
              <td class="px-3 py-1 text-ink-300 truncate max-w-[180px]">
                {r.name}
              </td>
              <td class="px-3 py-1">
                <PurposeBadge purpose={r.purpose} />
              </td>
              <td class="px-3 py-1">
                <code class="text-accent-500 break-all">{r.command}</code>
              </td>
              <td class="px-3 py-1 text-ink-500 truncate max-w-[260px]">
                {r.source}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </details>
  );
}

function PurposeBadge({ purpose }: { purpose: string }) {
  const color =
    {
      test: 'text-green-400',
      lint: 'text-ink-warning',
      format: 'text-ink-warning',
      build: 'text-blue-400',
      dev: 'text-accent-500',
      run: 'text-accent-500',
      release: 'text-orange-400',
      ci: 'text-purple-400',
      other: 'text-ink-500',
    }[purpose] || 'text-ink-500';
  return <span class={`text-[10px] uppercase tracking-wider ${color}`}>{purpose}</span>;
}
