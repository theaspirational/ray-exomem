<script lang="ts">
	import { base } from '$app/paths';
	import { goto } from '$app/navigation';
	import { Folder, Brain } from '@lucide/svelte';
	import EmptyState from '$lib/components/EmptyState.svelte';
	import { Card } from '$lib/components/ui/card/index.js';
	import type { TreeNode } from '$lib/exomem.svelte';
	import { treeExomDisplayName } from '$lib/path.svelte';

	let { node }: { node: Extract<TreeNode, { kind: 'folder' }> } = $props();

	const sortedChildren = $derived.by(() => {
		const ch = [...node.children];
		const exoms = ch.filter((c) => c.kind === 'exom');
		const folders = ch.filter((c) => c.kind === 'folder');
		const byName = (a: TreeNode, b: TreeNode) => a.name.localeCompare(b.name);
		exoms.sort(byName);
		folders.sort(byName);
		return [...exoms, ...folders];
	});

	function goChild(n: TreeNode) {
		const p = n.path.startsWith('/') ? n.path.slice(1) : n.path;
		goto(`${base}/tree/${p}`);
	}
</script>

<div class="flex flex-col gap-4">
	{#if sortedChildren.length === 0}
		<EmptyState icon={Folder} message="No children" />
	{:else}
		<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
			{#each sortedChildren as ch (ch.path)}
				<button
					type="button"
					class="text-left"
					onclick={() => goChild(ch)}
				>
					<Card
						class="border-border bg-card/50 transition-colors hover:border-primary/50 hover:bg-secondary/40"
						size="sm"
					>
						<div class="flex items-center gap-2">
							{#if ch.kind === 'folder'}
								<Folder class="mt-0.5 size-4 shrink-0 text-foreground/60" />
							{:else}
								<Brain class="mt-0.5 size-4 shrink-0 text-foreground/60" />
							{/if}
							<div class="min-w-0 flex-1">
								<p class="truncate font-medium text-foreground">
									{ch.kind === 'exom' ? treeExomDisplayName(ch) : ch.name}
								</p>
								<p class="mt-0.5 text-[11px] text-muted-foreground">
									{#if ch.kind === 'exom'}
										{ch.fact_count} facts
									{:else}
										{ch.children?.length ?? 0} children
									{/if}
								</p>
							</div>
						</div>
					</Card>
				</button>
			{/each}
		</div>
	{/if}
</div>
