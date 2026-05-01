<script lang="ts">
	import { browser } from '$app/environment';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import LoadingState from '$lib/components/LoadingState.svelte';
	import { fetchObservations, type ObservationRow } from '$lib/exomem.svelte';

	let { exomPath }: { exomPath: string } = $props();

	let observations = $state<ObservationRow[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let retry = $state(0);

	$effect(() => {
		if (!browser) return;
		exomPath;
		retry;
		let cancelled = false;
		loading = true;
		error = null;
		fetchObservations(exomPath)
			.then((rows) => {
				if (!cancelled) observations = rows;
			})
			.catch((e: unknown) => {
				if (!cancelled) error = e instanceof Error ? e.message : 'Failed to load observations';
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
	<LoadingState message="Loading observations..." />
{:else if error}
	<ErrorState
		message={error}
		onRetry={() => {
			error = null;
			retry++;
		}}
	/>
{:else if observations.length === 0}
	<p class="font-serif text-sm text-muted-foreground">No observations recorded for this exom.</p>
{:else}
	<div class="space-y-2">
		{#each observations as obs (obs.obs_id)}
			<div class="rounded-md border border-border/60 px-3 py-2">
				<div class="flex flex-wrap items-center gap-2">
					<span class="font-mono text-xs text-muted-foreground">{obs.obs_id}</span>
					{#if obs.source_type}
						<Badge variant="outline" class="h-5 px-1.5 text-[10px]">{obs.source_type}</Badge>
					{/if}
					{#if obs.branch_name}
						<Badge variant="outline" class="h-5 px-1.5 text-[10px]">
							origin {obs.branch_name}
						</Badge>
					{/if}
				</div>
				<p class="mt-1 text-sm leading-relaxed text-foreground">{obs.content}</p>
				{#if obs.tags?.length}
					<div class="mt-2 flex flex-wrap gap-1">
						{#each obs.tags as tag (tag)}
							<Badge variant="outline" class="h-5 px-1.5 text-[10px]">{tag}</Badge>
						{/each}
					</div>
				{/if}
			</div>
		{/each}
	</div>
{/if}
