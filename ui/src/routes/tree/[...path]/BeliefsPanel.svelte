<script lang="ts">
	import { browser } from '$app/environment';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import LoadingState from '$lib/components/LoadingState.svelte';
	import { fetchBeliefs, type BeliefRow } from '$lib/exomem.svelte';

	let { exomPath, branch = 'main' }: { exomPath: string; branch?: string } = $props();

	let beliefs = $state<BeliefRow[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let retry = $state(0);

	$effect(() => {
		if (!browser) return;
		exomPath;
		branch;
		retry;
		let cancelled = false;
		loading = true;
		error = null;
		fetchBeliefs(exomPath, branch)
			.then((rows) => {
				if (!cancelled) beliefs = rows;
			})
			.catch((e: unknown) => {
				if (!cancelled) error = e instanceof Error ? e.message : 'Failed to load beliefs';
			})
			.finally(() => {
				if (!cancelled) loading = false;
			});
		return () => {
			cancelled = true;
		};
	});
</script>

{#if loading}
	<LoadingState message="Loading beliefs..." />
{:else if error}
	<ErrorState
		message={error}
		onRetry={() => {
			error = null;
			retry++;
		}}
	/>
{:else if beliefs.length === 0}
	<p class="font-serif text-sm text-muted-foreground">No active beliefs on this branch.</p>
{:else}
	<div class="space-y-2">
		{#each beliefs as belief (belief.belief_id)}
			<div class="rounded-md border border-border/60 px-3 py-2">
				<div class="flex flex-wrap items-center gap-2">
					<span class="font-mono text-xs text-muted-foreground">{belief.belief_id}</span>
					<Badge variant="outline" class="h-5 px-1.5 text-[10px]">
						{Math.round(belief.confidence * 100)}%
					</Badge>
					{#if belief.branch_name}
						<Badge variant="outline" class="h-5 px-1.5 text-[10px]">
							{belief.branch_name}
						</Badge>
					{/if}
				</div>
				<p class="mt-1 font-serif text-sm leading-relaxed text-foreground">
					{belief.claim_text}
				</p>
				{#if belief.rationale}
					<p class="mt-1 text-xs leading-relaxed text-muted-foreground">{belief.rationale}</p>
				{/if}
			</div>
		{/each}
	</div>
{/if}
