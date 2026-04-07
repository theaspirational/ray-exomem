/**
 * Global app state shared across pages.
 * Holds the selected Exom and connection status.
 */

import {
	DEFAULT_EXOM,
	ExomemLiveState,
	fetchExomemStatus,
	fetchExoms,
	getExomemBaseUrl
} from '$lib/exomem.svelte';
import type { ExomEntry } from '$lib/types';

class AppState {
	selectedExom = $state(DEFAULT_EXOM);
	exoms = $state<ExomEntry[]>([]);
	baseUrl = $state(getExomemBaseUrl());
	/** Server process uptime for the selected exom (seconds), from `/status`. */
	serverUptimeSec = $state<number | null>(null);
	live = new ExomemLiveState();

	activeExoms = $derived(this.exoms.filter((e) => !e.archived));

	async refreshExoms() {
		try {
			this.exoms = await fetchExoms();
		} catch {
			// silently fail — individual pages handle errors
		}
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
