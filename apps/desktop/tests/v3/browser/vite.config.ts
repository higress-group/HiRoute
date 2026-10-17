import { defineConfig } from 'vite';

// The production OpenAPI download reads the repository's canonical contract.
export default defineConfig({ server: { allowedHosts: ['console.test'], fs: { allow: [new URL('../../../../../', import.meta.url).pathname] } } });
