<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { GitBranch, Loader2, RefreshCw } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Separator } from '$lib/components/ui/separator/index.js';
	import * as Tabs from '$lib/components/ui/tabs/index.js';
	import {
		fetchBranches,
		fetchFactsList,
		unarchiveSessionExom,
		type BranchRow,
		type ListedFact,
		type TreeExom
	} from '$lib/exomem.svelte';
	import FactsDataTable from './FactsDataTable.svelte';
	import SessionFactsPanel from './SessionFactsPanel.svelte';

	let {
		node,
		sessionModes = false,
		readOnly = false,
		contentDimmed = false,
		showUnarchive = false
	}: {
		node: TreeExom;
		sessionModes?: boolean;
		readOnly?: boolean;
		contentDimmed?: boolean;
		showUnarchive?: boolean;
	} = $props();

	let tab = $state('facts');
	let branchesLoading = $state(false);
	let branches = $state<BranchRow[]>([]);
	let branchesErr = $state<string | null>(null);

	let factsLoading = $state(false);
	let facts = $state<ListedFact[]>([]);
	let factsErr = $state<string | null>(null);

	let unarchiveBusy = $state(false);
	let factsRetry = $state(0);
	let branchesRetry = $state(0);

	$effect(() => {
		node.path;
		tab;
		branchesRetry;
		if (tab !== 'branches') return;
		let cancelled = false;
		branchesLoading = true;
		branchesErr = null;
		fetchBranches(node.path)
			.then((r) => {
				if (!cancelled) branches = r;
			})
			.catch((e: unknown) => {
				if (!cancelled) branchesErr = e instanceof Error ? e.message : 'Failed to load branches';
			})
			.finally(() => {
				if (!cancelled) branchesLoading = false;
			});
		return () => {
			cancelled = true;
		};
	});

	$effect(() => {
		node.path;
		tab;
		sessionModes;
		factsRetry;
		if (tab !== 'facts' || sessionModes) return;
		let cancelled = false;
		factsLoading = true;
		factsErr = null;
		fetchFactsList(node.path)
			.then((r) => {
				if (!cancelled) facts = r;
			})
			.catch((e: unknown) => {
				if (!cancelled) factsErr = e instanceof Error ? e.message : 'Failed to load facts';
			})
			.finally(() => {
				if (!cancelled) factsLoading = false;
			});
		return () => {
			cancelled = true;
		};
	});

	function onUnarchive() {
		actorPrompt.run(async () => {
			unarchiveBusy = true;
			try {
				await unarchiveSessionExom(node.path);
				toast.success('Session unarchived');
				await invalidateAll();
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Unarchive failed');
			} finally {
				unarchiveBusy = false;
			}
		});
	}

	const kindLabel = $derived(
		node.exom_kind === 'project-main'
			? 'project-main'
			: node.exom_kind === 'session'
				? 'session'
				: node.exom_kind === 'bare'
					? 'bare'
					: node.exom_kind
	);
</script>

<div class="flex flex-col gap-4" class:opacity-60={contentDimmed}>
	<header class="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
		<div class="min-w-0 flex-1 space-y-2">
			<p class="break-all font-mono text-sm text-zinc-100">{node.path}</p>
			<div class="flex flex-wrap items-center gap-2 text-xs">
				<span class="text-zinc-500">Facts</span>
				<span class="font-mono text-zinc-200">{node.fact_count}</span>
				<Separator orientation="vertical" class="hidden h-4 sm:inline-flex" />
				<Badge variant="outline" class="border-zinc-600 font-mono text-[10px] text-zinc-200">
					<GitBranch class="mr-1 size-3 opacity-70" />
					{node.current_branch}
				</Badge>
				<Badge variant="secondary" class="text-[10px] capitalize text-zinc-200">{kindLabel}</Badge>
				{#if node.archived}
					<Badge variant="outline" class="border-amber-700/60 text-amber-200">archived</Badge>
				{/if}
				{#if node.closed}
					<Badge variant="outline" class="border-red-800/60 text-red-200">closed</Badge>
				{/if}
			</div>
		</div>
		{#if showUnarchive}
			<Button
				size="sm"
				variant="secondary"
				disabled={unarchiveBusy}
				onclick={() => void onUnarchive()}
			>
				{#if unarchiveBusy}
					<Loader2 class="mr-1 size-3 animate-spin" />
				{/if}
				Unarchive
			</Button>
		{/if}
	</header>

	<Tabs.Root bind:value={tab} class="w-full">
		<Tabs.List class="bg-zinc-950/80">
			<Tabs.Trigger value="facts">Facts</Tabs.Trigger>
			<Tabs.Trigger value="branches">Branches</Tabs.Trigger>
			<Tabs.Trigger value="history">History</Tabs.Trigger>
			<Tabs.Trigger value="graph">Graph</Tabs.Trigger>
			<Tabs.Trigger value="rules">Rules</Tabs.Trigger>
		</Tabs.List>

		<Tabs.Content value="facts" class="mt-4">
			{#if sessionModes}
				<SessionFactsPanel exomPath={node.path} />
			{:else if factsErr}
				<div class="flex flex-col gap-2 rounded-md border border-red-900/40 bg-red-950/25 px-3 py-2 text-sm text-red-200">
					<p>{factsErr}</p>
					<Button
						variant="outline"
						size="sm"
						class="w-fit border-red-800/60 text-red-100"
						onclick={() => {
							factsErr = null;
							factsRetry++;
						}}
					>
						<RefreshCw class="mr-1 size-3" />
						Retry
					</Button>
				</div>
			{:else}
				<FactsDataTable facts={facts} loading={factsLoading} emptyMessage="No facts yet" />
			{/if}
		</Tabs.Content>

		<Tabs.Content value="branches" class="mt-4 space-y-3">
			{#if branchesLoading}
				<p class="flex items-center gap-2 text-sm text-zinc-500">
					<Loader2 class="size-4 animate-spin" /> Loading branches…
				</p>
			{:else if branchesErr}
				<div class="flex flex-col gap-2 rounded-md border border-red-900/40 bg-red-950/25 px-3 py-2 text-sm text-red-200">
					<p>{branchesErr}</p>
					<Button
						variant="outline"
						size="sm"
						class="w-fit border-red-800/60 text-red-100"
						onclick={() => {
							branchesErr = null;
							branchesRetry++;
						}}
					>
						<RefreshCw class="mr-1 size-3" />
						Retry
					</Button>
				</div>
			{:else if branches.length === 0}
				<p class="text-sm text-zinc-500">No branches</p>
			{:else}
				<ul class="space-y-2">
					{#each branches as b (b.branch_id)}
						<li
							class="flex flex-wrap items-center justify-between gap-2 rounded-md border border-zinc-700 bg-zinc-950/40 px-3 py-2"
						>
							<div class="flex flex-wrap items-center gap-2">
								<span class="font-mono text-sm text-zinc-100">{b.name}</span>
								{#if b.is_current}
									<Badge class="text-[10px]">current</Badge>
								{/if}
								{#if b.archived}
									<Badge variant="outline" class="text-[10px]">archived</Badge>
								{/if}
							</div>
							<div class="flex flex-wrap items-center gap-2 text-xs text-zinc-400">
								<span>{b.fact_count} facts</span>
								{#if b.claimed_by}
									<Badge variant="secondary" class="font-mono text-[10px] text-zinc-200">
										{b.claimed_by}
									</Badge>
								{:else}
									<span class="text-zinc-600">unclaimed</span>
								{/if}
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</Tabs.Content>

		<Tabs.Content value="history" class="mt-4">
			<p class="text-sm text-zinc-500">Coming soon</p>
		</Tabs.Content>
		<Tabs.Content value="graph" class="mt-4">
			<p class="text-sm text-zinc-500">Coming soon</p>
		</Tabs.Content>
		<Tabs.Content value="rules" class="mt-4">
			<p class="text-sm text-zinc-500">Coming soon</p>
		</Tabs.Content>
	</Tabs.Root>
</div>
