import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	preprocess: vitePreprocess(),
	compilerOptions: {
		runes: true
	},
	kit: {
		// Must match the daemon's BASE_PATH (server.rs::BASE_PATH, baked from
		// $RAY_EXOMEM_BASE_PATH at build time). Empty = root mount.
		paths: {
			base: (process.env.RAY_EXOMEM_BASE_PATH || '').replace(/\/+$/, '')
		},
		adapter: adapter({
			pages: 'build',
			assets: 'build',
			fallback: 'index.html'
		})
	}
};

export default config;
