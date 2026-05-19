import { describe, expect, test, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/preact';
import { useHashState, useRecentPaths } from './hooks';

// Hook tests exercise happy-dom — the URL/History APIs `useHashState`
// reads from, and the localStorage API `useRecentPaths` writes to.

beforeEach(() => {
  // Reset hash + localStorage so tests don't leak into each other.
  window.location.hash = '';
  window.localStorage.clear();
});

describe('useHashState', () => {
  test('returns the fallback when the hash has no entry for the key', () => {
    const { result } = renderHook(() => useHashState('tab', 'overview'));
    expect(result.current[0]).toBe('overview');
  });

  test('reads an existing value out of the hash', () => {
    window.location.hash = '#tab=hotspots&path=/x';
    const { result } = renderHook(() => useHashState('tab', 'overview'));
    expect(result.current[0]).toBe('hotspots');
  });

  test('writing a non-fallback value updates the hash', () => {
    const { result } = renderHook(() => useHashState('tab', 'overview'));
    act(() => result.current[1]('hotspots'));
    expect(window.location.hash).toContain('tab=hotspots');
    expect(result.current[0]).toBe('hotspots');
  });

  test('writing the fallback value removes the key from the hash', () => {
    window.location.hash = '#tab=hotspots';
    const { result } = renderHook(() => useHashState('tab', 'overview'));
    act(() => result.current[1]('overview'));
    expect(window.location.hash).not.toContain('tab=');
  });
});

describe('useRecentPaths', () => {
  test('starts empty when localStorage has no entry', () => {
    const { result } = renderHook(() => useRecentPaths());
    expect(result.current.paths).toEqual([]);
  });

  test('reads pre-existing entries from localStorage', () => {
    window.localStorage.setItem(
      'belisarius:recent-paths',
      JSON.stringify(['/a', '/b']),
    );
    const { result } = renderHook(() => useRecentPaths());
    expect(result.current.paths).toEqual(['/a', '/b']);
  });

  test('ignores corrupt JSON in localStorage', () => {
    window.localStorage.setItem('belisarius:recent-paths', 'this is { not json');
    const { result } = renderHook(() => useRecentPaths());
    expect(result.current.paths).toEqual([]);
  });

  test('remember dedupes and bumps to the front', () => {
    const { result } = renderHook(() => useRecentPaths());
    act(() => result.current.remember('/a'));
    act(() => result.current.remember('/b'));
    act(() => result.current.remember('/a')); // re-touch
    expect(result.current.paths).toEqual(['/a', '/b']);
  });

  test('remember truncates to `max`', () => {
    const { result } = renderHook(() => useRecentPaths('test-key', 2));
    act(() => result.current.remember('/a'));
    act(() => result.current.remember('/b'));
    act(() => result.current.remember('/c'));
    expect(result.current.paths).toEqual(['/c', '/b']);
  });

  test('remember ignores blank input', () => {
    const { result } = renderHook(() => useRecentPaths());
    act(() => result.current.remember('   '));
    expect(result.current.paths).toEqual([]);
  });
});
