import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

// The production OpenAPI download reads the repository's canonical contract.
export default defineConfig({ server: { allowedHosts: ['console.test'], fs: { allow: [fileURLToPath(new URL('../../../../../', import.meta.url))] } } });
