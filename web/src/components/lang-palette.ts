// Single source of truth for the language colors used across the FileGraph,
// LocTreemap, ArchitectureView, and the Mermaid export. Strokes are vivid
// Carbon-aligned hues; fills are light tints so nodes read on a white canvas.

export type LangColors = {
  stroke: string;
  fill: string;
};

const TABLE: Record<string, LangColors> = {
  rust: { stroke: '#d97a5d', fill: '#fdebe5' },
  typescript: { stroke: '#0f62fe', fill: '#e6efff' },
  tsx: { stroke: '#0f62fe', fill: '#e6efff' },
  javascript: { stroke: '#bca84a', fill: '#fdf4cf' },
  python: { stroke: '#24a148', fill: '#dff0e3' },
  go: { stroke: '#0098a6', fill: '#dceff2' },
  java: { stroke: '#8a3ffc', fill: '#efe6f6' },
  ruby: { stroke: '#da1e28', fill: '#fde2e3' },
  css: { stroke: '#8a3ffc', fill: '#efe6f6' },
  html: { stroke: '#8a3ffc', fill: '#efe6f6' },
  toml: { stroke: '#8c8c8c', fill: '#f4f4f4' },
  json: { stroke: '#8c8c8c', fill: '#f4f4f4' },
  markdown: { stroke: '#8c8c8c', fill: '#f4f4f4' },
  yaml: { stroke: '#8c8c8c', fill: '#f4f4f4' },
};

const DEFAULT_COLORS: LangColors = { stroke: '#525252', fill: '#ffffff' };

export function languageColors(lang: string | null | undefined): LangColors {
  if (!lang) return DEFAULT_COLORS;
  return TABLE[lang.toLowerCase()] ?? DEFAULT_COLORS;
}

/** Convenience for FileGraph and other consumers that only need the stroke. */
export function languageStroke(lang: string | null | undefined): string {
  return languageColors(lang).stroke;
}

export const KNOWN_LANGUAGES = Object.keys(TABLE);
