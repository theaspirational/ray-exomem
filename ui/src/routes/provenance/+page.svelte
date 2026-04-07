<script lang="ts">
	import { TreePine, Search, Loader2, CircleAlert, ChevronRight, ChevronDown } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Badge } from '$lib/components/ui/badge';
	import { app } from '$lib/stores.svelte';
	import {
		exportBackupText,
		parseFactsFromExport,
		fetchExomemSchema,
		fetchExplain,
		type ProofTreeNode,
		type ExplainResponse
	} from '$lib/exomem.svelte';
	import type { FactEntry, ExomemSchemaRelation } from '$lib/types';
	import { onMount } from 'svelte';

	let facts = $state<FactEntry[]>([]);
	let derivedRelations = $state<ExomemSchemaRelation[]>([]);
	let loading = $state(true);
	let searchQuery = $state('');
	let selectedFact = $state<FactEntry | null>(null);

	// Explain state
	let explainResult = $state<ExplainResponse | null>(null);
	let explainLoading = $state(false);
	let explainError = $state<string | null>(null);

	// Track which tree nodes are expanded
	let expandedNodes = $state<Set<string>>(new Set());

	const derivedFacts = $derived(
		facts.filter((f) => f.kind === 'derived')
	);

	const filteredFacts = $derived(() => {
		const q = searchQuery.trim().toLowerCase();
		if (!q) return derivedFacts;
		return derivedFacts.filter(
			(f) =>
				f.predicate.toLowerCase().includes(q) ||
				f.terms.some((t) => t.toLowerCase().includes(q))
		);
	});

	onMount(async () => {
		try {
			const [dlText, schema] = await Promise.all([
				exportBackupText(app.selectedExom),
				fetchExomemSchema(app.selectedExom)
			]);
			facts = parseFactsFromExport(dlText);
			derivedRelations = schema.relations.filter((r) => r.kind === 'derived');

			for (const rel of derivedRelations) {
				if (rel.sample_tuples) {
					for (const tuple of rel.sample_tuples) {
						facts.push({
							predicate: rel.name,
							terms: tuple.map(String),
							kind: 'derived',
							confidence: null,
							source: null
						});
					}
				}
			}
		} catch {
			// handled silently
		} finally {
			loading = false;
		}
	});

	async function selectAndExplain(fact: FactEntry) {
		selectedFact = fact;
		explainResult = null;
		explainError = null;
		explainLoading = true;
		expandedNodes = new Set();

		try {
			const result = await fetchExplain(fact.predicate, fact.terms, 10, app.selectedExom);
			explainResult = result;
			// Auto-expand root
			if (result.tree) {
				expandedNodes.add(result.tree.id);
			}
		} catch (e) {
			explainError = e instanceof Error ? e.message : 'Failed to fetch provenance';
		} finally {
			explainLoading = false;
		}
	}

	function toggleNode(nodeId: string) {
		const next = new Set(expandedNodes);
		if (next.has(nodeId)) {
			next.delete(nodeId);
		} else {
			next.add(nodeId);
		}
		expandedNodes = next;
	}

	function nodeLabel(node: ProofTreeNode): string {
		return `${node.predicate}(${node.terms.map(String).join(', ')})`;
	}
</script>

<div class="flex flex-col gap-6 p-4 sm:p-6 lg:p-8">
	<div>
		<h1 class="text-2xl font-semibold tracking-tight">Provenance</h1>
		<p class="text-sm text-muted-foreground">
			Why does this fact exist? Select a derived fact to trace its support chain — the rules that fired and the base facts that justify it.
		</p>
	</div>

	<div class="flex items-center gap-3">
		<div class="relative flex-1 max-w-md">
			<Search class="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
			<Input class="pl-9" placeholder="Search derived facts..." bind:value={searchQuery} />
		</div>
		<Badge variant="outline">{derivedFacts.length} derived facts</Badge>
	</div>

	<div class="grid gap-6 lg:grid-cols-[1fr_1fr]">
		<!-- Derived facts list -->
		<div class="flex flex-col gap-2">
			<h2 class="text-sm font-medium text-muted-foreground">Derived facts</h2>
			{#if loading}
				<div class="flex flex-col gap-2">
					{#each Array.from({ length: 5 }) as _, i (i)}
						<div class="h-12 animate-pulse rounded-lg bg-muted/40"></div>
					{/each}
				</div>
			{:else if filteredFacts().length === 0}
				<div class="flex flex-col items-center gap-2 rounded-lg border border-border/60 px-6 py-12 text-center">
					<TreePine class="size-8 text-muted-foreground/30" />
					<p class="text-sm text-muted-foreground">
						{derivedFacts.length === 0
							? 'No derived facts yet. Add rules and evaluate to generate derivations.'
							: 'No matches for your search.'}
					</p>
				</div>
			{:else}
				<div class="max-h-[60vh] overflow-y-auto rounded-lg border border-border/60 divide-y divide-border/40 no-scrollbar">
					{#each filteredFacts() as fact, i (fact.predicate + fact.terms.join(',') + i)}
						<button
							class="flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors hover:bg-muted/30
								{selectedFact === fact ? 'bg-primary/10 border-l-2 border-l-fact-derived' : ''}"
							onclick={() => selectAndExplain(fact)}
						>
							<span class="font-mono text-sm text-fact-derived">{fact.predicate}</span>
							<span class="font-mono text-xs text-muted-foreground">({fact.terms.join(', ')})</span>
						</button>
					{/each}
				</div>
			{/if}
		</div>

		<!-- Proof tree area -->
		<div class="flex flex-col gap-2">
			<h2 class="text-sm font-medium text-muted-foreground">Support chain</h2>
			<div class="rounded-lg border border-border/60">
				{#if !selectedFact}
					<div class="flex flex-col items-center gap-2 px-6 py-12 text-center">
						<TreePine class="size-8 text-muted-foreground/30" />
						<p class="text-sm text-muted-foreground">Select a derived fact to see why it exists.</p>
					</div>
				{:else if explainLoading}
					<div class="flex flex-col items-center gap-3 px-6 py-12 text-center">
						<Loader2 class="size-5 animate-spin text-primary" />
						<p class="text-sm text-muted-foreground">Tracing provenance...</p>
					</div>
				{:else if explainError}
					<div class="flex flex-col items-center gap-3 px-6 py-8 text-center">
						<CircleAlert class="size-5 text-contra" />
						<div class="flex flex-col gap-1">
							<p class="text-sm font-medium text-contra">Could not trace provenance</p>
							<p class="text-xs text-contra/70">{explainError}</p>
						</div>
						<Button variant="outline" size="sm" onclick={() => selectedFact && selectAndExplain(selectedFact)}>
							Retry
						</Button>
					</div>
				{:else if explainResult}
					<div class="p-4">
						<!-- Root fact highlight -->
						<div class="mb-4 rounded-lg border border-fact-derived/30 bg-fact-derived/5 px-4 py-3">
							<div class="flex items-center gap-2">
								<Badge variant="secondary" class="text-[0.6rem] px-1.5 h-4">derived</Badge>
								<span class="font-mono text-sm text-fact-derived">
									{explainResult.predicate}({explainResult.terms.join(', ')})
								</span>
							</div>
							{#if explainResult.meta}
								<div class="mt-2 flex flex-wrap gap-3 text-xs text-muted-foreground">
									{#if explainResult.meta.confidence != null}
										<span>Confidence: <span class="font-medium text-foreground">{explainResult.meta.confidence}</span></span>
									{/if}
									{#if explainResult.meta.source}
										<span>Source: <span class="font-medium text-foreground">{explainResult.meta.source}</span></span>
									{/if}
								</div>
							{/if}
						</div>

						<!-- Recursive proof tree -->
						<div class="text-sm">
							{#snippet proofNode(node: ProofTreeNode, depth: number)}
								{@const isExpanded = expandedNodes.has(node.id)}
								{@const hasChildren = (node.derivations && node.derivations.length > 0) || false}
								{@const isBase = node.kind === 'base'}

								<div class="flex flex-col" style="margin-left: {depth * 16}px">
									<button
										class="flex items-center gap-1.5 rounded px-1.5 py-1 text-left transition-colors hover:bg-muted/30"
										onclick={() => hasChildren && toggleNode(node.id)}
										disabled={!hasChildren}
									>
										{#if hasChildren}
											{#if isExpanded}
												<ChevronDown class="size-3 shrink-0 text-muted-foreground" />
											{:else}
												<ChevronRight class="size-3 shrink-0 text-muted-foreground" />
											{/if}
										{:else}
											<span class="size-3 shrink-0"></span>
										{/if}

										<span class="font-mono text-xs {isBase ? 'text-fact-base' : 'text-fact-derived'}">
											{nodeLabel(node)}
										</span>

										<Badge
											variant={isBase ? 'outline' : 'secondary'}
											class="text-[0.55rem] px-1 h-3.5 ml-1"
										>{isBase ? 'base' : 'derived'}</Badge>

										{#if node.truncated}
											<Badge variant="outline" class="text-[0.55rem] px-1 h-3.5 text-muted-foreground">truncated</Badge>
										{/if}

										{#if node.confidence != null}
											<span class="ml-1 text-[0.6rem] text-muted-foreground">conf:{node.confidence}</span>
										{/if}
									</button>

									{#if isExpanded && node.derivations}
										{#each node.derivations as derivation, di (di)}
											<div class="ml-4 mt-0.5 border-l border-border/40 pl-2">
												<span class="text-[0.6rem] text-muted-foreground">
													via rule #{derivation.rule_index}: <span class="font-mono">{derivation.rule_head}</span>
												</span>
												{#each derivation.sources as source (source.id)}
													{@render proofNode(source, depth + 1)}
												{/each}
											</div>
										{/each}
									{/if}

									{#if isBase && node.source}
										<span class="ml-6 text-[0.6rem] text-muted-foreground">
											asserted by: {node.source}
										</span>
									{/if}
								</div>
							{/snippet}

							{@render proofNode(explainResult.tree, 0)}
						</div>
					</div>
				{/if}
			</div>
		</div>
	</div>

	<!-- Derived relations reference -->
	{#if derivedRelations.length > 0}
		<div class="flex flex-col gap-2">
			<h2 class="text-sm font-medium text-muted-foreground">Derived relations</h2>
			<div class="flex flex-wrap gap-2">
				{#each derivedRelations as rel (rel.name)}
					<div class="rounded-md border border-border/60 px-3 py-1.5 text-sm">
						<span class="font-mono text-fact-derived">{rel.name}</span>
						<span class="text-muted-foreground">/{rel.arity}</span>
						<span class="ml-1 text-xs text-muted-foreground">{rel.cardinality} facts</span>
					</div>
				{/each}
			</div>
		</div>
	{/if}
</div>
