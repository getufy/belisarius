// Centralized color constants for inline styles where Tailwind classes can't apply
// (e.g. JS template literals, React style={{}} props, SVG attributes).
// For class-based styling, prefer Tailwind tokens like `text-ink-err`.

export const COLORS = {
  err: '#da1e28',
  warning: '#f1c21b',
  success: '#24a148',
  errSubtle: '#fff1f1',
  warningSubtle: '#fcf4d6',
  successSubtle: '#defbe6',
} as const;
