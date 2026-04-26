<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { auth } from '$lib/auth.svelte';
	import { getExomemBaseUrl } from '$lib/exomem.svelte';

	type AuthErrorResponse = {
		code?: string;
		message?: string;
		suggestion?: string;
	};

	type GsiCredentialResponse = {
		credential: string;
	};

	type GsiSharedState = {
		scriptPromise: Promise<void> | null;
		initializedClientId: string | null;
		onCredential: ((response: GsiCredentialResponse) => void | Promise<void>) | null;
	};

	const DEFAULT_LOGIN_ERROR = 'Login failed';

	let error = $state<string | null>(null);
	let loading = $state(false);

	/** Auth provider info fetched from the server. */
	let provider = $state<'google' | 'mock' | null>(null);
	let googleClientId = $state<string | null>(null);
	let infoLoaded = $state(false);
	let infoError = $state(false);
	let gsiReady = $state(false);

	onMount(() => {
		if (auth.isAuthenticated) {
			goto(`${base}/`);
			return;
		}
		void fetchAuthInfo();
		return () => {
			gsiSharedState().onCredential = null;
		};
	});

	async function fetchAuthInfo() {
		try {
			const apiBase = getExomemBaseUrl().replace('/ray-exomem', '');
			const resp = await fetch(`${apiBase}/auth/info`, { credentials: 'include' });
			if (resp.ok) {
				const data: { provider: 'google' | 'mock' | null; google_client_id: string | null } =
					await resp.json();
				provider = data.provider;
				googleClientId = data.google_client_id;
			} else {
				infoError = true;
			}
		} catch {
			infoError = true;
		} finally {
			infoLoaded = true;
			// Load GSI after DOM updates with the button container
			if (googleClientId) {
				requestAnimationFrame(() => {
					void initGSI();
				});
			}
		}
	}

	function gsiSharedState(): GsiSharedState {
		const root = globalThis as typeof globalThis & { __rayExomemGsi?: GsiSharedState };
		root.__rayExomemGsi ??= {
			scriptPromise: null,
			initializedClientId: null,
			onCredential: null
		};
		return root.__rayExomemGsi;
	}

	function googleAccounts() {
		const root = globalThis as typeof globalThis & {
			google?: {
				accounts?: {
					id?: {
						initialize: (config: Record<string, unknown>) => void;
						renderButton: (element: HTMLElement, options: Record<string, unknown>) => void;
					};
				};
			};
		};
		return root.google?.accounts?.id ?? null;
	}

	async function ensureGsiLoaded(): Promise<void> {
		if (googleAccounts()) return;
		const shared = gsiSharedState();
		if (shared.scriptPromise) {
			await shared.scriptPromise;
			return;
		}
		shared.scriptPromise = new Promise<void>((resolve, reject) => {
			const existing = document.querySelector<HTMLScriptElement>(
				'script[src="https://accounts.google.com/gsi/client"]'
			);
			if (existing) {
				if (googleAccounts()) {
					resolve();
					return;
				}
				existing.addEventListener('load', () => resolve(), { once: true });
				existing.addEventListener('error', () => reject(new Error('Failed to load Google Sign-In')), {
					once: true
				});
				return;
			}
			const script = document.createElement('script');
			script.src = 'https://accounts.google.com/gsi/client';
			script.async = true;
			script.onload = () => resolve();
			script.onerror = () => reject(new Error('Failed to load Google Sign-In'));
			document.head.appendChild(script);
		});
		try {
			await shared.scriptPromise;
		} catch (err) {
			shared.scriptPromise = null;
			throw err;
		}
	}

	function configureGSI() {
		if (!googleClientId) return;
		const api = googleAccounts();
		if (!api) return;
		const shared = gsiSharedState();
		shared.onCredential = handleCredentialResponse;
		if (shared.initializedClientId === googleClientId) {
			return;
		}
		api.initialize({
			client_id: googleClientId,
			callback: (response: GsiCredentialResponse) => {
				void shared.onCredential?.(response);
			},
			auto_select: false,
			use_fedcm_for_prompt: false
		});
		shared.initializedClientId = googleClientId;
	}

	function renderGSIButton() {
		const el = document.getElementById('google-signin-btn');
		const api = googleAccounts();
		if (!el || !api) return;
		el.replaceChildren();
		api.renderButton(el, {
			theme: 'outline',
			size: 'large',
			width: 340,
			shape: 'rectangular',
			text: 'signin_with'
		});
		gsiReady = true;
	}

	async function initGSI() {
		if (!googleClientId) return;
		try {
			await ensureGsiLoaded();
			configureGSI();
			renderGSIButton();
		} catch (e) {
			infoError = true;
			error = e instanceof Error ? e.message : 'Failed to load Google Sign-In';
		}
	}

	async function handleCredentialResponse(response: GsiCredentialResponse) {
		await doLogin(response.credential, 'google');
	}

	function formatLoginError(body: AuthErrorResponse, status: number): string {
		const parts = [body.message, body.suggestion].filter(Boolean);
		return parts.join(' ') || `${DEFAULT_LOGIN_ERROR} (${status})`;
	}

	async function doLogin(idToken: string, loginProvider: string) {
		loading = true;
		error = null;
		try {
			const apiBase = getExomemBaseUrl().replace('/ray-exomem', '');
			const resp = await fetch(`${apiBase}/auth/login`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ id_token: idToken, provider: loginProvider })
			});
			if (!resp.ok) {
				const body = (await resp.json().catch(() => ({}))) as AuthErrorResponse;
				error = formatLoginError(body, resp.status);
				return;
			}
			await auth.checkSession();
			goto(`${base}/`);
		} catch (e) {
			error = e instanceof Error ? e.message : 'Login failed';
		} finally {
			loading = false;
		}
	}

	/* Mock login for development / testing. */
	let mockEmail = $state('');
	let mockName = $state('');

	async function mockLogin() {
		if (!mockEmail || !mockName) return;
		await doLogin(`mock:${mockEmail}:${mockName}`, 'mock');
	}
</script>

<svelte:head>
	<title>Sign In - Ray Exomem</title>
</svelte:head>

<div class="flex min-h-screen items-center justify-center bg-background px-4 text-foreground">
	<div class="w-full max-w-sm space-y-8">
		<div class="text-center">
			<h1 class="font-serif text-2xl font-medium tracking-tight">Ray Exomem</h1>
			<p class="mt-2 font-sans text-sm text-muted-foreground">persistent memory for your agents</p>
		</div>

		<div
			class="rounded-lg border border-border bg-card p-6 shadow-[0_0_0_1px_color-mix(in_oklch,_white_4%,_transparent)]"
		>
			{#if error}
				<div class="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
					{error}
				</div>
			{/if}

			{#if !infoLoaded}
				<div class="flex items-center justify-center py-6">
					<div class="h-5 w-5 animate-spin rounded-full border-2 border-border border-t-primary"></div>
					<span class="ml-3 text-sm text-muted-foreground">Loading...</span>
				</div>
			{:else if infoError}
				<p class="py-4 text-center text-sm text-muted-foreground">
					Could not reach authentication service.
				</p>
			{:else if provider === null}
				<p class="py-4 text-center text-sm text-muted-foreground">
					Authentication not configured
				</p>
			{:else}
				{#if googleClientId}
					<div class="flex justify-center">
						<div id="google-signin-btn"></div>
					</div>
					{#if !gsiReady}
						<div class="flex items-center justify-center py-2">
							<div class="h-4 w-4 animate-spin rounded-full border-2 border-border border-t-primary"></div>
							<span class="ml-2 text-xs text-muted-foreground">Loading Google Sign-In...</span>
						</div>
					{/if}
				{/if}

				{#if provider === 'mock'}
					{#if googleClientId}
						<div class="my-5 flex items-center gap-3">
							<div class="h-px flex-1 bg-border"></div>
							<span class="text-xs text-muted-foreground">or</span>
							<div class="h-px flex-1 bg-border"></div>
						</div>
					{/if}

					<div class="space-y-3">
						<p class="text-center text-xs font-medium uppercase tracking-wider text-muted-foreground">
							Development mode
						</p>
						<form onsubmit={(e) => { e.preventDefault(); mockLogin(); }} class="space-y-3">
							<div>
								<label for="mock-email" class="block text-xs font-medium text-muted-foreground"
									>Email</label
								>
								<input
									id="mock-email"
									type="email"
									bind:value={mockEmail}
									placeholder="you@example.com"
									required
									class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-primary"
								/>
							</div>
							<div>
								<label for="mock-name" class="block text-xs font-medium text-muted-foreground"
									>Display Name</label
								>
								<input
									id="mock-name"
									type="text"
									bind:value={mockName}
									placeholder="Your Name"
									required
									class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-primary"
								/>
							</div>
							<button
								type="submit"
								disabled={loading || !mockEmail || !mockName}
								class="w-full rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
							>
								{#if loading}
									Signing in...
								{:else}
									Sign In (Dev)
								{/if}
							</button>
						</form>
					</div>
				{/if}
			{/if}
		</div>
	</div>
</div>
