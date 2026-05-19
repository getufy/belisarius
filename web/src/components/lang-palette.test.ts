import { describe, expect, test } from 'vitest';
import { languageColors, languageStroke, KNOWN_LANGUAGES } from './lang-palette';

// Pure-function tests for the language palette. No DOM, no fixtures.
// These exist as the smoke test for the vitest+preact toolchain — if
// `pnpm test` works at all, this file is the cheapest signal.

describe('languageColors', () => {
  test('returns the entry for a known language', () => {
    const c = languageColors('rust');
    expect(c.stroke).toBe('#d97a5d');
    expect(c.fill).toBe('#fdebe5');
  });

  test('is case-insensitive', () => {
    expect(languageColors('Rust')).toEqual(languageColors('rust'));
    expect(languageColors('TYPESCRIPT')).toEqual(languageColors('typescript'));
  });

  test('returns the default colors for null / undefined / empty', () => {
    const def = { stroke: '#525252', fill: '#ffffff' };
    expect(languageColors(null)).toEqual(def);
    expect(languageColors(undefined)).toEqual(def);
    expect(languageColors('')).toEqual(def);
  });

  test('returns the default colors for an unknown language', () => {
    const def = { stroke: '#525252', fill: '#ffffff' };
    expect(languageColors('cobol')).toEqual(def);
  });
});

describe('languageStroke', () => {
  test('returns just the stroke from the full color entry', () => {
    expect(languageStroke('rust')).toBe('#d97a5d');
    expect(languageStroke('python')).toBe('#24a148');
  });

  test('falls back to the default stroke for unknown input', () => {
    expect(languageStroke(undefined)).toBe('#525252');
  });
});

describe('KNOWN_LANGUAGES', () => {
  test('exports a non-empty list of language ids', () => {
    expect(KNOWN_LANGUAGES.length).toBeGreaterThan(5);
    expect(KNOWN_LANGUAGES).toContain('rust');
    expect(KNOWN_LANGUAGES).toContain('typescript');
  });

  test('every known language has a non-default color', () => {
    const defaultStroke = '#525252';
    for (const lang of KNOWN_LANGUAGES) {
      expect(languageStroke(lang)).not.toBe(defaultStroke);
    }
  });
});
