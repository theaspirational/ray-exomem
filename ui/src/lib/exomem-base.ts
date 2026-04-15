import { browser } from '$app/environment';
import { env } from '$env/dynamic/public';

const DEFAULT_BASE_URL = 'http://127.0.0.1:9780';

function normalizeBaseUrl(baseUrl: string): string {
	const trimmed = baseUrl.trim().replace(/\/+$/, '');
	return trimmed.endsWith('/ray-exomem') ? trimmed : `${trimmed}/ray-exomem`;
}

export function getExomemBaseUrl(): string {
	const configured = env.PUBLIC_TEIDE_EXOMEM_BASE_URL?.trim();
	if (configured) return normalizeBaseUrl(configured);

	if (browser) {
		const { origin, port } = window.location;
		// In the Vite dev server, the UI runs on 5173 and the daemon still lives on 9780.
		// When the UI is served from the daemon itself, use the current origin so LAN access
		// keeps working on phones/tablets and other machines.
		if (port !== '5173') return normalizeBaseUrl(origin);
	}

	return normalizeBaseUrl(DEFAULT_BASE_URL);
}
