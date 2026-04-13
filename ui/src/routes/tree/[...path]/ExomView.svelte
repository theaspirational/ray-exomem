<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { GitBranch, Loader2 } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import DataRow from '$lib/components/DataRow.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import LoadingState from '$lib/components/LoadingState.svelte';
	import StatCard from '$lib/components/StatCard.svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Tabs from '$lib/components/ui/tabs/index.js';
	import { fetchBranches, unarchiveSessionExom, type BranchRow, type TreeExom } from '$lib/exomem.svelte';
	import FactsManager from './FactsManager.svelte';
	import GraphPanel from './GraphPanel.svelte';
	import RulesPanel from './RulesPanel.svelte';
	import SessionFactsPanel from './SessionFactsPanel.svelte';
	import TimelinePanel from './TimelinePanel.svelte';

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

	let unarchiveBusy = $state(false);
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
		<div class="min-w-0 flex-1 space-y-3">
			<div class="flex flex-wrap items-center gap-2 text-xs">
				<Badge variant="secondary" class="text-[10px] capitalize text-zinc-200">{kindLabel}</Badge>
				{#if node.archived}
					<Badge variant="outline" class="border-amber-700/60 text-amber-200">archived</Badge>
				{/if}
				{#if node.closed}
					<Badge variant="outline" class="border-red-800/60 text-red-200">closed</Badge>
				{/if}
			</div>
			<div class="grid gap-2 sm:grid-cols-2">
				<StatCard label="Facts" value={node.fact_count} />
				<StatCard label="Branch" value={node.current_branch} icon={GitBranch} />
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
			{:else if tab === 'facts'}
				<FactsManager exomPath={node.path} />
			{/if}
		</Tabs.Content>

		<Tabs.Content value="branches" class="mt-4 space-y-3">
			{#if branchesLoading}
				<LoadingState message="Loading branches…" />
			{:else if branchesErr}
				<ErrorState
					message={branchesErr}
					onRetry={() => {
						branchesErr = null;
						branchesRetry++;
					}}
				/>
			{:else if branches.length === 0}
				<p class="text-sm text-zinc-500">No branches</p>
			{:else}
				<div class="space-y-2">
					{#each branches as b (b.branch_id)}
						<DataRow
							icon={GitBranch}
							label={b.name}
							badges={[
								...(b.is_current ? [{ text: 'current', variant: 'default' as const }] : []),
								...(b.archived ? [{ text: 'archived', variant: 'outline' as const }] : []),
								...(b.claimed_by ? [{ text: b.claimed_by, variant: 'secondary' as const }] : [])
							]}
							trailing={`${b.fact_count} facts`}
						/>
					{/each}
				</div>
			{/if}
		</Tabs.Content>

		<Tabs.Content value="history" class="mt-4">
			{#if tab === 'history'}
				<TimelinePanel exomPath={node.path} />
			{/if}
		</Tabs.Content>
		<Tabs.Content value="graph" class="mt-4">
			{#if tab === 'graph'}
				<GraphPanel exomPath={node.path} />
			{/if}
		</Tabs.Content>
		<Tabs.Content value="rules" class="mt-4">
			{#if tab === 'rules'}
				<RulesPanel exomPath={node.path} />
			{/if}
		</Tabs.Content>
	</Tabs.Root>
</div>
