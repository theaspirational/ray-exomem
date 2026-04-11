<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { ArrowLeft, ArrowRightSquare, Check, Copy, GitMerge, Loader2, Route } from '@lucide/svelte';

	import MergeDialog from '$lib/MergeDialog.svelte';
	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import {
		fetchBranchDiff,
		fetchBranches,
		fetchBranchRows,
		fetchMergeRows,
		type BranchDiffResult,
		type BranchRow,
		type BranchViewRow,
		type MergeViewRow
	} from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';

	const branchId = $derived(decodeURIComponent(page.params.id ?? ''));

	let branch = $state<BranchRow | null>(null);
	let branchView = $state<BranchViewRow | null>(null);
	let merges = $state<MergeViewRow[]>([]);
	let branches = $state<BranchRow[]>([]);
	let diff = $state<BranchDiffResult | null>(null);
	let loading = $state(true);
	let errorMessage = $state<string | null>(null);
	let baseBranch = $state('main');
	let mergeOpen = $state(false);
	let copiedSnippet = $state<string | null>(null);
	let copyTimer: ReturnType<typeof setTimeout> | null = null;

	function diffQuery(): string {
		return `;; compare ${branchId} to ${baseBranch}\n(query ${app.selectedExom} (find ?branch ?id ?name ?archived ?createdTx) (where (branch-row ?branch ?id ?name ?archived ?createdTx)))`;
	}

	function mergeHistoryQuery(): string {
		return `(query ${app.selectedExom} (find ?tx ?source ?target ?actor ?when) (where (merge-row ?tx ?source ?target ?actor ?when)))`;
	}

	async function copySnippet(key: string, text: string) {
		if (!browser || !navigator.clipboard) return;
		await navigator.clipboard.writeText(text);
		copiedSnippet = key;
		if (copyTimer) clearTimeout(copyTimer);
		copyTimer = setTimeout(() => {
			copiedSnippet = null;
		}, 1600);
	}

	async function openInQuery(text: string) {
		await goto(`${base}/query?draft=${encodeURIComponent(text)}`);
	}

	function formatCell(row: Record<string, unknown>): string {
		return JSON.stringify(row, null, 0);
	}

	async function load() {
		if (!branchId) return;
		loading = true;
		errorMessage = null;
		try {
			const [diffRes, branchList, branchRows, mergeRows] = await Promise.all([
				fetchBranchDiff(branchId, baseBranch, app.selectedExom),
				fetchBranches(app.selectedExom),
				fetchBranchRows(app.selectedExom),
				fetchMergeRows(app.selectedExom)
			]);
			diff = diffRes;
			branches = branchList;
			branch = branchList.find((item) => item.branch_id === branchId) ?? null;
			branchView = branchRows.find((item) => item.branch_id === branchId) ?? null;
			merges = mergeRows.filter((row) => row.source === branchId || row.target === branchId);
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : String(e);
			diff = null;
			branch = null;
			branchView = null;
			merges = [];
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (!browser) return;
		branchId;
		baseBranch;
		app.selectedExom;
		void load();
		return () => {
			if (copyTimer) clearTimeout(copyTimer);
		};
	});
</script>

<div class="mx-auto max-w-6xl space-y-6 p-6">
	<p class="text-sm">
		<a href={`${base}/branches`} class="inline-flex items-center gap-1 text-muted-foreground hover:text-foreground">
			<ArrowLeft class="size-3.5" />
			All branches
		</a>
	</p>

	<div class="flex flex-wrap items-start justify-between gap-4">
		<div>
			<div class="flex flex-wrap items-center gap-2">
				<h1 class="font-mono text-xl font-semibold tracking-tight">{branchId}</h1>
				{#if branch?.is_current}
					<Badge variant="secondary" class="h-5 px-2 text-[0.65rem]">current</Badge>
				{/if}
				{#if branch?.archived}
					<Badge variant="outline" class="h-5 px-2 text-[0.65rem]">archived</Badge>
				{/if}
			</div>
			<p class="mt-1 text-sm text-muted-foreground">Diff, merge history, and branch-row metadata for this branch.</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			<label class="flex items-center gap-2 text-sm">
				<span class="text-muted-foreground">Base</span>
				<select bind:value={baseBranch} class="rounded-md border border-input bg-background px-2 py-1 font-mono text-xs">
					{#each branches as item (item.branch_id)}
						<option value={item.branch_id}>{item.branch_id}</option>
					{/each}
				</select>
			</label>
			<Button variant="default" size="sm" class="gap-1.5" onclick={() => (mergeOpen = true)}>
				<GitMerge class="size-3.5" />
				Merge to current
			</Button>
		</div>
	</div>

	<div class="grid gap-3 xl:grid-cols-4">
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Branch name</p>
			<p class="mt-1 font-mono text-lg font-semibold">{branchView?.name ?? branch?.name ?? branchId}</p>
			<p class="mt-1 text-xs text-muted-foreground">Display label from branch metadata.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Parent</p>
			<p class="mt-1 font-mono text-lg font-semibold">{branch?.parent_branch_id ?? '—'}</p>
			<p class="mt-1 text-xs text-muted-foreground">Base branch for speculative history.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Created tx</p>
			<p class="mt-1 font-mono text-lg font-semibold">{branchView?.created_tx ?? (branch ? `tx/${branch.created_tx_id}` : '—')}</p>
			<p class="mt-1 text-xs text-muted-foreground">Transaction entity from <span class="font-mono">branch-row</span>.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Merge events</p>
			<p class="mt-1 text-lg font-semibold">{merges.length}</p>
			<p class="mt-1 text-xs text-muted-foreground">Rows involving this branch in <span class="font-mono">merge-row</span>.</p>
		</div>
	</div>

	<div class="grid gap-4 xl:grid-cols-[0.95fr_1.05fr]">
		<div class="rounded-lg border border-border/60 bg-card p-4">
			<div class="flex items-center justify-between gap-2">
				<div>
					<h2 class="text-sm font-medium text-muted-foreground">Query hooks</h2>
					<p class="mt-1 text-sm text-foreground">Inspect the branch metadata and merge history in the query console.</p>
				</div>
				<Route class="size-4 text-muted-foreground" />
			</div>
			<div class="mt-3 grid gap-2">
				<div class="rounded-md border border-border/50 bg-muted/20 px-3 py-3">
					<div class="flex items-start justify-between gap-3">
						<div>
							<span class="font-mono text-sm text-fact-derived">branch-row</span>
							<p class="mt-1 text-xs text-muted-foreground">Full branch metadata view across the exom.</p>
						</div>
						<div class="flex items-center gap-1">
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => copySnippet('diff-query', diffQuery())}>
								{#if copiedSnippet === 'diff-query'}
									<Check class="mr-1 size-3.5" />
									Copied
								{:else}
									<Copy class="mr-1 size-3.5" />
									Copy
								{/if}
							</Button>
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => openInQuery(diffQuery())}>
								<ArrowRightSquare class="mr-1 size-3.5" />
								Open
							</Button>
						</div>
					</div>
				</div>
				<div class="rounded-md border border-border/50 bg-muted/20 px-3 py-3">
					<div class="flex items-start justify-between gap-3">
						<div>
							<span class="font-mono text-sm text-fact-derived">merge-row</span>
							<p class="mt-1 text-xs text-muted-foreground">Merge history across source and target branches.</p>
						</div>
						<div class="flex items-center gap-1">
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => copySnippet('merge-query', mergeHistoryQuery())}>
								{#if copiedSnippet === 'merge-query'}
									<Check class="mr-1 size-3.5" />
									Copied
								{:else}
									<Copy class="mr-1 size-3.5" />
									Copy
								{/if}
							</Button>
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => openInQuery(mergeHistoryQuery())}>
								<ArrowRightSquare class="mr-1 size-3.5" />
								Open
							</Button>
						</div>
					</div>
				</div>
			</div>
		</div>

		<div class="rounded-lg border border-border/60 bg-card p-4">
			<h2 class="text-sm font-medium text-muted-foreground">Merge history</h2>
			{#if merges.length === 0}
				<p class="mt-3 text-sm text-muted-foreground">No merge rows reference this branch yet.</p>
			{:else}
				<div class="mt-3 space-y-2">
					{#each merges as row (row.tx + row.source + row.target)}
						<div class="rounded-md border border-border/50 bg-muted/20 px-3 py-2.5">
							<div class="flex flex-wrap items-center gap-2">
								<span class="font-mono text-xs text-foreground">{row.tx}</span>
								<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{row.source} → {row.target}</Badge>
							</div>
							<p class="mt-1 text-xs text-muted-foreground">actor {row.actor} at {row.when}</p>
						</div>
					{/each}
				</div>
			{/if}
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
						<li class="text-foreground/90">{formatCell(row)}</li>
					{/each}
				</ul>
			</section>
			<section class="rounded-lg border border-rose-500/25 bg-rose-500/5 p-4">
				<h2 class="text-sm font-medium text-rose-700 dark:text-rose-400">Removed ({diff.removed.length})</h2>
				<ul class="mt-2 max-h-64 space-y-1 overflow-y-auto font-mono text-xs">
					{#each diff.removed as row}
						<li class="text-foreground/90">{formatCell(row)}</li>
					{/each}
				</ul>
			</section>
			<section class="rounded-lg border border-amber-500/25 bg-amber-500/5 p-4 md:col-span-3">
				<h2 class="text-sm font-medium text-amber-800 dark:text-amber-300">Changed ({diff.changed.length})</h2>
				<ul class="mt-2 max-h-64 space-y-1 overflow-y-auto font-mono text-xs">
					{#each diff.changed as row}
						<li class="text-foreground/90">{formatCell(row)}</li>
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
