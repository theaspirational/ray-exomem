<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { Toaster } from '$lib/components/ui/sonner/index.js';
	import CommandPalette from '$lib/CommandPalette.svelte';
	import { commandPaletteState } from '$lib/commandPaletteState.svelte';
	import Drawer from '$lib/Drawer.svelte';
	import TopBar from '$lib/TopBar.svelte';
	import StatusBar from '$lib/StatusBar.svelte';
	import ConnectAgentSheet from '$lib/Welcome/ConnectAgentSheet.svelte';
	import { auth } from '$lib/auth.svelte';
	import { app } from '$lib/stores.svelte';

	let { children }: { children: Snippet } = $props();

	const isLoginRoute = $derived(page.url.pathname.startsWith(`${base}/login`));
	let appStarted = $state(false);
	let redirectingToLogin = $state(false);
	let cleanups: Array<() => void> = [];

	function redirectToLogin() {
		if (redirectingToLogin || isLoginRoute) return;
		redirectingToLogin = true;
		void goto(`${base}/login`, { replaceState: true });
	}

	function startApp() {
		if (appStarted) return;
		appStarted = true;

		void app.refreshExoms();
		const connectTimer = window.setTimeout(() => app.live.connect(), 75);
		const uptimeInterval = window.setInterval(() => void app.refreshServerUptime(), 15_000);
		const onKey = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
				e.preventDefault();
				commandPaletteState.show();
			}
		};
		window.addEventListener('keydown', onKey);
		cleanups = [
			() => clearTimeout(connectTimer),
			() => clearInterval(uptimeInterval),
			() => window.removeEventListener('keydown', onKey),
			() => app.live.disconnect()
		];
	}

	function stopApp() {
		if (!appStarted) return;
		appStarted = false;
		for (const fn of cleanups) fn();
		cleanups = [];
	}

	onMount(async () => {
		await auth.checkSession();

		if (!auth.isAuthenticated && !isLoginRoute) {
			redirectToLogin();
			return;
		}

		if (auth.isAuthenticated && !isLoginRoute) {
			app.ensureAuthenticatedDefaultExom(auth.user?.email ?? null);
			startApp();
		}
	});

	$effect(() => {
		if (auth.loading) return;
		if (auth.isAuthenticated && !isLoginRoute) {
			redirectingToLogin = false;
			app.ensureAuthenticatedDefaultExom(auth.user?.email ?? null);
			startApp();
			return;
		}
		stopApp();
		app.clearSelection();
		if (isLoginRoute) {
			redirectingToLogin = false;
			return;
		}
		if (!auth.isAuthenticated) {
			redirectToLogin();
		}
	});

	$effect(() => {
		if (!auth.isAuthenticated || isLoginRoute) return;
		app.selectedExom;
		void app.refreshServerUptime();
	});

	onMount(() => {
		return () => stopApp();
	});
</script>

<svelte:head>
	<title>Ray Exomem</title>
</svelte:head>

{#if auth.loading}
	<div class="flex h-screen items-center justify-center bg-zinc-900 text-zinc-500">
		<p class="text-sm">Checking session...</p>
	</div>
{:else if isLoginRoute}
	{@render children()}
	<Toaster richColors position="bottom-right" />
{:else if auth.isAuthenticated}
	<div class="flex h-screen flex-col overflow-hidden bg-zinc-900 font-sans text-zinc-100">
		<div class="flex min-h-0 flex-1">
			<Drawer />
			<div class="flex min-h-0 min-w-0 flex-1 flex-col">
				<TopBar />
				<main class="flex min-h-0 flex-1 flex-col overflow-y-auto">
					{@render children()}
				</main>
			</div>
		</div>
		<StatusBar />
		<ConnectAgentSheet />
		<CommandPalette bind:open={commandPaletteState.open} />
			<Toaster richColors position="bottom-right" />
	</div>
{:else}
	<div class="flex h-screen items-center justify-center bg-zinc-900 text-zinc-500">
		<p class="text-sm">Redirecting to sign in...</p>
	</div>
{/if}
