<script lang="ts">
	import { Calendar, Search } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Badge } from '$lib/components/ui/badge';
	import { app } from '$lib/stores.svelte';
	import { exportBackupText, parseFactsFromExport } from '$lib/exomem.svelte';
	import type { FactEntry } from '$lib/types';
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';

	let facts = $state<FactEntry[]>([]);
	let loading = $state(true);
	let searchQuery = $state('');

	const temporalFacts = $derived(
		facts.filter((f) => f.validFrom != null)
	);

	const filteredFacts = $derived(() => {
		const q = searchQuery.trim().toLowerCase();
		if (!q) return temporalFacts;
		return temporalFacts.filter(
			(f) =>
				f.predicate.toLowerCase().includes(q) ||
				f.terms.some((t) => t.toLowerCase().includes(q))
		);
	});

	// Group facts by their validity start date for the timeline
	const timelineGroups = $derived(() => {
		const groups = new Map<string, FactEntry[]>();
		for (const fact of filteredFacts()) {
			// Extract just the date portion for grouping
			const from = fact.validFrom ?? 'unknown';
			const key = from.includes('T') ? from.split('T')[0] : from;
			const existing = groups.get(key);
			if (existing) {
				existing.push(fact);
			} else {
				groups.set(key, [fact]);
			}
		}
		return Array.from(groups.entries()).sort(([a], [b]) => a.localeCompare(b));
	});

	onMount(async () => {
		try {
			const dlText = await exportBackupText(app.selectedExom);
			facts = parseFactsFromExport(dlText);
		} catch {
			// handled silently
		} finally {
			loading = false;
		}
	});
</script>

<div class="flex flex-col gap-6 p-4 sm:p-6 lg:p-8">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">Validity Timeline</h1>
		<p class="text-sm text-muted-foreground">
			Bitemporal validity timeline. Each entry shows when a fact was true in the real world, independent of when it was recorded.
		</p>
	</div>

	<div class="flex flex-col gap-3 sm:flex-row sm:items-center">
		<div class="relative flex-1 max-w-md">
			<Search class="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
			<Input class="pl-9" placeholder="Search facts by predicate or terms..." bind:value={searchQuery} />
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
					Assert facts with <code class="font-mono">--valid-from</code> / <code class="font-mono">--valid-to</code> to see them on the bitemporal timeline.
				</p>
			</div>
			<Button variant="outline" size="sm" href={resolve('/facts')}>
				Browse Facts
			</Button>
		</div>
	{:else}
		<!-- Timeline visualization -->
		<div class="relative flex flex-col gap-0">
			{#each timelineGroups() as [date, groupFacts], i (date)}
				<div class="flex gap-4">
					<!-- Date column -->
					<div class="flex w-28 shrink-0 flex-col items-end pt-1">
						<span class="font-mono text-xs font-medium tabular-nums text-foreground">{date}</span>
					</div>

					<!-- Timeline line -->
					<div class="relative flex flex-col items-center">
						<div class="absolute top-0 bottom-0 w-px bg-border/60"></div>
						<div class="relative z-10 mt-1.5 size-2.5 rounded-full border-2 border-primary bg-background"></div>
					</div>

					<!-- Facts for this date -->
					<div class="flex flex-1 flex-col gap-1.5 pb-6 pt-0.5">
						{#each groupFacts as fact, j (fact.predicate + fact.terms.join(',') + j)}
							<div class="rounded-md border border-border/60 px-3 py-2 transition-colors hover:border-border">
								<div class="flex items-center gap-2">
									<span class="font-mono text-sm text-fact-base">{fact.predicate}</span>
									<span class="font-mono text-xs text-muted-foreground">({fact.terms.join(', ')})</span>
								</div>
								<div class="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
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
	{/if}
</div>
