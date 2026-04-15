import { browser } from '$app/environment';
import { goto } from '$app/navigation';
import { base } from '$app/paths';
import { getExomemBaseUrl } from '$lib/exomem-base';

export interface AuthUser {
	email: string;
	display_name: string;
	provider: string;
	role: string;
}

interface AuthSessionResponse {
	authenticated: boolean;
	user?: AuthUser;
}

function authApiBase(): string {
	return getExomemBaseUrl().replace('/ray-exomem', '');
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
			const resp = await fetch(`${authApiBase()}/auth/session`, { credentials: 'include' });
			if (resp.ok) {
				const session = (await resp.json()) as AuthSessionResponse;
				this.user = session.authenticated ? (session.user ?? null) : null;
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
		await fetch(`${authApiBase()}/auth/logout`, {
			method: 'POST',
			credentials: 'include'
		});
		this.user = null;
		goto(`${base}/login`);
	}
}

export const auth = new AuthState();
