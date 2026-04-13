<script lang="ts">
	import { Loader2, RefreshCw } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { fetchTree, type TreeNode } from '$lib/exomem.svelte';
	import ArchivedView from './ArchivedView.svelte';
	import ExomView from './ExomView.svelte';
	import FolderView from './FolderView.svelte';
	import SessionView from './SessionView.svelte';

	let { data } = $props();

	let node = $state<TreeNode | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

	async function loadNode() {
		loading = true;
		error = null;
		try {
			const p = data.path?.trim() || undefined;
			node = await fetchTree(p, { depth: 1, branches: true, archived: true });
		} catch (e) {
			node = null;
			error = e instanceof Error ? e.message : 'Failed to load node';
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		data.path;
		void loadNode();
	});
</script>

<div class="min-h-[60vh] p-4 sm:p-6">

	{#if loading}
		<div class="flex items-center gap-2 text-sm text-zinc-400">
			<Loader2 class="size-4 animate-spin" />
			Loading node…
		</div>
	{:else if error}
		<div
			class="flex flex-col gap-2 rounded-md border border-red-900/50 bg-red-950/30 px-3 py-2 text-sm text-red-200"
		>
			<p>{error}</p>
			<Button variant="outline" size="sm" class="w-fit" onclick={() => void loadNode()}>
				<RefreshCw class="mr-1 size-3" />
				Retry
			</Button>
		</div>
	{:else if node}
		{#if node.kind === 'folder'}
			<FolderView {node} />
		{:else if node.kind === 'exom' && node.archived}
			<ArchivedView {node} />
		{:else if node.kind === 'exom' && node.exom_kind === 'session'}
			<SessionView {node} />
		{:else if node.kind === 'exom'}
			<ExomView {node} />
		{/if}
	{:else}
		<p class="text-sm text-zinc-500">Nothing to show for this path.</p>
	{/if}
</div>
