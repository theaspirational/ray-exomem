/**
 * Global app state shared across pages.
 * Holds the selected Exom and connection status.
 */

import {
	ExomemLiveState,
	fetchExomemStatus,
	getExomemBaseUrl
} from '$lib/exomem.svelte';

class AppState {
	selectedExom = $state<string | null>(null);
	baseUrl = $state(getExomemBaseUrl());
	/** Server process uptime for the selected exom (seconds), from `/status`. */
	serverUptimeSec = $state<number | null>(null);
	live = new ExomemLiveState();

	switchExom(name: string | null) {
		this.selectedExom = name;
	}

	clearSelection() {
		this.selectedExom = null;
		this.serverUptimeSec = null;
	}

	async refreshServerUptime() {
		if (!this.selectedExom) {
			this.serverUptimeSec = null;
			return;
		}
		try {
			const s = await fetchExomemStatus(this.selectedExom);
			this.serverUptimeSec = s.server.uptime_sec;
		} catch {
			this.serverUptimeSec = null;
		}
	}
}

export const app = new AppState();
