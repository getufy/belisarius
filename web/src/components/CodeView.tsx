import { useEffect, useState } from 'preact/hooks';
import { api, Snippet } from '../api';

type Props = {
  path: string;        // project root
  file: string;        // relative file
  line: number;        // 1-indexed target line
  radius?: number;
  onClose: () => void;
};

export function CodeView({ path, file, line, radius = 30, onClose }: Props) {
  const [data, setData] = useState<Snippet | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    setData(null);
    setErr(null);
    api
      .snippet(path, file, line, radius)
      .then(setData)
      .catch((e) => setErr(String(e)));
  }, [path, file, line, radius]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div
      class="fixed inset-0 z-50 flex items-stretch justify-center bg-black/40 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        class="m-4 w-full max-w-5xl border border-ink-700 bg-ink-900 shadow-xl flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <header class="flex items-baseline justify-between gap-3 border-b border-ink-700 px-4 py-2">
          <div class="min-w-0">
            <p class="text-[10px] uppercase tracking-wider text-ink-500">code preview</p>
            <p class="text-sm text-ink-300 truncate">
              <code class="text-accent-500">{file}</code>:{line}
            </p>
          </div>
          <div class="flex items-center gap-3 text-xs text-ink-500">
            {data && (
              <span>
                {data.start_line}–{data.end_line} of {data.total_lines}
                {data.language && <span class="ml-2 pill">{data.language}</span>}
              </span>
            )}
            <EditorLinks path={path} file={file} line={line} />
            <button
              class="btn-mini"
              onClick={onClose}
              title="Close (esc)"
            >
              esc
            </button>
          </div>
        </header>

        <div class="flex-1 overflow-auto bg-ink-800">
          {err && <p class="p-4 text-ink-err text-sm">{err}</p>}
          {!err && !data && <p class="p-4 text-ink-500 text-sm">Loading…</p>}
          {data && <Lines snippet={data} target={line} />}
        </div>
      </div>
    </div>
  );
}

function EditorLinks({ path, file, line }: { path: string; file: string; line: number }) {
  // `path` is project root as the server sees it. For absolute paths the
  // vscode:// URL works directly; for `.` or relative paths we can't construct
  // an absolute path from the browser, so the link is best-effort.
  const isAbsolute = path.startsWith('/');
  const full = isAbsolute ? `${path.replace(/\/$/, '')}/${file}` : null;
  if (!full) return null;
  return (
    <>
      <a
        class="btn-mini"
        href={`vscode://file${full}:${line}`}
        title="Open in VS Code"
      >
        code
      </a>
      <a
        class="btn-mini"
        href={`cursor://file${full}:${line}`}
        title="Open in Cursor"
      >
        cursor
      </a>
    </>
  );
}

function Lines({ snippet, target }: { snippet: Snippet; target: number }) {
  const rows = snippet.snippet.split('\n');
  const pad = String(snippet.end_line).length;
  return (
    <pre class="text-[12px] leading-5 font-mono">
      {rows.map((row, i) => {
        const lineNo = snippet.start_line + i;
        const isTarget = lineNo === target;
        return (
          <div
            key={lineNo}
            class={`flex ${isTarget ? 'bg-accent-500/10' : ''}`}
            ref={(el) => {
              if (el && isTarget) {
                queueMicrotask(() => el.scrollIntoView({ block: 'center' }));
              }
            }}
          >
            <span
              class={`select-none pr-3 pl-3 text-right ${
                isTarget ? 'text-accent-500' : 'text-ink-600'
              }`}
              style={{ minWidth: `${pad + 2}ch` }}
            >
              {String(lineNo).padStart(pad, ' ')}
            </span>
            <span class={isTarget ? 'text-ink-300' : 'text-ink-300'}>
              {row || '\u00a0'}
            </span>
          </div>
        );
      })}
    </pre>
  );
}
