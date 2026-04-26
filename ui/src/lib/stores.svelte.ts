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
	selectedExom = $state<string | null>(null);
	baseUrl = $state(getExomemBaseUrl());
	/** Server process uptime for the selected exom (seconds), from `/status`. */
	serverUptimeSec = $state<number | null>(null);
	live = new ExomemLiveState();

	async refreshExoms() {
		// No-op: tree UI uses fetchTree() directly. Old /api/exoms endpoint removed.
	}

	/**
	 * Path the UI lands on for an authenticated user with no prior selection.
	 * `main` is the bare exom auto-created on a fresh persistent state, so it
	 * is guaranteed to exist regardless of which (if any) bootstrap fixtures
	 * are configured.
	 */
	defaultExomForUser(_email: string): string {
		return 'main';
	}

	ensureAuthenticatedDefaultExom(email: string | null) {
		if (!email) return;
		if (this.selectedExom === null || this.selectedExom === DEFAULT_EXOM) {
			this.selectedExom = this.defaultExomForUser(email);
		}
	}

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
