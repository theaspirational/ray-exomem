<script lang="ts">
	import { base } from '$app/paths';
	import { GitBranch, Loader2, Plus, Trash2 } from '@lucide/svelte';

	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { createBranch, deleteBranch, fetchBranches, type BranchRow } from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';

	let branches = $state<BranchRow[]>([]);
	let loading = $state(true);
	let errorMessage = $state<string | null>(null);
	let newId = $state('');
	let newName = $state('');
	let creating = $state(false);
	let deleting = $state<string | null>(null);
	let confirmDelete = $state<string | null>(null);

	async function load() {
		loading = true;
		errorMessage = null;
		try {
			branches = await fetchBranches(app.selectedExom);
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : String(e);
			branches = [];
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		app.selectedExom;
		void load();
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

<div class="mx-auto max-w-4xl space-y-8 p-6">
	<div class="flex flex-wrap items-start justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">Branches</h1>
			<p class="mt-1 text-sm text-muted-foreground">
				Isolated views of the knowledge base (exom: <span class="font-mono">{app.selectedExom}</span>).
			</p>
		</div>
		<Button variant="outline" size="sm" onclick={() => void load()} disabled={loading}>Refresh</Button>
	</div>

	{#if errorMessage}
		<p class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">{errorMessage}</p>
	{/if}

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
						<th class="px-3 py-2 text-right text-xs font-medium text-muted-foreground">Facts</th>
						<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">State</th>
						<th class="w-28 px-3 py-2"></th>
					</tr>
				</thead>
				<tbody class="divide-y divide-border/30">
					{#each branches as b (b.branch_id)}
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
								</div>
								{#if b.name !== b.branch_id}
									<p class="mt-0.5 pl-5 text-xs text-muted-foreground">{b.name}</p>
								{/if}
							</td>
							<td class="px-3 py-2 font-mono text-xs text-muted-foreground">
								{b.parent_branch_id ?? '—'}
							</td>
							<td class="px-3 py-2 text-right tabular-nums">{b.fact_count}</td>
							<td class="px-3 py-2 text-xs">
								{#if b.archived}
									<span class="text-muted-foreground">archived</span>
								{:else if b.is_current}
									<span class="text-primary">current</span>
								{:else}
									<span class="text-muted-foreground">—</span>
								{/if}
							</td>
							<td class="px-3 py-2 text-right">
								{#if b.branch_id !== 'main' && !b.archived}
									{#if confirmDelete === b.branch_id}
										<div class="flex justify-end gap-1">
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
										</div>
									{:else}
										<Button variant="ghost" size="sm" class="h-7 text-xs text-muted-foreground" onclick={() => (confirmDelete = b.branch_id)}>
											Delete
										</Button>
									{/if}
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</div>
