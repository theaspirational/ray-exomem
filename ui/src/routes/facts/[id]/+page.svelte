<script lang="ts">
	import { browser } from '$app/environment';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import {
		ArrowLeft,
		ArrowRightSquare,
		Check,
		CircleAlert,
		Clock3,
		Copy,
		Loader2,
		Route
	} from '@lucide/svelte';

	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import {
		fetchExomemStatus,
		fetchFactDetail,
		fetchTxRows,
		type TxViewRow
	} from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';
	import type { FactDetail } from '$lib/types';

	type FactTouchRow = {
		eventId: string;
		eventType: string;
		tx: TxViewRow | null;
	};

	let detail = $state<FactDetail | null>(null);
	let txRows = $state<TxViewRow[]>([]);
	let currentBranch = $state('main');
	let loading = $state(true);
	let loadError = $state<string | null>(null);
	let copiedSnippet = $state<string | null>(null);
	let copyTimer: ReturnType<typeof setTimeout> | null = null;

	const factId = $derived(page.params.id ?? '');
	const tupleValue = $derived(
		Array.isArray(detail?.fact?.tuple) && detail.fact.tuple.length >= 3
			? String(detail.fact.tuple[2] ?? '')
			: ''
	);
	const validityStart = $derived(detail?.fact.interval?.start ?? '—');
	const validityEnd = $derived(detail?.fact.interval?.end ?? '—');
	const txIndex = $derived.by(() => {
		const index: Record<string, TxViewRow> = {};
		for (const row of txRows) {
			index[row.tx] = row;
			index[row.id] = row;
			index[`tx/${row.id}`] = row;
			index[`tx${row.id}`] = row;
		}
		return index;
	});
	const touchRows = $derived.by((): FactTouchRow[] =>
		(detail?.touch_history ?? []).map((event) => ({
			eventId: event.event_id,
			eventType: event.event_type,
			tx: txIndex[event.event_id] ?? null
		}))
	);
	const metadataRows = $derived.by(() => {
		if (!detail?.metadata) return [];
		return [
			{ attribute: 'fact/predicate', value: detail.metadata.predicate },
			{ attribute: 'fact/value', value: detail.metadata.value },
			{ attribute: 'fact/confidence', value: String(detail.metadata.confidence) },
			{ attribute: 'fact/provenance', value: detail.metadata.provenance },
			{ attribute: 'fact/valid_from', value: detail.metadata.valid_from },
			{ attribute: 'fact/valid_to', value: detail.metadata.valid_to ?? 'open' },
			{ attribute: 'fact/created_by', value: detail.metadata.created_by },
			{ attribute: 'fact/superseded_by', value: detail.metadata.superseded_by ?? '—' },
			{ attribute: 'fact/revoked_by', value: detail.metadata.revoked_by ?? '—' }
		];
	});
	const createdBy = $derived(detail?.metadata?.created_by ?? null);
	const supersededBy = $derived(detail?.metadata?.superseded_by ?? null);
	const revokedBy = $derived(detail?.metadata?.revoked_by ?? null);
	const createdTxRow = $derived(createdBy ? txIndex[createdBy] ?? null : null);
	const supersededTxRow = $derived(supersededBy ? txIndex[supersededBy] ?? null : null);
	const revokedTxRow = $derived(revokedBy ? txIndex[revokedBy] ?? null : null);

	function factAttrsQuery(): string {
		return `;; query template for fact ${factId.replace(/"/g, '\\"')}\n;; start from this relation shape, then refine the where-clause as needed\n(query ${app.selectedExom} (find ?fact ?a ?v) (where (fact-row ?fact ?pred ?value) (?fact ?a ?v)))`;
	}

	function factLineageQuery(): string {
		return `;; lineage query template for fact ${factId.replace(/"/g, '\\"')}\n;; start from this relation shape, then add the filter terms you need\n(query ${app.selectedExom} (find ?fact ?pred ?value ?prov ?tx ?actor ?when ?branch) (where (fact-with-tx ?fact ?pred ?value ?prov ?tx ?actor ?when) (?tx 'tx/branch ?branch)))`;
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

	function txSummary(row: TxViewRow | null): string {
		if (!row) return 'tx row unavailable';
		return `${row.tx} · ${row.actor} · ${row.action} · ${row.when}`;
	}

	async function loadFactDetail() {
		if (!factId) {
			loadError = 'Missing fact id.';
			detail = null;
			txRows = [];
			loading = false;
			return;
		}
		loading = true;
		loadError = null;
		try {
			const [fact, tx, status] = await Promise.all([
				fetchFactDetail(factId, app.selectedExom),
				fetchTxRows(app.selectedExom),
				fetchExomemStatus(app.selectedExom)
			]);
			detail = fact;
			txRows = tx;
			currentBranch = status.current_branch ?? 'main';
		} catch (error) {
			loadError = error instanceof Error ? error.message : 'Failed to load fact detail.';
			detail = null;
			txRows = [];
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (!browser) return;
		app.selectedExom;
		page.params.id;
		void loadFactDetail();
		return () => {
			if (copyTimer) clearTimeout(copyTimer);
		};
	});
</script>

<div class="flex flex-col gap-4 p-4 sm:p-6 lg:p-8">
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
		<div class="flex items-start gap-3">
			<a
				href={`${base}/facts`}
				class="mt-0.5 inline-flex size-8 items-center justify-center rounded-md border border-border/60 bg-card text-muted-foreground transition-colors hover:text-foreground"
				aria-label="Back to facts"
			>
				<ArrowLeft class="size-4" />
			</a>
			<div>
				<h1 class="text-2xl font-semibold tracking-tight">Fact Detail</h1>
				<p class="text-sm text-muted-foreground">
					Inspect the current snapshot, queryable metadata, and transaction lineage for
					<span class="font-medium text-foreground">{factId}</span>
					in
					<span class="font-medium text-foreground">{app.selectedExom}</span>.
				</p>
			</div>
		</div>
			<div class="flex flex-wrap items-center gap-2">
				<Badge variant="outline" class="h-6 px-2 font-mono text-[0.65rem]">
					branch {currentBranch}
				</Badge>
				<Button variant="outline" size="sm" onclick={() => copySnippet('attrs', factAttrsQuery())}>
				{#if copiedSnippet === 'attrs'}
					<Check data-icon="inline-start" class="size-3.5" />
				{:else}
					<Copy data-icon="inline-start" class="size-3.5" />
				{/if}
					Copy attrs template
				</Button>
				<Button variant="outline" size="sm" onclick={() => openInQuery(factLineageQuery())}>
					<ArrowRightSquare data-icon="inline-start" class="size-3.5" />
					Open lineage template
				</Button>
			</div>
		</div>

	{#if loadError}
		<div
			class="flex gap-3 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
			role="alert"
		>
			<CircleAlert class="mt-0.5 size-4 shrink-0" />
			<div class="flex-1">{loadError}</div>
		</div>
	{:else if loading}
		<div class="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
			<Loader2 class="size-4 animate-spin" />
			Loading fact detail...
		</div>
	{:else if detail}
		<div class="grid gap-4 lg:grid-cols-[minmax(0,1.25fr)_minmax(0,0.95fr)]">
			<section class="rounded-lg border border-border/60 bg-card p-4">
				<div class="flex flex-wrap items-start gap-2">
					<div class="min-w-0 flex-1">
						<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Snapshot</p>
						<h2 class="mt-1 break-all font-mono text-base text-foreground">{detail.fact.id}</h2>
						<p class="mt-2 font-mono text-sm text-foreground/90">
							{detail.fact.predicate}({tupleValue})
						</p>
					</div>
					<Badge
						variant={detail.fact.status === 'active' ? 'outline' : 'destructive'}
						class="h-5 px-2 text-[0.65rem] capitalize"
					>
						{detail.fact.status}
					</Badge>
				</div>

				<div class="mt-4 grid gap-3 sm:grid-cols-2">
					<div class="rounded-md border border-border/50 bg-muted/20 p-3 text-sm">
						<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Validity</p>
						<p class="mt-2 font-mono text-xs text-foreground/90">{validityStart}</p>
						<p class="mt-1 font-mono text-xs text-muted-foreground">to {validityEnd}</p>
					</div>
					<div class="rounded-md border border-border/50 bg-muted/20 p-3 text-sm">
						<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Provenance kind</p>
						<p class="mt-2 font-mono text-xs text-foreground/90">{detail.provenance.type}</p>
						<p class="mt-1 text-xs text-muted-foreground">
							Current tx hooks are queryable via <span class="font-mono">fact/created_by</span> and related attrs.
						</p>
					</div>
				</div>

				<div class="mt-4">
					<div class="mb-2 flex items-center gap-2">
						<Route class="size-3.5 text-primary" />
						<p class="text-sm font-medium">Queryable attributes</p>
					</div>
					<div class="overflow-x-auto rounded-md border border-border/50">
						<table class="w-full text-sm">
							<thead>
								<tr class="border-b border-border/40 bg-muted/30">
									<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">Attribute</th>
									<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">Value</th>
								</tr>
							</thead>
							<tbody class="divide-y divide-border/30">
								{#each metadataRows as row (`${row.attribute}:${row.value}`)}
									<tr>
										<td class="px-3 py-2 font-mono text-xs text-foreground/90">{row.attribute}</td>
										<td class="px-3 py-2 font-mono text-xs text-muted-foreground break-all">{row.value}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				</div>
			</section>

			<div class="flex flex-col gap-4">
				<section class="rounded-lg border border-border/60 bg-card p-4">
					<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Transaction lineage</p>
					<div class="mt-3 space-y-3 text-sm">
						<div class="rounded-md border border-border/50 bg-muted/20 p-3">
							<p class="text-xs font-medium text-foreground">Created by</p>
							<p class="mt-1 font-mono text-xs text-muted-foreground">{createdBy ?? '—'}</p>
							<p class="mt-2 text-xs text-muted-foreground">{txSummary(createdTxRow)}</p>
						</div>
						<div class="rounded-md border border-border/50 bg-muted/20 p-3">
							<p class="text-xs font-medium text-foreground">Superseded by</p>
							<p class="mt-1 font-mono text-xs text-muted-foreground">{supersededBy ?? '—'}</p>
							<p class="mt-2 text-xs text-muted-foreground">{txSummary(supersededTxRow)}</p>
						</div>
						<div class="rounded-md border border-border/50 bg-muted/20 p-3">
							<p class="text-xs font-medium text-foreground">Revoked by</p>
							<p class="mt-1 font-mono text-xs text-muted-foreground">{revokedBy ?? '—'}</p>
							<p class="mt-2 text-xs text-muted-foreground">{txSummary(revokedTxRow)}</p>
						</div>
					</div>
				</section>

				<section class="rounded-lg border border-border/60 bg-card p-4">
					<div class="flex items-center gap-2">
						<Clock3 class="size-3.5 text-primary" />
						<p class="text-sm font-medium">Touch history</p>
					</div>
					<div class="mt-3 space-y-3">
						{#if touchRows.length === 0}
							<p class="text-sm text-muted-foreground">No transaction touches recorded for this fact.</p>
						{:else}
							{#each touchRows as item (`${item.eventId}:${item.eventType}`)}
								<div class="rounded-md border border-border/50 bg-muted/20 p-3">
									<div class="flex flex-wrap items-center gap-2">
										<Badge variant="outline" class="h-5 px-2 text-[0.65rem]">{item.eventType}</Badge>
										<span class="font-mono text-xs text-foreground/90">{item.eventId}</span>
									</div>
									{#if item.tx}
										<div class="mt-2 grid gap-1 text-xs text-muted-foreground sm:grid-cols-2">
											<div><span class="text-foreground/80">actor</span> {item.tx.actor}</div>
											<div><span class="text-foreground/80">branch</span> {item.tx.branch}</div>
											<div><span class="text-foreground/80">action</span> {item.tx.action}</div>
											<div><span class="text-foreground/80">when</span> {item.tx.when}</div>
										</div>
									{:else}
										<p class="mt-2 text-xs text-muted-foreground">No matching <span class="font-mono">tx-row</span> was found for this event.</p>
									{/if}
								</div>
							{/each}
						{/if}
					</div>
				</section>
			</div>
		</div>
	{/if}
</div>
