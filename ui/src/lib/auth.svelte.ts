import { browser } from '$app/environment';
import { goto } from '$app/navigation';
import { getExomemBaseUrl } from '$lib/exomem.svelte';

export interface AuthUser {
	email: string;
	display_name: string;
	provider: string;
	role: string;
}

class AuthState {
	user = $state<AuthUser | null>(null);
	loading = $state(true);
	error = $state<string | null>(null);

	get isAuthenticated() {
		return this.user !== null;
	}

	get isAdmin() {
		return this.user?.role === 'admin' || this.user?.role === 'top-admin';
	}

	get isTopAdmin() {
		return this.user?.role === 'top-admin';
	}

	async checkSession() {
		if (!browser) return;
		this.loading = true;
		try {
			const base = getExomemBaseUrl().replace('/ray-exomem', '');
			const resp = await fetch(`${base}/auth/me`, { credentials: 'include' });
			if (resp.ok) {
				this.user = await resp.json();
			} else {
				this.user = null;
			}
		} catch {
			this.user = null;
		} finally {
			this.loading = false;
		}
	}

	async logout() {
		const base = getExomemBaseUrl().replace('/ray-exomem', '');
		await fetch(`${base}/auth/logout`, {
			method: 'POST',
			credentials: 'include'
		});
		this.user = null;
		goto('/login');
	}
}

export const auth = new AuthState();
