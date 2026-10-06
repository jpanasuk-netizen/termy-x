import { createFileRoute } from '@tanstack/react-router';
import { source } from '@/lib/source';
import { createFromSource } from 'fumadocs-core/search/server';

const server = createFromSource(source, {
  // https://docs.orama.com/docs/orama-js/supported-languages
  language: 'english',
});

// Serve the exported search index; it is prerendered at build time (see
// `pages` in vite.config.ts) and queried client-side, so the running server
// never holds the Orama index in memory.
export const Route = createFileRoute('/api/search.json')({
  server: {
    handlers: {
      GET: async () => server.staticGET(),
    },
  },
});
