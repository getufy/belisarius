// Global vitest setup: extend `expect` with @testing-library/jest-dom matchers
// (toBeInTheDocument, toHaveTextContent, etc.) so component tests read
// naturally. Loaded once before the test suite via `setupFiles` in
// `vitest.config.ts`.
import '@testing-library/jest-dom/vitest';
