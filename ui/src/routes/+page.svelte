<script lang="ts">
	import { onMount } from 'svelte';
	import {
		Activity,
		Check,
		CircleAlert,
		Copy,
		Cpu,
		Database,
		GitBranchPlus,
		KeyRound,
		Play,
		Route,
		RefreshCw
	} from '@lucide/svelte';

	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import {
		fetchExomemClusters,
		fetchExomemLogs,
		fetchExomemSchema,
		fetchExomemStatus,
		fetchTxRows,
		triggerEvaluate
	} from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';
	import type {
		ExomemClusterSummary,
		ExomemLoggedEvent,
		ExomemSchemaResponse,
		ExomemStatus,
	} from '$lib/types';
	import type { TxViewRow } from '$lib/exomem.svelte';

	let status = $state<ExomemStatus | null>(null);
	let schema = $state<ExomemSchemaResponse | null>(null);
	let clusters = $state<ExomemClusterSummary[]>([]);
	let logs = $state<ExomemLoggedEvent[]>([]);
	let txRows = $state<TxViewRow[]>([]);
	let loading = $state(true);
	let refreshing = $state(false);
	let errorMessage = $state<string | null>(null);
	let actionBusy = $state(false);
	let copiedView = $state<string | null>(null);
	let copyTimer: ReturnType<typeof setTimeout> | null = null;

	const activityFeed = $derived(
		app.live.events.length > 0 ? app.live.events : logs
	);

	const statusCards = $derived(
		status
			? [
					{
						label: 'Active facts',
						value: status.stats.facts,
						sub: `${status.schema?.user_predicates.length ?? 0} user predicates`,
						icon: Database,
						color: 'text-fact-base'
					},
					{
						label: 'Derived views',
						value: status.schema?.builtin_view_count ?? status.stats.derived_tuples,
						sub: `${status.stats.rules?.count ?? status.stats.directives} stored rules`,
						icon: GitBranchPlus,
						color: 'text-fact-derived'
					},
					{
						label: 'System attrs',
						value: status.schema?.system_attribute_count ?? 0,
						sub: `${status.schema?.coordination_attribute_count ?? 0} coordination attrs`,
						icon: Cpu,
						color: 'text-rule-accent'
					},
					{
						label: 'Symbol entries',
						value: status.stats.sym_entries ?? 0,
						sub: `branch ${status.current_branch ?? 'main'}`,
						icon: KeyRound,
						color: 'text-primary'
					}
				]
			: []
	);

	const topRelations = $derived(
		schema
			? schema.relations
					.slice()
					.sort((a, b) => {
						const bySize = b.cardinality - a.cardinality;
						if (bySize !== 0) return bySize;
						return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
					})
					.slice(0, 8)
			: []
	);

	const sortedClusters = $derived(
		clusters.length > 0
			? clusters.slice().sort((a, b) => {
					const byFacts = b.fact_count - a.fact_count;
					if (byFacts !== 0) return byFacts;
					const byActive = b.active_count - a.active_count;
					if (byActive !== 0) return byActive;
					return a.id.localeCompare(b.id, undefined, { sensitivity: 'base' });
				})
			: []
	);

	const maxCardinality = $derived(
		topRelations.length > 0 ? topRelations[0].cardinality : 1
	);

	const builtinViews = $derived(schema?.ontology?.builtin_views.slice(0, 6) ?? []);
	const userPredicates = $derived(status?.schema?.user_predicates ?? []);
	const recentTxRows = $derived(
		txRows
			.slice()
			.sort((a, b) => {
				const ai = Number.parseInt(a.id.replace(/^tx\//, ''), 10);
				const bi = Number.parseInt(b.id.replace(/^tx\//, ''), 10);
				if (!Number.isNaN(ai) && !Number.isNaN(bi)) return bi - ai;
				return b.when.localeCompare(a.when);
			})
			.slice(0, 12)
	);

	function varsForArity(arity: number): string[] {
		return Array.from({ length: arity }, (_, i) => `?v${i + 1}`);
	}

	function builtinViewQuery(view: NonNullable<ExomemSchemaResponse['ontology']>['builtin_views'][number]): string {
		const vars = varsForArity(view.arity);
		return `(query ${app.selectedExom} (find ${vars.join(' ')}) (where (${view.name} ${vars.join(' ')})))`;
	}

	async function copyBuiltinView(view: NonNullable<ExomemSchemaResponse['ontology']>['builtin_views'][number]) {
		if (!navigator.clipboard) return;
		await navigator.clipboard.writeText(builtinViewQuery(view));
		copiedView = view.name;
		if (copyTimer) clearTimeout(copyTimer);
		copyTimer = setTimeout(() => {
			copiedView = null;
		}, 1600);
	}

	onMount(() => {
		void refreshAll();
		const interval = window.setInterval(() => void refreshAll({ silent: true }), 15_000);
		return () => {
			window.clearInterval(interval);
			if (copyTimer) clearTimeout(copyTimer);
		};
	});

	async function refreshAll({ silent = false }: { silent?: boolean } = {}) {
		if (!silent) refreshing = true;
		errorMessage = null;

		try {
			const exom = app.selectedExom;
			const [s, sc, cl, lg, tx] = await Promise.all([
				fetchExomemStatus(exom),
				fetchExomemSchema(exom),
				fetchExomemClusters(exom),
				fetchExomemLogs(exom),
				fetchTxRows(exom)
			]);
			status = s;
			app.serverUptimeSec = s.server.uptime_sec;
			schema = sc;
			clusters = cl;
			logs = lg;
			txRows = tx;
			app.live.connect();
		} catch (error) {
			app.live.disconnect();
			app.serverUptimeSec = null;
			txRows = [];
			errorMessage =
				error instanceof Error ? error.message : 'Unable to reach Exomem server.';
		} finally {
			loading = false;
			refreshing = false;
		}
	}

	async function handleEvaluate() {
		actionBusy = true;
		try {
			await triggerEvaluate(app.selectedExom);
			await refreshAll();
		} catch (e) {
			errorMessage = e instanceof Error ? e.message : 'Evaluate failed';
		} finally {
			actionBusy = false;
		}
	}

	function eventSummary(event: ExomemLoggedEvent): string {
		const t = event.type.toLowerCase();
		if (t === 'query')
			return `${event.query_text ?? 'Query'} — ${event.tuples_matched ?? 0} tuples`;
		if (t.startsWith('assert'))
			return `${event.predicate ?? event.pattern ?? 'fact'} asserted${event.terms?.length ? ` (${event.terms.join(', ')})` : ''}`;
		if (t.startsWith('retract'))
			return `${event.pattern ?? 'fact'} retracted — ${event.tuples_retracted ?? 0} revoked`;
		if (t === 'evaluate')
			return `Evaluated — ${event.new_derivations ?? 0} new derivations`;
		if (t === 'load')
			return `Loaded ${event.source ?? 'program'} — ${event.facts_added ?? 0} facts`;
		return event.type;
	}

	function eventColor(type: string): string {
		const t = type.toLowerCase();
		if (t === 'query') return 'text-primary';
		if (t.startsWith('assert')) return 'text-fact-base';
		if (t.startsWith('retract')) return 'text-contra';
		if (t === 'evaluate') return 'text-fact-derived';
		if (t === 'load') return 'text-rule-accent';
		return 'text-muted-foreground';
	}

	function formatEventTime(ts: string): string {
		if (!ts) return '';
		const iso = Date.parse(ts);
		if (!Number.isNaN(iso)) {
			return new Date(iso).toLocaleString(undefined, {
				month: 'short',
				day: 'numeric',
				hour: '2-digit',
				minute: '2-digit',
				second: '2-digit'
			});
		}
		const n = Number(ts.replace(/s$/i, '').trim());
		if (!Number.isNaN(n) && n > 1e9 && n < 1e11) {
			return new Date(n * 1000).toLocaleString(undefined, {
				month: 'short',
				day: 'numeric',
				hour: '2-digit',
				minute: '2-digit',
				second: '2-digit'
			});
		}
		return ts;
	}
</script>

<div class="flex flex-col gap-6 p-4 sm:p-6 lg:p-8">
	<!-- Header -->
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight">Dashboard</h1>
			<p class="text-sm text-muted-foreground">
				Exom: <span class="font-medium text-foreground">{app.selectedExom}</span>
			</p>
			<p class="mt-1 text-xs text-muted-foreground">
				API base: <span class="font-mono">{app.baseUrl}</span>
			</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			<Button variant="outline" size="sm" onclick={handleEvaluate} disabled={actionBusy}>
				<Play data-icon="inline-start" class="size-3.5" />
				Evaluate
			</Button>
			<Button variant="outline" size="sm" onclick={() => refreshAll()} disabled={refreshing}>
				<RefreshCw data-icon="inline-start" class="size-3.5 {refreshing ? 'animate-spin' : ''}" />
				Refresh
			</Button>
		</div>
	</div>

	{#if errorMessage}
		<div
			class="flex gap-3 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
			role="alert"
		>
			<CircleAlert class="mt-0.5 size-4 shrink-0" />
			<div>
				<p class="font-medium">Cannot reach Exomem</p>
				<p class="mt-0.5 text-destructive/80">{errorMessage}</p>
			</div>
		</div>
	{/if}

	<!-- KPI cards -->
	<section class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
		{#if statusCards.length > 0}
			{#each statusCards as card (card.label)}
				{@const Icon = card.icon}
				<div class="group flex items-center gap-4 rounded-lg border border-border/60 bg-card px-4 py-3.5 transition-colors hover:border-border">
					<div class="rounded-md bg-muted p-2 {card.color}">
						<Icon class="size-4" />
					</div>
					<div class="flex flex-col">
						<span class="text-2xl font-semibold tabular-nums tracking-tight">{card.value}</span>
						<span class="text-xs text-muted-foreground">{card.label} · {card.sub}</span>
					</div>
				</div>
			{/each}
		{:else}
			{#each Array.from({ length: 2 }) as _, i (i)}
				<div class="flex items-center gap-4 rounded-lg border border-border/60 bg-card px-4 py-3.5">
					<div class="size-9 animate-pulse rounded-md bg-muted"></div>
					<div class="flex flex-col gap-1.5">
						<div class="h-6 w-16 animate-pulse rounded bg-muted"></div>
						<div class="h-3 w-24 animate-pulse rounded bg-muted/60"></div>
					</div>
				</div>
			{/each}
		{/if}
	</section>

	<div class="grid gap-6 xl:grid-cols-[1.1fr_0.9fr]">
		<section class="flex flex-col gap-3 rounded-lg border border-border/60 bg-card p-4">
			<div class="flex items-center justify-between gap-3">
				<div>
					<h2 class="text-sm font-medium text-muted-foreground">Datalog Model</h2>
					<p class="mt-1 text-sm text-foreground">
						The UI is reading the same substrate the daemon exposes: user facts, built-in views, and system attributes.
					</p>
				</div>
				<Route class="size-4 text-muted-foreground" />
			</div>
			<div class="grid gap-3 sm:grid-cols-3">
				<div class="rounded-md border border-border/50 bg-muted/20 p-3">
					<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">User predicates</p>
					<p class="mt-1 text-lg font-semibold">{userPredicates.length}</p>
					<p class="mt-1 text-xs text-muted-foreground">Stable predicates you assert directly.</p>
				</div>
				<div class="rounded-md border border-border/50 bg-muted/20 p-3">
					<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Built-in views</p>
					<p class="mt-1 text-lg font-semibold">{schema?.ontology?.builtin_views.length ?? 0}</p>
					<p class="mt-1 text-xs text-muted-foreground">Derived views like <span class="font-mono">fact-row</span> and <span class="font-mono">tx-row</span>.</p>
				</div>
				<div class="rounded-md border border-border/50 bg-muted/20 p-3">
					<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">System attrs</p>
					<p class="mt-1 text-lg font-semibold">{schema?.ontology?.system_attributes.length ?? 0}</p>
					<p class="mt-1 text-xs text-muted-foreground">Queryable metadata such as <span class="font-mono">fact/provenance</span> and <span class="font-mono">tx/actor</span>.</p>
				</div>
			</div>
		</section>

		<section class="flex flex-col gap-3 rounded-lg border border-border/60 bg-card p-4">
			<div class="flex items-center justify-between">
				<h2 class="text-sm font-medium text-muted-foreground">Built-in Views</h2>
				{#if schema?.ontology}
					<span class="text-xs text-muted-foreground">{schema.ontology.builtin_views.length} total</span>
				{/if}
			</div>
			{#if builtinViews.length > 0}
				<div class="flex flex-wrap gap-2">
					{#each builtinViews as view (view.name)}
						<div class="rounded-md border border-border/50 bg-muted/20 px-2.5 py-2">
							<div class="flex items-start justify-between gap-2">
								<div class="flex items-center gap-2">
									<span class="font-mono text-xs text-foreground">{view.name}</span>
									<Badge variant="outline" class="h-4 px-1.5 text-[0.6rem]">arity {view.arity}</Badge>
								</div>
								<Button
									variant="ghost"
									size="sm"
									class="h-6 px-2 text-[0.65rem]"
									onclick={() => copyBuiltinView(view)}
								>
									{#if copiedView === view.name}
										<Check class="mr-1 size-3" />
										Copied
									{:else}
										<Copy class="mr-1 size-3" />
										Query
									{/if}
								</Button>
							</div>
							<p class="mt-1 max-w-[20rem] text-[0.7rem] leading-relaxed text-muted-foreground">{view.description}</p>
							<div class="mt-2 rounded border border-border/40 bg-background/60 px-2 py-1.5 font-mono text-[10px] leading-relaxed text-muted-foreground">
								{builtinViewQuery(view)}
							</div>
						</div>
					{/each}
				</div>
			{:else}
				<p class="text-sm text-muted-foreground">{loading ? 'Loading built-in views...' : 'No ontology data yet.'}</p>
			{/if}
		</section>
	</div>

	<div class="grid gap-6 xl:grid-cols-[1fr_1.2fr]">
		<!-- Relations breakdown -->
		<section class="flex flex-col gap-3">
			<div class="flex items-center justify-between">
				<h2 class="text-sm font-medium text-muted-foreground">Predicates by size</h2>
				{#if schema}
					<span class="text-xs text-muted-foreground">{schema.summary.relation_count} total</span>
				{/if}
			</div>
			<div class="flex flex-col gap-1.5">
				{#if topRelations.length > 0}
					{#each topRelations as rel (rel.name)}
						<div class="group flex flex-col gap-2 rounded-md px-2 py-1.5 transition-colors hover:bg-muted/40 sm:flex-row sm:items-center">
							<span class="w-full shrink-0 truncate font-mono text-xs sm:w-32" title={rel.name}>{rel.name}</span>
							<div class="flex-1">
								<div
									class="h-2 rounded-full transition-all {rel.kind === 'derived' ? 'bg-fact-derived/40' : 'bg-fact-base/40'}"
									style="width: {Math.max(4, (rel.cardinality / maxCardinality) * 100)}%"
								></div>
							</div>
							<span class="w-10 self-end text-right font-mono text-xs tabular-nums text-muted-foreground sm:self-auto">{rel.cardinality}</span>
							<Badge
								variant={rel.kind === 'derived' ? 'secondary' : 'outline'}
								class="text-[0.6rem] px-1.5 h-4"
							>{rel.kind}</Badge>
						</div>
					{/each}
				{:else if loading}
					{#each Array.from({ length: 5 }) as _, i (i)}
						<div class="flex items-center gap-3 px-2 py-1.5">
							<div class="h-3 w-28 animate-pulse rounded bg-muted"></div>
							<div class="h-2 flex-1 animate-pulse rounded-full bg-muted/60"></div>
						</div>
					{/each}
				{:else}
					<p class="px-2 py-4 text-sm text-muted-foreground">No predicates yet.</p>
				{/if}
			</div>
		</section>

		<!-- Transaction stream -->
		<section class="flex flex-col gap-3">
			<div class="flex items-center justify-between">
				<h2 class="text-sm font-medium text-muted-foreground">Recent Transactions</h2>
				<span class="text-xs text-muted-foreground">{recentTxRows.length} visible rows</span>
			</div>
			<div class="max-h-[28rem] overflow-y-auto rounded-lg border border-border/60 thin-scrollbar">
				{#if recentTxRows.length === 0}
					<div class="flex flex-col items-center justify-center gap-2 px-6 py-12 text-center text-sm text-muted-foreground">
						<Activity class="size-6 opacity-30" />
						<p>{loading ? 'Loading transactions...' : 'No tx rows yet. Facts will appear here as the exom grows.'}</p>
					</div>
				{:else}
					<div class="divide-y divide-border/40">
						{#each recentTxRows as row (row.tx + row.id)}
							<div
								class="flex items-start gap-3 border-l-2 border-transparent px-3 py-2.5 pl-2.5 transition-colors hover:bg-muted/20"
								class:border-l-primary={row.action === 'query'}
								class:border-l-fact-base={row.action.startsWith('assert')}
								class:border-l-contra={row.action.startsWith('retract')}
								class:border-l-fact-derived={row.action === 'merge-branch'}
								class:border-l-rule-accent={row.action.startsWith('branch')}
							>
								<span class="mt-0.5 min-w-[5.5rem] shrink-0 font-mono text-[0.65rem] font-medium uppercase tracking-wide {eventColor(row.action)}">{row.action}</span>
								<div class="flex min-w-0 flex-1 flex-col gap-0.5">
									<p class="text-sm leading-snug">
										<span class="font-mono">{row.tx}</span> by <span class="font-medium">{row.actor}</span> on <span class="font-mono">{row.branch}</span>
									</p>
									<span class="font-mono text-[0.6rem] text-muted-foreground/70">{formatEventTime(row.when)}</span>
								</div>
								<span class="shrink-0 font-mono text-[0.6rem] text-muted-foreground">{row.id}</span>
							</div>
						{/each}
					</div>
				{/if}
			</div>
		</section>
	</div>

	<section class="flex flex-col gap-3">
		<div class="flex items-center justify-between">
			<h2 class="text-sm font-medium text-muted-foreground">Event Stream</h2>
			<span class="text-xs text-muted-foreground">{activityFeed.length} events</span>
		</div>
		<div class="max-h-64 overflow-y-auto rounded-lg border border-border/60 thin-scrollbar">
			{#if activityFeed.length === 0}
				<div class="flex flex-col items-center justify-center gap-2 px-6 py-12 text-center text-sm text-muted-foreground">
					<Activity class="size-6 opacity-30" />
					<p>{loading ? 'Loading events...' : 'No events yet.'}</p>
				</div>
			{:else}
				<div class="divide-y divide-border/40">
					{#each activityFeed.slice(0, 20) as event (event.id)}
						<div class="flex items-start gap-3 px-3 py-2.5 transition-colors hover:bg-muted/20">
							<span class="mt-0.5 min-w-[5.5rem] shrink-0 font-mono text-[0.65rem] font-medium uppercase tracking-wide {eventColor(event.type)}">{event.type}</span>
							<div class="flex min-w-0 flex-1 flex-col gap-0.5">
								<p class="text-sm leading-snug">{eventSummary(event)}</p>
								<span class="font-mono text-[0.6rem] text-muted-foreground/70">{formatEventTime(event.timestamp)}</span>
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</div>
	</section>

	<!-- Clusters summary -->
	{#if sortedClusters.length > 0}
		<section class="flex flex-col gap-3">
			<h2 class="text-sm font-medium text-muted-foreground">Fact clusters</h2>
			<div class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
				{#each sortedClusters.slice(0, 12) as cluster (cluster.id)}
					<div class="rounded-lg border border-border/60 px-3 py-2.5 transition-colors hover:border-border">
						<div class="flex items-center justify-between gap-2">
							<span class="truncate text-sm font-medium">{cluster.label}</span>
							<span class="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[0.6rem] text-muted-foreground">
								{cluster.kind.replace('shared_', '')}
							</span>
						</div>
						<div class="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
							<span>{cluster.fact_count} facts</span>
							<span>{cluster.active_count} active</span>
							{#if cluster.deprecated_count > 0}
								<span class="text-contra">{cluster.deprecated_count} deprecated</span>
							{/if}
						</div>
					</div>
				{/each}
			</div>
		</section>
	{/if}
</div>
