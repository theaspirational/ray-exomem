<script lang="ts">
	import { Loader2, RefreshCw } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { fetchTree, type TreeNode } from '$lib/exomem.svelte';

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

<div class="flex flex-col gap-4 p-4 sm:p-6">
	<div class="font-mono text-[11px] text-zinc-500">
		<span class="text-zinc-400">Path</span>
		<span class="ml-2 text-zinc-200">{data.path || '/'}</span>
	</div>

	<p class="text-sm text-zinc-400">Focus view coming in Phase 9</p>

	{#if loading}
		<div class="flex items-center gap-2 text-sm text-zinc-400">
			<Loader2 class="size-4 animate-spin" />
			Loading node…
		</div>
	{:else if error}
		<div class="flex flex-col gap-2 rounded-md border border-red-900/50 bg-red-950/30 px-3 py-2 text-sm text-red-200">
			<p>{error}</p>
			<Button variant="outline" size="sm" class="w-fit" onclick={() => void loadNode()}>
				<RefreshCw class="mr-1 size-3" />
				Retry
			</Button>
		</div>
	{:else if node}
		<div class="rounded-lg border border-zinc-700 bg-zinc-900/50 px-4 py-3 font-mono text-xs text-zinc-300">
			<p class="text-[0.65rem] uppercase tracking-wide text-zinc-500">Kind</p>
			<p class="mt-1 text-sm text-zinc-100">{node.kind}</p>

			{#if node.kind === 'exom'}
				<div class="mt-3 grid gap-2 sm:grid-cols-2">
					<div>
						<p class="text-[0.65rem] uppercase tracking-wide text-zinc-500">exom_kind</p>
						<p class="text-zinc-200">{node.exom_kind}</p>
					</div>
					<div>
						<p class="text-[0.65rem] uppercase tracking-wide text-zinc-500">fact_count</p>
						<p class="text-zinc-200">{node.fact_count}</p>
					</div>
					<div>
						<p class="text-[0.65rem] uppercase tracking-wide text-zinc-500">current_branch</p>
						<p class="text-zinc-200">{node.current_branch}</p>
					</div>
					<div>
						<p class="text-[0.65rem] uppercase tracking-wide text-zinc-500">last_tx</p>
						<p class="text-zinc-200">{node.last_tx ?? '—'}</p>
					</div>
					<div>
						<p class="text-[0.65rem] uppercase tracking-wide text-zinc-500">archived / closed</p>
						<p class="text-zinc-200">{node.archived} / {node.closed}</p>
					</div>
					{#if node.branches}
						<div class="sm:col-span-2">
							<p class="text-[0.65rem] uppercase tracking-wide text-zinc-500">branches</p>
							<p class="break-all text-zinc-200">{node.branches.join(', ') || '—'}</p>
						</div>
					{/if}
				</div>
			{:else}
				<p class="mt-2 text-zinc-400">
					Folder · {node.children.length} entr{node.children.length === 1 ? 'y' : 'ies'}
				</p>
			{/if}
		</div>
	{/if}
</div>
