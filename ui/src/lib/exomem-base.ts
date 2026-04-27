import { browser } from '$app/environment';
import { base } from '$app/paths';
import { env } from '$env/dynamic/public';

const DEFAULT_BASE_URL = 'http://127.0.0.1:9780';

function trimSlash(s: string): string {
	return s.trim().replace(/\/+$/, '');
}

/** Origin (scheme + host + port) the daemon is reachable at — no path suffix. */
export function getDaemonOrigin(): string {
	const configured = env.PUBLIC_TEIDE_EXOMEM_BASE_URL?.trim();
	if (configured) return trimSlash(configured);

	if (browser) {
		const { origin, port } = window.location;
		// In the Vite dev server, the UI runs on 5173 and the daemon lives on 9780.
		// When the UI is served from the daemon itself, use the current origin so LAN
		// access keeps working on phones/tablets and other machines.
		if (port !== '5173') return trimSlash(origin);
	}

	return trimSlash(DEFAULT_BASE_URL);
}

/** Full prefix to which `/api/...`, `/auth/...`, `/mcp`, `/events` are appended. */
export function getExomemBaseUrl(): string {
	return `${getDaemonOrigin()}${base}`;
}
