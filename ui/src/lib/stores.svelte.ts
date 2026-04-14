/**
 * Global app state shared across pages.
 * Holds the selected Exom and connection status.
 */

import {
	DEFAULT_EXOM,
	ExomemLiveState,
	fetchExomemStatus,
	getExomemBaseUrl
} from '$lib/exomem.svelte';

class AppState {
	selectedExom = $state(DEFAULT_EXOM);
	baseUrl = $state(getExomemBaseUrl());
	/** Server process uptime for the selected exom (seconds), from `/status`. */
	serverUptimeSec = $state<number | null>(null);
	live = new ExomemLiveState();

	async refreshExoms() {
		// No-op: tree UI uses fetchTree() directly. Old /api/exoms endpoint removed.
	}

	switchExom(name: string) {
		this.selectedExom = name;
	}

	async refreshServerUptime() {
		try {
			const s = await fetchExomemStatus(this.selectedExom);
			this.serverUptimeSec = s.server.uptime_sec;
		} catch {
			this.serverUptimeSec = null;
		}
	}
}

export const app = new AppState();
