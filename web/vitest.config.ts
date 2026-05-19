import { defineConfig } from 'vitest/config';
import preact from '@preact/preset-vite';

// Vitest config for the web/ package. happy-dom over jsdom for ~3× startup
// speed; the components we test only need DOM primitives + URL/History APIs.
//
// Test files live next to the source they cover, named `*.test.ts(x)`. Run
// with `pnpm test` (one-shot) or `pnpm test:watch` (re-runs on save).
export default defineConfig({
  plugins: [preact()],
  test: {
    environment: 'happy-dom',
    globals: false,
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['./vitest.setup.ts'],
  },
});
