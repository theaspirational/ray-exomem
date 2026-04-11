<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import {
		TreePine,
		Search,
		Loader2,
		CircleAlert,
		ChevronRight,
		ChevronDown,
		Copy,
		ArrowRightSquare,
		Check,
		Route
	} from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Badge } from '$lib/components/ui/badge';
	import { app } from '$lib/stores.svelte';
	import {
		fetchExomemSchema,
		fetchExplain,
		fetchExomemStatus,
		fetchTxRows,
		type ProofTreeNode,
		type ExplainResponse
	} from '$lib/exomem.svelte';
	import type { FactEntry, ExomemSchemaRelation, ExomemSchemaResponse } from '$lib/types';
	import type { TxViewRow } from '$lib/exomem.svelte';

	type BuiltinView = NonNullable<ExomemSchemaResponse['ontology']>['builtin_views'][number];
	type DerivedFactItem = {
		key: string;
		fact: FactEntry;
		category: 'system' | 'authored';
		description: string | null;
		arity: number;
	};

	let schema = $state<ExomemSchemaResponse | null>(null);
	let derivedFacts = $state<DerivedFactItem[]>([]);
	let derivedRelations = $state<ExomemSchemaRelation[]>([]);
	let builtinViews = $state<BuiltinView[]>([]);
	let txRows = $state<TxViewRow[]>([]);
	let currentBranch = $state('main');
	let loading = $state(true);
	let searchQuery = $state('');
	let selectedFact = $state<DerivedFactItem | null>(null);
	let loadError = $state<string | null>(null);

	let explainResult = $state<ExplainResponse | null>(null);
	let explainLoading = $state(false);
	let explainError = $state<string | null>(null);
	let expandedNodes = $state<Set<string>>(new Set());
	let copiedSnippet = $state<string | null>(null);
	let copyTimer: ReturnType<typeof setTimeout> | null = null;

	const builtinViewMap = $derived(
		new Map((schema?.ontology?.builtin_views ?? []).map((view) => [view.name, view]))
	);
	const authoredDerivedRelations = $derived(
		derivedRelations.filter((rel) => !builtinViewMap.has(rel.name))
	);
	const recentTxRows = $derived(
		txRows
			.slice()
			.sort((a, b) => {
				const ai = Number.parseInt(a.id.replace(/^tx\//, ''), 10);
				const bi = Number.parseInt(b.id.replace(/^tx\//, ''), 10);
				if (!Number.isNaN(ai) && !Number.isNaN(bi)) return bi - ai;
				return b.when.localeCompare(a.when);
			})
			.slice(0, 8)
	);
	const filteredFacts = $derived.by(() => {
		const q = searchQuery.trim().toLowerCase();
		if (!q) return derivedFacts;
		return derivedFacts.filter(
			(item) =>
				item.fact.predicate.toLowerCase().includes(q) ||
				item.fact.terms.some((t) => t.toLowerCase().includes(q)) ||
				item.description?.toLowerCase().includes(q)
		);
	});

	function varsForArity(arity: number): string[] {
		return Array.from({ length: Math.max(arity, 1) }, (_, i) => `?v${i + 1}`);
	}

	function predicateQuery(predicate: string, arity: number, note?: string): string {
		const vars = varsForArity(arity);
		const comment = note ? `;; ${note}\n` : '';
		return `${comment}(query ${app.selectedExom} (find ${vars.join(' ')}) (where (${predicate} ${vars.join(' ')})))`;
	}

	function nodeQuery(node: ProofTreeNode): string {
		return predicateQuery(
			node.predicate,
			node.terms.length,
			`support probe for ${node.predicate}(${node.terms.map(String).join(', ')})`
		);
	}

	function relationQuery(name: string, arity: number): string {
		return predicateQuery(name, arity, `inspect derived relation ${name}`);
	}

	function txRowQuery(): string {
		return `(query ${app.selectedExom} (find ?tx ?id ?actor ?action ?when ?branch) (where (tx-row ?tx ?id ?actor ?action ?when ?branch)))`;
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

	async function loadProvenance() {
		loading = true;
		loadError = null;
		selectedFact = null;
		explainResult = null;
		explainError = null;
		expandedNodes = new Set();
		try {
			const [schemaRes, status, tx] = await Promise.all([
				fetchExomemSchema(app.selectedExom),
				fetchExomemStatus(app.selectedExom),
				fetchTxRows(app.selectedExom)
			]);
			schema = schemaRes;
			currentBranch = status.current_branch ?? 'main';
			txRows = tx;
			builtinViews = schemaRes.ontology?.builtin_views ?? [];
			derivedRelations = schemaRes.relations.filter((r) => r.kind === 'derived');

			const builtinMap = new Map((schemaRes.ontology?.builtin_views ?? []).map((view) => [view.name, view]));
			const nextFacts: DerivedFactItem[] = [];
			for (const rel of schemaRes.relations) {
				if (rel.kind !== 'derived' || !rel.sample_tuples?.length) continue;
				const builtin = builtinMap.get(rel.name);
				for (const [index, tuple] of rel.sample_tuples.entries()) {
					nextFacts.push({
						key: `${rel.name}:${tuple.map(String).join('|')}:${index}`,
						fact: {
							predicate: rel.name,
							terms: tuple.map(String),
							kind: 'derived',
							confidence: null,
							source: null
						},
						category: builtin ? 'system' : 'authored',
						description: builtin?.description ?? null,
						arity: rel.arity
					});
				}
			}
			derivedFacts = nextFacts;
		} catch (e) {
			loadError = e instanceof Error ? e.message : 'Failed to load provenance data.';
			txRows = [];
			currentBranch = 'main';
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (!browser) return;
		app.selectedExom;
		void loadProvenance();
		return () => {
			if (copyTimer) clearTimeout(copyTimer);
		};
	});

	async function selectAndExplain(item: DerivedFactItem) {
		selectedFact = item;
		explainResult = null;
		explainError = null;
		explainLoading = true;
		expandedNodes = new Set();

		try {
			const result = await fetchExplain(item.fact.predicate, item.fact.terms, 10, app.selectedExom);
			explainResult = result;
			expandedNodes = new Set([result.tree.id]);
		} catch (e) {
			explainError = e instanceof Error ? e.message : 'Failed to fetch provenance';
		} finally {
			explainLoading = false;
		}
	}

	function toggleNode(nodeId: string) {
		const next = new Set(expandedNodes);
		if (next.has(nodeId)) next.delete(nodeId);
		else next.add(nodeId);
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
			Trace why a derived fact exists, then jump straight into the query console with a predicate-scoped Rayfall draft.
		</p>
	</div>

	<div class="grid gap-3 lg:grid-cols-[1.1fr_0.9fr]">
		<div class="rounded-lg border border-border/60 bg-card p-4">
			<div class="flex items-center justify-between gap-2">
				<div>
					<h2 class="text-sm font-medium text-muted-foreground">Derived fact browser</h2>
					<p class="mt-1 text-sm text-foreground">Sampled derived rows from the current exom schema.</p>
				</div>
				<Badge variant="secondary">{derivedFacts.length}</Badge>
			</div>
			<p class="mt-3 text-xs leading-relaxed text-muted-foreground">
				System views come from the ontology and expose Datalog-native joins like <span class="font-mono">fact-row</span> and <span class="font-mono">tx-row</span>. Authored relations come from project rules.
			</p>
		</div>

		<div class="rounded-lg border border-border/60 bg-card p-4">
			<div class="flex items-center justify-between gap-2">
				<div>
					<h2 class="text-sm font-medium text-muted-foreground">Query handoff</h2>
					<p class="mt-1 text-sm text-foreground">Copy or open a predicate-scoped Rayfall draft from any proof node.</p>
				</div>
				<Route class="size-4 text-muted-foreground" />
			</div>
			<p class="mt-3 text-xs leading-relaxed text-muted-foreground">
				The query snippets do not bypass Datalog. They open the same query console the rest of the UI uses, prefilled with a probe over the selected predicate.
			</p>
			<div class="mt-3 flex flex-wrap gap-2">
				<Badge variant="outline" class="h-5 px-2 text-[0.65rem]">branch {currentBranch}</Badge>
				<Badge variant="outline" class="h-5 px-2 text-[0.65rem]">{recentTxRows.length} tx rows</Badge>
			</div>
		</div>
	</div>

	<div class="flex items-center gap-3">
		<div class="relative flex-1 max-w-md">
			<Search class="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
			<Input class="pl-9" placeholder="Search derived facts or ontology descriptions..." bind:value={searchQuery} />
		</div>
		<Badge variant="outline">{derivedFacts.filter((item) => item.category === 'authored').length} authored</Badge>
		<Badge variant="outline">{derivedFacts.filter((item) => item.category === 'system').length} system</Badge>
	</div>

	<div class="grid gap-6 lg:grid-cols-[1fr_1fr]">
		<div class="flex flex-col gap-2">
			<h2 class="text-sm font-medium text-muted-foreground">Derived facts</h2>
			{#if loading}
				<div class="flex flex-col gap-2">
					{#each Array.from({ length: 5 }) as _, i (i)}
						<div class="h-12 animate-pulse rounded-lg bg-muted/40"></div>
					{/each}
				</div>
			{:else if loadError}
				<div class="rounded-lg border border-contra/30 bg-contra/5 px-4 py-3 text-sm text-contra">
					{loadError}
				</div>
			{:else if filteredFacts.length === 0}
				<div class="flex flex-col items-center gap-2 rounded-lg border border-border/60 px-6 py-12 text-center">
					<TreePine class="size-8 text-muted-foreground/30" />
					<p class="text-sm text-muted-foreground">
						{derivedFacts.length === 0
							? 'No sampled derived facts yet. Add rules and evaluate to generate derivations.'
							: 'No matches for your search.'}
					</p>
				</div>
			{:else}
				<div class="max-h-[60vh] overflow-y-auto rounded-lg border border-border/60 divide-y divide-border/40 no-scrollbar">
					{#each filteredFacts as item (item.key)}
						<button
							class="flex w-full items-start gap-3 px-3 py-2.5 text-left transition-colors hover:bg-muted/30
								{selectedFact?.key === item.key ? 'bg-primary/10 border-l-2 border-l-fact-derived' : ''}"
							onclick={() => selectAndExplain(item)}
						>
							<div class="min-w-0 flex-1">
								<div class="flex flex-wrap items-center gap-2">
									<span class="font-mono text-sm text-fact-derived">{item.fact.predicate}</span>
									<Badge variant={item.category === 'system' ? 'outline' : 'secondary'} class="h-4 px-1.5 text-[10px]">
										{item.category}
									</Badge>
									<Badge variant="outline" class="h-4 px-1.5 text-[10px]">/{item.arity}</Badge>
								</div>
								<div class="mt-1 font-mono text-xs text-muted-foreground">({item.fact.terms.join(', ')})</div>
								{#if item.description}
									<p class="mt-1 text-[0.7rem] leading-relaxed text-muted-foreground">{item.description}</p>
								{/if}
							</div>
						</button>
					{/each}
				</div>
			{/if}
		</div>

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
					{@const selectedQuery = relationQuery(explainResult.predicate, explainResult.terms.length)}
					<div class="p-4">
						<div class="mb-4 rounded-lg border border-fact-derived/30 bg-fact-derived/5 px-4 py-3">
							<div class="flex flex-wrap items-center gap-2">
								<Badge variant="secondary" class="text-[0.6rem] px-1.5 h-4">
									{selectedFact.category === 'system' ? 'system view' : 'authored'}
								</Badge>
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
							<div class="mt-3 flex flex-wrap gap-2">
								<Button
									variant="outline"
									size="sm"
									onclick={() => copySnippet('selected-fact', selectedQuery)}
								>
									{#if copiedSnippet === 'selected-fact'}
										<Check class="mr-1 size-3.5" />
										Copied
									{:else}
										<Copy class="mr-1 size-3.5" />
										Copy query
									{/if}
								</Button>
								<Button
									variant="ghost"
									size="sm"
									onclick={() => openInQuery(selectedQuery)}
								>
									<ArrowRightSquare class="mr-1 size-3.5" />
									Open in Query
								</Button>
							</div>
						</div>

						<div class="text-sm">
							{#snippet proofNode(node: ProofTreeNode, depth: number)}
								{@const isExpanded = expandedNodes.has(node.id)}
								{@const hasChildren = (node.derivations && node.derivations.length > 0) || false}
								{@const isBase = node.kind === 'base'}
								<div class="flex flex-col" style="margin-left: {depth * 16}px">
									<div class="flex items-start gap-2 rounded px-1.5 py-1 transition-colors hover:bg-muted/30">
										<button
											class="flex min-w-0 flex-1 items-center gap-1.5 text-left"
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

											<span class="min-w-0 truncate font-mono text-xs {isBase ? 'text-fact-base' : 'text-fact-derived'}">
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

										<div class="flex items-center gap-1">
											<Button
												variant="ghost"
												size="sm"
												class="h-6 px-2 text-[0.65rem]"
												onclick={() => copySnippet(node.id, nodeQuery(node))}
											>
												{#if copiedSnippet === node.id}
													<Check class="mr-1 size-3" />
													Copied
												{:else}
													<Copy class="mr-1 size-3" />
													Query
												{/if}
											</Button>
											<Button
												variant="ghost"
												size="sm"
												class="h-6 px-2 text-[0.65rem]"
												onclick={() => openInQuery(nodeQuery(node))}
											>
												<ArrowRightSquare class="mr-1 size-3" />
												Open
											</Button>
										</div>
									</div>

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

	{#if schema}
		<div class="grid gap-6 xl:grid-cols-[1fr_1fr]">
			<section class="flex flex-col gap-2">
				<div class="flex items-center justify-between gap-2">
					<h2 class="text-sm font-medium text-muted-foreground">System views</h2>
					<Badge variant="outline">{builtinViews.length}</Badge>
				</div>
				<div class="grid gap-2">
					{#each builtinViews as view (view.name)}
						<div class="rounded-lg border border-border/60 bg-card/40 px-3 py-3">
							<div class="flex items-start justify-between gap-3">
								<div class="min-w-0">
									<div class="flex flex-wrap items-center gap-2">
										<span class="font-mono text-sm text-fact-derived">{view.name}</span>
										<Badge variant="outline" class="h-4 px-1.5 text-[10px]">arity {view.arity}</Badge>
										<Badge variant="outline" class="h-4 px-1.5 text-[10px]">system</Badge>
									</div>
									<p class="mt-1 text-xs leading-relaxed text-muted-foreground">{view.description}</p>
								</div>
								<div class="flex items-center gap-1">
									<Button
										variant="ghost"
										size="sm"
										class="h-7 px-2 text-[0.7rem]"
										onclick={() => copySnippet(`view:${view.name}`, relationQuery(view.name, view.arity))}
									>
										{#if copiedSnippet === `view:${view.name}`}
											<Check class="mr-1 size-3.5" />
											Copied
										{:else}
											<Copy class="mr-1 size-3.5" />
											Copy
										{/if}
									</Button>
									<Button
										variant="ghost"
										size="sm"
										class="h-7 px-2 text-[0.7rem]"
										onclick={() => openInQuery(relationQuery(view.name, view.arity))}
									>
										<ArrowRightSquare class="mr-1 size-3.5" />
										Open
									</Button>
								</div>
							</div>
						</div>
					{/each}
				</div>
			</section>

			<section class="flex flex-col gap-2">
				<div class="flex items-center justify-between gap-2">
					<h2 class="text-sm font-medium text-muted-foreground">Recent tx rows</h2>
					<div class="flex items-center gap-1">
						<Button
							variant="ghost"
							size="sm"
							class="h-7 px-2 text-[0.7rem]"
							onclick={() => copySnippet('tx-row', txRowQuery())}
						>
							{#if copiedSnippet === 'tx-row'}
								<Check class="mr-1 size-3.5" />
								Copied
							{:else}
								<Copy class="mr-1 size-3.5" />
								Copy query
							{/if}
						</Button>
						<Button
							variant="ghost"
							size="sm"
							class="h-7 px-2 text-[0.7rem]"
							onclick={() => openInQuery(txRowQuery())}
						>
							<ArrowRightSquare class="mr-1 size-3.5" />
							Open
						</Button>
					</div>
				</div>
				{#if recentTxRows.length === 0}
					<div class="rounded-lg border border-border/60 bg-card/40 px-4 py-6 text-sm text-muted-foreground">
						No visible tx rows yet.
					</div>
				{:else}
					<div class="grid gap-2">
						{#each recentTxRows as row (row.tx + row.id)}
							<div class="rounded-lg border border-border/60 bg-card/40 px-3 py-3">
								<div class="flex flex-wrap items-center gap-2">
									<span class="font-mono text-sm text-rule-accent">{row.tx}</span>
									<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{row.action}</Badge>
									<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{row.branch}</Badge>
								</div>
								<p class="mt-1 text-xs text-muted-foreground">actor {row.actor} at {row.when}</p>
							</div>
						{/each}
					</div>
				{/if}
			</section>

			<section class="flex flex-col gap-2">
				<div class="flex items-center justify-between gap-2">
					<h2 class="text-sm font-medium text-muted-foreground">Authored derived relations</h2>
					<Badge variant="secondary">{authoredDerivedRelations.length}</Badge>
				</div>
				{#if authoredDerivedRelations.length === 0}
					<div class="rounded-lg border border-border/60 bg-card/40 px-4 py-6 text-sm text-muted-foreground">
						No authored derived relations yet.
					</div>
				{:else}
					<div class="grid gap-2">
						{#each authoredDerivedRelations as rel (rel.name)}
							<div class="rounded-lg border border-border/60 bg-card/40 px-3 py-3">
								<div class="flex items-start justify-between gap-3">
									<div class="min-w-0">
										<div class="flex flex-wrap items-center gap-2">
											<span class="font-mono text-sm text-rule-accent">{rel.name}</span>
											<Badge variant="secondary" class="h-4 px-1.5 text-[10px]">authored</Badge>
											<Badge variant="outline" class="h-4 px-1.5 text-[10px]">/{rel.arity}</Badge>
											{#if rel.cardinality != null}
												<Badge variant="outline" class="h-4 px-1.5 text-[10px]">{rel.cardinality} rows</Badge>
											{/if}
										</div>
										<p class="mt-1 text-xs leading-relaxed text-muted-foreground">
											Project-specific derived relation from authored rules.
										</p>
									</div>
									<div class="flex items-center gap-1">
										<Button
											variant="ghost"
											size="sm"
											class="h-7 px-2 text-[0.7rem]"
											onclick={() => copySnippet(`rel:${rel.name}`, relationQuery(rel.name, rel.arity))}
										>
											{#if copiedSnippet === `rel:${rel.name}`}
												<Check class="mr-1 size-3.5" />
												Copied
											{:else}
												<Copy class="mr-1 size-3.5" />
												Copy
											{/if}
										</Button>
										<Button
											variant="ghost"
											size="sm"
											class="h-7 px-2 text-[0.7rem]"
											onclick={() => openInQuery(relationQuery(rel.name, rel.arity))}
										>
											<ArrowRightSquare class="mr-1 size-3.5" />
											Open
										</Button>
									</div>
								</div>
							</div>
						{/each}
					</div>
				{/if}
			</section>
		</div>
	{/if}
</div>
