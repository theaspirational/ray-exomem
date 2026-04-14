<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { auth } from '$lib/auth.svelte';
	import { getExomemBaseUrl } from '$lib/exomem.svelte';

	let error = $state<string | null>(null);
	let loading = $state(false);

	/* Google client ID — set PUBLIC_GOOGLE_CLIENT_ID env var when ready. */
	const GOOGLE_CLIENT_ID = '';

	onMount(() => {
		if (auth.isAuthenticated) {
			goto(`${base}/`);
			return;
		}
		if (GOOGLE_CLIENT_ID) {
			loadGSI();
		}
	});

	function loadGSI() {
		const script = document.createElement('script');
		script.src = 'https://accounts.google.com/gsi/client';
		script.async = true;
		script.onload = initializeGSI;
		document.head.appendChild(script);
	}

	function initializeGSI() {
		// @ts-ignore — google.accounts loaded via external script
		google.accounts.id.initialize({
			client_id: GOOGLE_CLIENT_ID,
			callback: handleCredentialResponse
		});
		// @ts-ignore
		google.accounts.id.renderButton(document.getElementById('google-signin-btn'), {
			theme: 'outline',
			size: 'large',
			width: 300
		});
	}

	async function handleCredentialResponse(response: { credential: string }) {
		await doLogin(response.credential, 'google');
	}

	async function doLogin(idToken: string, provider: string) {
		loading = true;
		error = null;
		try {
			const apiBase = getExomemBaseUrl().replace('/ray-exomem', '');
			const resp = await fetch(`${apiBase}/auth/login`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ id_token: idToken, provider })
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

			<!-- Google Sign-In (only rendered when client ID is configured) -->
			{#if GOOGLE_CLIENT_ID}
				<div class="flex justify-center">
					<div id="google-signin-btn"></div>
				</div>
				<div class="my-5 flex items-center gap-3">
					<div class="h-px flex-1 bg-zinc-800"></div>
					<span class="text-xs text-zinc-600">or</span>
					<div class="h-px flex-1 bg-zinc-800"></div>
				</div>
			{/if}

			<!-- Mock login for development -->
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
					class="w-full rounded-md bg-zinc-100 px-4 py-2 text-sm font-medium text-zinc-900 transition hover:bg-zinc-200 disabled:cursor-not-allowed disabled:opacity-50"
				>
					{#if loading}
						Signing in...
					{:else}
						Sign In
					{/if}
				</button>
			</form>
		</div>

		<p class="text-center text-xs text-zinc-600">
			Development mode — mock authentication
		</p>
	</div>
</div>
