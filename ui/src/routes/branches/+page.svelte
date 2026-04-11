<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { Check, Copy, GitBranch, GitMerge, Loader2, Plus, RefreshCw, Route, Trash2 } from '@lucide/svelte';

	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import {
		createBranch,
		deleteBranch,
		fetchBranchRows,
		fetchBranches,
		fetchExomemStatus,
		fetchMergeRows,
		switchBranch,
		type BranchRow,
		type BranchViewRow,
		type MergeViewRow
	} from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';

	let branches = $state<BranchRow[]>([]);
	let branchViews = $state<BranchViewRow[]>([]);
	let merges = $state<MergeViewRow[]>([]);
	let currentBranch = $state('main');
	let loading = $state(true);
	let errorMessage = $state<string | null>(null);
	let newId = $state('');
	let newName = $state('');
	let creating = $state(false);
	let switching = $state<string | null>(null);
	let deleting = $state<string | null>(null);
	let confirmDelete = $state<string | null>(null);
	let copiedSnippet = $state<string | null>(null);
	let copyTimer: ReturnType<typeof setTimeout> | null = null;

	const branchViewMap = $derived(
		new Map(branchViews.map((view) => [view.branch_id, view]))
	);
	const archivedCount = $derived(branches.filter((b) => b.archived).length);
	const activeCount = $derived(branches.filter((b) => !b.archived).length);

	function branchRowQuery(): string {
		return `(query ${app.selectedExom} (find ?branch ?id ?name ?archived ?createdTx) (where (branch-row ?branch ?id ?name ?archived ?createdTx)))`;
	}

	function mergeRowQuery(): string {
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

	async function load() {
		loading = true;
		errorMessage = null;
		try {
			const [branchList, status, branchRows, mergeRows] = await Promise.all([
				fetchBranches(app.selectedExom),
				fetchExomemStatus(app.selectedExom),
				fetchBranchRows(app.selectedExom),
				fetchMergeRows(app.selectedExom)
			]);
			branches = branchList;
			currentBranch = status.current_branch ?? 'main';
			branchViews = branchRows;
			merges = mergeRows;
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : String(e);
			branches = [];
			branchViews = [];
			merges = [];
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (!browser) return;
		app.selectedExom;
		void load();
		return () => {
			if (copyTimer) clearTimeout(copyTimer);
		};
	});

	async function handleCreate(e: Event) {
		e.preventDefault();
		const id = newId.trim();
		if (!id) return;
		creating = true;
		errorMessage = null;
		try {
			await createBranch(id, newName.trim() || id, app.selectedExom);
			newId = '';
			newName = '';
			await load();
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : String(e);
		} finally {
			creating = false;
		}
	}

	async function handleSwitch(branchId: string) {
		switching = branchId;
		errorMessage = null;
		try {
			await switchBranch(branchId, app.selectedExom);
			await load();
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : String(e);
		} finally {
			switching = null;
		}
	}

	async function handleDelete(branchId: string) {
		deleting = branchId;
		errorMessage = null;
		try {
			await deleteBranch(branchId, app.selectedExom);
			confirmDelete = null;
			await load();
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : String(e);
		} finally {
			deleting = null;
		}
	}
</script>

<div class="mx-auto max-w-6xl space-y-8 p-6">
	<div class="flex flex-wrap items-start justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">Branches</h1>
			<p class="mt-1 text-sm text-muted-foreground">
				Branch state is presented from both the management API and the Datalog-native <span class="font-mono">branch-row</span> / <span class="font-mono">merge-row</span> views.
			</p>
		</div>
		<Button variant="outline" size="sm" onclick={() => void load()} disabled={loading}>
			<RefreshCw class="mr-1 size-3.5" />
			Refresh
		</Button>
	</div>

	<div class="grid gap-3 xl:grid-cols-4">
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Current branch</p>
			<p class="mt-1 font-mono text-lg font-semibold">{currentBranch}</p>
			<p class="mt-1 text-xs text-muted-foreground">Active visibility baseline for queries and writes.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Active branches</p>
			<p class="mt-1 text-lg font-semibold">{activeCount}</p>
			<p class="mt-1 text-xs text-muted-foreground">Non-archived speculative workspaces.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Archived branches</p>
			<p class="mt-1 text-lg font-semibold">{archivedCount}</p>
			<p class="mt-1 text-xs text-muted-foreground">Retained branch history that is no longer active.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Merge events</p>
			<p class="mt-1 text-lg font-semibold">{merges.length}</p>
			<p class="mt-1 text-xs text-muted-foreground">Rows from the built-in <span class="font-mono">merge-row</span> view.</p>
		</div>
	</div>

	<div class="grid gap-4 xl:grid-cols-[0.95fr_1.05fr]">
		<div class="rounded-lg border border-border/60 bg-card p-4">
			<div class="flex items-center justify-between gap-2">
				<div>
					<h2 class="text-sm font-medium text-muted-foreground">System views</h2>
					<p class="mt-1 text-sm text-foreground">Branch and merge metadata are queryable like the rest of the substrate.</p>
				</div>
				<Route class="size-4 text-muted-foreground" />
			</div>
			<div class="mt-3 grid gap-2">
				<div class="rounded-md border border-border/50 bg-muted/20 px-3 py-3">
					<div class="flex items-start justify-between gap-3">
						<div>
							<div class="flex items-center gap-2">
								<span class="font-mono text-sm text-fact-derived">branch-row</span>
								<Badge variant="outline" class="h-4 px-1.5 text-[10px]">system</Badge>
							</div>
							<p class="mt-1 text-xs text-muted-foreground">Branch entity, id, name, archive state, and creating tx.</p>
						</div>
						<div class="flex items-center gap-1">
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => copySnippet('branch-row', branchRowQuery())}>
								{#if copiedSnippet === 'branch-row'}
									<Check class="mr-1 size-3.5" />
									Copied
								{:else}
									<Copy class="mr-1 size-3.5" />
									Copy
								{/if}
							</Button>
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => openInQuery(branchRowQuery())}>Open</Button>
						</div>
					</div>
				</div>

				<div class="rounded-md border border-border/50 bg-muted/20 px-3 py-3">
					<div class="flex items-start justify-between gap-3">
						<div>
							<div class="flex items-center gap-2">
								<span class="font-mono text-sm text-fact-derived">merge-row</span>
								<Badge variant="outline" class="h-4 px-1.5 text-[10px]">system</Badge>
							</div>
							<p class="mt-1 text-xs text-muted-foreground">Merge tx, source branch, target branch, actor, and timestamp.</p>
						</div>
						<div class="flex items-center gap-1">
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => copySnippet('merge-row', mergeRowQuery())}>
								{#if copiedSnippet === 'merge-row'}
									<Check class="mr-1 size-3.5" />
									Copied
								{:else}
									<Copy class="mr-1 size-3.5" />
									Copy
								{/if}
							</Button>
							<Button variant="ghost" size="sm" class="h-7 px-2 text-[0.7rem]" onclick={() => openInQuery(mergeRowQuery())}>Open</Button>
						</div>
					</div>
				</div>
			</div>
		</div>

		<form class="flex flex-wrap items-end gap-3 rounded-lg border border-border/60 bg-card/40 p-4" onsubmit={handleCreate}>
			<div class="min-w-[10rem] flex-1 space-y-1">
				<label class="text-xs font-medium text-muted-foreground" for="new-branch-id">Branch id</label>
				<Input id="new-branch-id" bind:value={newId} placeholder="e.g. what-if-layoffs" class="font-mono text-sm" />
			</div>
			<div class="min-w-[12rem] flex-1 space-y-1">
				<label class="text-xs font-medium text-muted-foreground" for="new-branch-name">Display name</label>
				<Input id="new-branch-name" bind:value={newName} placeholder="Optional label" class="text-sm" />
			</div>
			<Button type="submit" disabled={creating || !newId.trim()} class="gap-1.5">
				{#if creating}
					<Loader2 class="size-4 animate-spin" />
				{:else}
					<Plus class="size-4" />
				{/if}
				Create
			</Button>
		</form>
	</div>

	{#if errorMessage}
		<p class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">{errorMessage}</p>
	{/if}

	{#if loading}
		<div class="flex items-center gap-2 text-sm text-muted-foreground">
			<Loader2 class="size-4 animate-spin" />
			Loading branches…
		</div>
	{:else}
		<div class="overflow-x-auto rounded-lg border border-border/60">
			<table class="w-full text-sm">
				<thead>
					<tr class="border-b border-border/40 bg-muted/30">
						<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">Branch</th>
						<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">Parent</th>
						<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">Created tx</th>
						<th class="px-3 py-2 text-right text-xs font-medium text-muted-foreground">Facts</th>
						<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">State</th>
						<th class="w-48 px-3 py-2"></th>
					</tr>
				</thead>
				<tbody class="divide-y divide-border/30">
					{#each branches as b (b.branch_id)}
						{@const view = branchViewMap.get(b.branch_id)}
						<tr class="hover:bg-muted/15">
							<td class="px-3 py-2">
								<div class="flex items-center gap-2">
									<GitBranch class="size-3.5 shrink-0 text-muted-foreground" />
									<a
										href={`${base}/branches/${encodeURIComponent(b.branch_id)}`}
										class="font-mono text-xs text-primary hover:underline"
									>
										{b.branch_id}
									</a>
									{#if b.is_current}
										<Badge variant="secondary" class="h-4 px-1.5 text-[10px]">current</Badge>
									{/if}
									{#if b.archived}
										<Badge variant="outline" class="h-4 px-1.5 text-[10px]">archived</Badge>
									{/if}
								</div>
								{#if b.name !== b.branch_id}
									<p class="mt-0.5 pl-5 text-xs text-muted-foreground">{b.name}</p>
								{/if}
							</td>
							<td class="px-3 py-2 font-mono text-xs text-muted-foreground">{b.parent_branch_id ?? '—'}</td>
							<td class="px-3 py-2 font-mono text-xs text-muted-foreground">{view?.created_tx ?? `tx/${b.created_tx_id}`}</td>
							<td class="px-3 py-2 text-right tabular-nums">{b.fact_count}</td>
							<td class="px-3 py-2 text-xs">
								{#if b.archived}
									<span class="text-muted-foreground">archived</span>
								{:else if b.is_current}
									<span class="text-primary">visible</span>
								{:else}
									<span class="text-muted-foreground">inactive</span>
								{/if}
							</td>
							<td class="px-3 py-2 text-right">
								<div class="flex justify-end gap-1">
									{#if !b.archived && !b.is_current}
										<Button variant="ghost" size="sm" class="h-7 text-xs" disabled={switching === b.branch_id} onclick={() => void handleSwitch(b.branch_id)}>
											{#if switching === b.branch_id}
												<Loader2 class="mr-1 size-3 animate-spin" />
											{/if}
											Switch
										</Button>
									{/if}
									{#if b.branch_id !== 'main' && !b.archived}
										{#if confirmDelete === b.branch_id}
											<Button variant="ghost" size="sm" class="h-7 text-xs" onclick={() => (confirmDelete = null)}>Cancel</Button>
											<Button
												variant="destructive"
												size="sm"
												class="h-7 gap-1 text-xs"
												disabled={deleting === b.branch_id}
												onclick={() => void handleDelete(b.branch_id)}
											>
												{#if deleting === b.branch_id}
													<Loader2 class="size-3 animate-spin" />
												{:else}
													<Trash2 class="size-3" />
												{/if}
												Archive
											</Button>
										{:else}
											<Button variant="ghost" size="sm" class="h-7 text-xs text-muted-foreground" onclick={() => (confirmDelete = b.branch_id)}>
												Delete
											</Button>
										{/if}
									{/if}
								</div>
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</div>
