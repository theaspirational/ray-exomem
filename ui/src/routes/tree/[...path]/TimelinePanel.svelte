<script lang="ts">
	import { browser } from '$app/environment';
	import { Calendar, GitBranch, Route, Search } from '@lucide/svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { exportBackupText, fetchExomemStatus, parseFactsFromExport } from '$lib/exomem.svelte';
	import type { FactEntry } from '$lib/types';

	let { exomPath, notebookMode = false }: { exomPath: string; notebookMode?: boolean } = $props();

	const INITIAL_TIMELINE_LIMIT = 20;

	let facts = $state<FactEntry[]>([]);
	let currentBranch = $state('main');
	let loading = $state(true);
	let searchQuery = $state('');
	let showAllTimeline = $state(false);

	const temporalFacts = $derived(
		facts.filter((f) => f.validFrom != null)
	);

	const filteredFacts = $derived.by(() => {
		const q = searchQuery.trim().toLowerCase();
		if (!q) return temporalFacts;
		return temporalFacts.filter(
			(f) =>
				f.predicate.toLowerCase().includes(q) ||
				f.terms.some((t) => t.toLowerCase().includes(q)) ||
				(f.branchRole ?? '').toLowerCase().includes(q) ||
				(f.branchOrigin ?? '').toLowerCase().includes(q)
		);
	});

	function compareTimelineFactsNewestFirst(a: FactEntry, b: FactEntry): number {
		return (b.validFrom ?? '').localeCompare(a.validFrom ?? '');
	}

	const sortedFilteredFacts = $derived(
		filteredFacts.slice().sort(compareTimelineFactsNewestFirst)
	);

	const visibleFacts = $derived(
		showAllTimeline ? sortedFilteredFacts : sortedFilteredFacts.slice(0, INITIAL_TIMELINE_LIMIT)
	);

	const hiddenTimelineCount = $derived(
		Math.max(0, sortedFilteredFacts.length - visibleFacts.length)
	);

	const branchRoleCounts = $derived({
		local: temporalFacts.filter((f) => f.branchRole === 'local').length,
		inherited: temporalFacts.filter((f) => f.branchRole === 'inherited').length,
		override: temporalFacts.filter((f) => f.branchRole === 'override').length
	});

	const openEndedCount = $derived(
		temporalFacts.filter((f) => !f.validTo).length
	);

	const timelineGroups = $derived.by(() => {
		const groups: Array<[string, FactEntry[]]> = [];
		for (const fact of visibleFacts) {
			const from = fact.validFrom ?? 'unknown';
			const key = from.includes('T') ? from.split('T')[0] : from;
			const existing = groups.find(([date]) => date === key);
			if (existing) existing[1].push(fact);
			else groups.push([key, [fact]]);
		}
		return groups.sort(([a], [b]) => b.localeCompare(a));
	});

	async function loadTimeline() {
		loading = true;
		showAllTimeline = false;
		try {
			const [dlText, status] = await Promise.all([
				exportBackupText(exomPath),
				fetchExomemStatus(exomPath)
			]);
			facts = parseFactsFromExport(dlText);
			currentBranch = status.current_branch ?? 'main';
		} catch {
			facts = [];
			currentBranch = 'main';
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (!browser) return;
		exomPath;
		void loadTimeline();
	});
</script>

{#if notebookMode && !loading && temporalFacts.length === 0}
	<p class="font-serif text-sm text-muted-foreground">No changes recorded yet.</p>
{:else}
<div class="flex flex-col gap-4">
	<div class="grid gap-3 xl:grid-cols-4">
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Current branch</p>
			<p class="mt-1 font-mono text-lg font-semibold">{currentBranch}</p>
			<p class="mt-1 text-xs text-muted-foreground">Visibility scope for the exported fact set.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Open-ended facts</p>
			<p class="mt-1 text-lg font-semibold">{openEndedCount}</p>
			<p class="mt-1 text-xs text-muted-foreground">Facts still valid with no explicit end date.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Inherited visibility</p>
			<p class="mt-1 text-lg font-semibold">{branchRoleCounts.inherited}</p>
			<p class="mt-1 text-xs text-muted-foreground">Facts visible from ancestor branches.</p>
		</div>
		<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
			<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Override/local</p>
			<p class="mt-1 text-lg font-semibold">{branchRoleCounts.override + branchRoleCounts.local}</p>
			<p class="mt-1 text-xs text-muted-foreground">Facts asserted or overridden in the active branch.</p>
		</div>
	</div>

	<div class="rounded-lg border border-border/60 bg-card p-4">
		<div class="flex items-center justify-between gap-3">
			<div>
				<h2 class="text-sm font-medium text-muted-foreground">Visibility model</h2>
				<p class="mt-1 text-sm text-foreground">
					Timeline cards expose branch role and branch origin so you can tell whether a visible fact is local, inherited, or overriding ancestor history.
				</p>
			</div>
			<Route class="size-4 text-muted-foreground" />
		</div>
		<div class="mt-3 flex flex-wrap gap-2">
			<Badge variant="outline" class="h-5 px-2 text-[0.65rem]"><GitBranch class="mr-1 size-3" /> local {branchRoleCounts.local}</Badge>
			<Badge variant="outline" class="h-5 px-2 text-[0.65rem]"><GitBranch class="mr-1 size-3" /> inherited {branchRoleCounts.inherited}</Badge>
			<Badge variant="outline" class="h-5 px-2 text-[0.65rem]"><GitBranch class="mr-1 size-3" /> override {branchRoleCounts.override}</Badge>
		</div>
	</div>

	<div class="flex flex-col gap-3 sm:flex-row sm:items-center">
		<div class="relative flex-1 max-w-md">
			<Search class="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
			<Input class="pl-9" placeholder="Search facts, branch role, or branch origin..." bind:value={searchQuery} />
		</div>
		<Badge variant="outline">{temporalFacts.length} temporal facts</Badge>
	</div>

	{#if loading}
		<div class="flex flex-col gap-4">
			{#each Array.from({ length: 4 }) as _, i (i)}
				<div class="flex gap-4">
					<div class="h-4 w-24 animate-pulse rounded bg-muted"></div>
					<div class="h-12 flex-1 animate-pulse rounded-lg bg-muted/40"></div>
				</div>
			{/each}
		</div>
	{:else if temporalFacts.length === 0}
		<div class="flex flex-col items-center gap-3 rounded-lg border border-border/60 px-6 py-16 text-center">
			<Calendar class="size-10 text-muted-foreground/30" />
			<div>
				<p class="text-sm font-medium text-muted-foreground">No temporal facts</p>
				<p class="mt-1 text-xs text-muted-foreground/70">
					Assert facts with valid-from/valid-to to see them here
				</p>
			</div>
		</div>
	{:else}
		<div class="relative flex flex-col gap-0">
			{#each timelineGroups as [date, groupFacts] (date)}
				<div class="flex gap-4">
					<div class="flex w-28 shrink-0 flex-col items-end pt-1">
						<span class="font-mono text-xs font-medium tabular-nums text-foreground">{date}</span>
					</div>

					<div class="relative flex flex-col items-center">
						<div class="absolute top-0 bottom-0 w-px bg-border/60"></div>
						<div class="relative z-10 mt-1.5 size-2.5 rounded-full border-2 border-primary bg-background"></div>
					</div>

					<div class="flex flex-1 flex-col gap-1.5 pb-6 pt-0.5">
						{#each groupFacts as fact, j (`${fact.predicate}-${fact.terms.join(',')}-${j}`)}
							<div class="rounded-md border border-border/60 px-3 py-2 transition-colors hover:border-border">
								<div class="flex flex-wrap items-center gap-2">
									<span class="font-mono text-sm text-fact-base">{fact.predicate}</span>
									<span class="font-mono text-xs text-muted-foreground">({fact.terms.join(', ')})</span>
									{#if fact.branchRole}
										<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{fact.branchRole}</Badge>
									{/if}
									{#if fact.branchOrigin}
										<Badge variant="outline" class="h-4 px-1.5 text-[10px]">origin {fact.branchOrigin}</Badge>
									{/if}
								</div>
								<div class="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
									<span>{fact.validFrom}</span>
									<span class="text-muted-foreground/50">&rarr;</span>
									<span>{fact.validTo ?? 'still active'}</span>
								</div>
							</div>
						{/each}
					</div>
				</div>
			{/each}
		</div>
		{#if hiddenTimelineCount > 0}
			<div class="flex justify-center pt-2">
				<Button variant="outline" size="sm" onclick={() => (showAllTimeline = true)}>
					Show all {sortedFilteredFacts.length} timeline events
				</Button>
			</div>
		{:else if showAllTimeline && sortedFilteredFacts.length > INITIAL_TIMELINE_LIMIT}
			<div class="flex justify-center pt-2">
				<Button variant="outline" size="sm" onclick={() => (showAllTimeline = false)}>
					Show latest {INITIAL_TIMELINE_LIMIT}
				</Button>
			</div>
		{/if}
	{/if}
</div>
{/if}
