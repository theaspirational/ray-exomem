<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { Separator } from '$lib/components/ui/separator/index.js';
	import { Toaster } from '$lib/components/ui/sonner/index.js';
	import ActorIdentityDialog from '$lib/ActorIdentityDialog.svelte';
	import CommandPalette from '$lib/CommandPalette.svelte';
	import { commandPaletteState } from '$lib/commandPaletteState.svelte';
	import Drawer from '$lib/Drawer.svelte';
	import TopBar from '$lib/TopBar.svelte';
	import StatusBar from '$lib/StatusBar.svelte';
	import { auth } from '$lib/auth.svelte';
	import { app } from '$lib/stores.svelte';

	let { children }: { children: Snippet } = $props();

	const isLoginRoute = $derived(page.url.pathname.startsWith(`${base}/login`));

	onMount(async () => {
		await auth.checkSession();

		if (!auth.isAuthenticated && !isLoginRoute) {
			goto(`${base}/login`);
			return;
		}

		if (auth.isAuthenticated) {
			startApp();
		}
	});

	/* Re-check: if auth flips to authenticated later (e.g. after login redirect back). */
	$effect(() => {
		if (auth.isAuthenticated && !isLoginRoute) {
			startApp();
		}
	});

	let appStarted = $state(false);
	let cleanups: Array<() => void> = [];

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

	$effect(() => {
		app.selectedExom;
		void app.refreshServerUptime();
	});

	onMount(() => {
		return () => {
			for (const fn of cleanups) fn();
		};
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
{:else}
	<div class="flex h-screen flex-col overflow-hidden bg-zinc-900 font-sans text-zinc-100">
		<div class="flex min-h-0 flex-1">
			<Drawer />
			<Separator orientation="vertical" class="bg-zinc-700" />
			<div class="flex min-h-0 min-w-0 flex-1 flex-col">
				<TopBar />
				<main class="flex min-h-0 flex-1 flex-col overflow-y-auto">
					{@render children()}
				</main>
			</div>
		</div>
		<StatusBar />
		<CommandPalette bind:open={commandPaletteState.open} />
		<ActorIdentityDialog />
		<Toaster richColors position="bottom-right" />
	</div>
{/if}
