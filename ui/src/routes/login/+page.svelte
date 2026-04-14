<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { auth } from '$lib/auth.svelte';
	import { getExomemBaseUrl } from '$lib/exomem.svelte';

	let error = $state<string | null>(null);
	let loading = $state(false);

	/** Auth provider info fetched from the server. */
	let provider = $state<'google' | 'mock' | null>(null);
	let googleClientId = $state<string | null>(null);
	let infoLoaded = $state(false);
	let infoError = $state(false);

	onMount(() => {
		if (auth.isAuthenticated) {
			goto(`${base}/`);
			return;
		}
		fetchAuthInfo();
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
		}
	}

	function handleGoogleSignIn() {
		if (!googleClientId) return;
		loadGSI();
	}

	function loadGSI() {
		const existing = document.querySelector('script[src*="accounts.google.com/gsi/client"]');
		if (existing) {
			initializeGSI();
			return;
		}
		const script = document.createElement('script');
		script.src = 'https://accounts.google.com/gsi/client';
		script.async = true;
		script.onload = initializeGSI;
		document.head.appendChild(script);
	}

	function initializeGSI() {
		// @ts-ignore -- google.accounts loaded via external script
		google.accounts.id.initialize({
			client_id: googleClientId,
			callback: handleCredentialResponse
		});
		// @ts-ignore
		google.accounts.id.prompt();
	}

	async function handleCredentialResponse(response: { credential: string }) {
		await doLogin(response.credential, 'google');
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
				const body = await resp.json().catch(() => ({}));
				error = body.message || 'Login failed';
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

<div class="flex min-h-screen items-center justify-center bg-zinc-900 px-4">
	<div class="w-full max-w-sm space-y-8">
		<!-- Branding -->
		<div class="text-center">
			<h1 class="text-2xl font-semibold tracking-tight text-zinc-100">Ray Exomem</h1>
			<p class="mt-1 text-sm text-zinc-500">Sign in to continue</p>
		</div>

		<!-- Card -->
		<div class="rounded-lg border border-zinc-800 bg-zinc-900/80 p-6 shadow-lg">
			{#if error}
				<div class="mb-4 rounded-md bg-red-950/40 px-3 py-2 text-sm text-red-400">
					{error}
				</div>
			{/if}

			{#if !infoLoaded}
				<!-- Loading state while fetching auth info -->
				<div class="flex items-center justify-center py-6">
					<div class="h-5 w-5 animate-spin rounded-full border-2 border-zinc-600 border-t-zinc-300"></div>
					<span class="ml-3 text-sm text-zinc-500">Loading...</span>
				</div>
			{:else if infoError}
				<p class="py-4 text-center text-sm text-zinc-500">
					Could not reach authentication service.
				</p>
			{:else if provider === null}
				<!-- Auth not configured -->
				<p class="py-4 text-center text-sm text-zinc-500">
					Authentication not configured
				</p>
			{:else}
				<!-- Google Sign-In button (primary action) -->
				{#if googleClientId}
					<button
						type="button"
						disabled={loading}
						onclick={handleGoogleSignIn}
						class="flex w-full items-center justify-center gap-3 rounded-md border border-zinc-300 bg-white px-4 py-2.5 text-sm font-medium text-zinc-800 transition hover:bg-zinc-100 disabled:cursor-not-allowed disabled:opacity-50"
					>
						<!-- Google "G" logo SVG -->
						<svg width="18" height="18" viewBox="0 0 48 48" aria-hidden="true">
							<path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"/>
							<path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"/>
							<path fill="#34A853" d="M10.53 28.59a14.5 14.5 0 0 1 0-9.18l-7.98-6.19a24.0 24.0 0 0 0 0 21.56l7.98-6.19z"/>
							<path fill="#FBBC05" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"/>
						</svg>
						{#if loading}
							Signing in...
						{:else}
							Sign in with Google
						{/if}
					</button>
				{/if}

				<!-- Mock login for development mode -->
				{#if provider === 'mock'}
					{#if googleClientId}
						<div class="my-5 flex items-center gap-3">
							<div class="h-px flex-1 bg-zinc-800"></div>
							<span class="text-xs text-zinc-600">or</span>
							<div class="h-px flex-1 bg-zinc-800"></div>
						</div>
					{/if}

					<div class="space-y-3">
						<p class="text-center text-xs font-medium uppercase tracking-wider text-zinc-600">
							Development mode
						</p>
						<form onsubmit={(e) => { e.preventDefault(); mockLogin(); }} class="space-y-3">
							<div>
								<label for="mock-email" class="block text-xs font-medium text-zinc-400">Email</label>
								<input
									id="mock-email"
									type="email"
									bind:value={mockEmail}
									placeholder="you@example.com"
									required
									class="mt-1 w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-600 focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500"
								/>
							</div>
							<div>
								<label for="mock-name" class="block text-xs font-medium text-zinc-400">Display Name</label>
								<input
									id="mock-name"
									type="text"
									bind:value={mockName}
									placeholder="Your Name"
									required
									class="mt-1 w-full rounded-md border border-zinc-700 bg-zinc-800 px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-600 focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500"
								/>
							</div>
							<button
								type="submit"
								disabled={loading || !mockEmail || !mockName}
								class="w-full rounded-md bg-zinc-700 px-4 py-2 text-sm font-medium text-zinc-200 transition hover:bg-zinc-600 disabled:cursor-not-allowed disabled:opacity-50"
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
