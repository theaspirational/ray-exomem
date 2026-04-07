<script lang="ts">
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { ArrowLeft, GitMerge, Loader2 } from '@lucide/svelte';

	import MergeDialog from '$lib/MergeDialog.svelte';
	import { Button } from '$lib/components/ui/button';
	import { fetchBranchDiff, type BranchDiffResult } from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';

	const branchId = $derived(decodeURIComponent(page.params.id ?? ''));

	let diff = $state<BranchDiffResult | null>(null);
	let loading = $state(true);
	let errorMessage = $state<string | null>(null);
	let baseBranch = $state('main');
	let mergeOpen = $state(false);

	async function load() {
		if (!branchId) return;
		loading = true;
		errorMessage = null;
		try {
			diff = await fetchBranchDiff(branchId, baseBranch, app.selectedExom);
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : String(e);
			diff = null;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		branchId;
		baseBranch;
		app.selectedExom;
		void load();
	});
</script>

<div class="mx-auto max-w-4xl space-y-6 p-6">
	<p class="text-sm">
		<a href={`${base}/branches`} class="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground">
			<ArrowLeft class="size-3.5" />
			All branches
		</a>
	</p>

	<div class="flex flex-wrap items-start justify-between gap-4">
		<div>
			<h1 class="font-mono text-xl font-semibold tracking-tight">{branchId}</h1>
			<p class="mt-1 text-sm text-muted-foreground">Diff vs base branch (read-only comparison).</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			<label class="flex items-center gap-2 text-sm">
				<span class="text-muted-foreground">Base</span>
				<input
					bind:value={baseBranch}
					class="rounded-md border border-input bg-background px-2 py-1 font-mono text-xs"
				/>
			</label>
			<Button variant="default" size="sm" class="gap-1.5" onclick={() => (mergeOpen = true)}>
				<GitMerge class="size-3.5" />
				Merge to current
			</Button>
		</div>
	</div>

	{#if errorMessage}
		<p class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">{errorMessage}</p>
	{/if}

	{#if loading}
		<div class="flex items-center gap-2 text-sm text-muted-foreground">
			<Loader2 class="size-4 animate-spin" />
			Loading diff…
		</div>
	{:else if diff}
		<div class="grid gap-4 md:grid-cols-3">
			<section class="rounded-lg border border-emerald-500/25 bg-emerald-500/5 p-4">
				<h2 class="text-sm font-medium text-emerald-700 dark:text-emerald-400">Added ({diff.added.length})</h2>
				<ul class="mt-2 max-h-64 space-y-1 overflow-y-auto font-mono text-xs">
					{#each diff.added as row}
						<li class="text-foreground/90">{JSON.stringify(row)}</li>
					{/each}
				</ul>
			</section>
			<section class="rounded-lg border border-rose-500/25 bg-rose-500/5 p-4">
				<h2 class="text-sm font-medium text-rose-700 dark:text-rose-400">Removed ({diff.removed.length})</h2>
				<ul class="mt-2 max-h-64 space-y-1 overflow-y-auto font-mono text-xs">
					{#each diff.removed as row}
						<li class="text-foreground/90">{JSON.stringify(row)}</li>
					{/each}
				</ul>
			</section>
			<section class="rounded-lg border border-amber-500/25 bg-amber-500/5 p-4 md:col-span-3">
				<h2 class="text-sm font-medium text-amber-800 dark:text-amber-300">Changed ({diff.changed.length})</h2>
				<ul class="mt-2 max-h-64 space-y-1 overflow-y-auto font-mono text-xs">
					{#each diff.changed as row}
						<li class="text-foreground/90">{JSON.stringify(row)}</li>
					{/each}
				</ul>
			</section>
		</div>
	{/if}
</div>

{#if mergeOpen}
	<MergeDialog
		sourceBranch={branchId}
		exom={app.selectedExom}
		onClose={() => {
			mergeOpen = false;
			void load();
		}}
	/>
{/if}
