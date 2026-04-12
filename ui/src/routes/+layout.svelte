<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import { Separator } from '$lib/components/ui/separator/index.js';
	import { Toaster } from '$lib/components/ui/sonner/index.js';
	import Drawer from '$lib/Drawer.svelte';
	import TopBar from '$lib/TopBar.svelte';
	import StatusBar from '$lib/StatusBar.svelte';
	import { app } from '$lib/stores.svelte';

	let { children }: { children: Snippet } = $props();

	onMount(() => {
		void app.refreshExoms();
		const connectTimer = window.setTimeout(() => app.live.connect(), 75);
		const uptimeInterval = window.setInterval(() => void app.refreshServerUptime(), 15_000);
		return () => {
			clearTimeout(connectTimer);
			clearInterval(uptimeInterval);
			app.live.disconnect();
		};
	});

	$effect(() => {
		app.selectedExom;
		void app.refreshServerUptime();
	});
</script>

<svelte:head>
	<title>Ray Exomem</title>
</svelte:head>

<div class="dark flex h-screen flex-col overflow-hidden bg-zinc-900 font-sans text-zinc-100">
	<div class="flex min-h-0 flex-1">
		<Drawer />
		<Separator orientation="vertical" class="bg-zinc-700" />
		<div class="flex min-h-0 min-w-0 flex-1 flex-col">
			<TopBar />
			<main class="min-h-0 flex-1 overflow-y-auto">
				{@render children()}
			</main>
		</div>
	</div>
	<StatusBar />
	<Toaster richColors position="bottom-right" />
</div>
