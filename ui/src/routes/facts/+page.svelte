<script lang="ts">
	import { browser } from '$app/environment';
	import { SvelteSet } from 'svelte/reactivity';
	import {
		Check,
		CircleAlert,
		Filter,
		Loader2,
		Minus,
		Pencil,
		Plus,
		RefreshCw,
		Search,
		Trash2,
		X
	} from '@lucide/svelte';

	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import {
		assertFact,
		exportBackupText,
		fetchExomemSchema,
		formatFact,
		parseFactsFromExport,
		retractFact,
		schemaToFacts,
		updateFact
	} from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';
	import type { ExomemSchemaResponse, FactEntry } from '$lib/types';

	// ---------------------------------------------------------------------------
	// State
	// ---------------------------------------------------------------------------

	let facts = $state<FactEntry[]>([]);
	let schema = $state<ExomemSchemaResponse | null>(null);
	let loading = $state(true);
	let refreshing = $state(false);
	let errorMessage = $state<string | null>(null);

	// Filters
	let searchQuery = $state('');
	let predicateFilter = $state('');
	let kindFilter = $state<'all' | 'base' | 'derived'>('all');

	// Selection
	let selectedIds = new SvelteSet<string>();

	// Add fact panel
	let showAddPanel = $state(false);
	let newPredicate = $state('');
	let newTerms = $state<string[]>(['']);
	let newIntervalFrom = $state('');
	let newIntervalTo = $state('');
	let submitting = $state(false);
	let deleting = $state<string | null>(null);
	let deleteConfirmKey = $state<string | null>(null);
	let editFact = $state<FactEntry | null>(null);
	let editRawText = $state('');
	let editing = $state(false);
	let editError = $state<string | null>(null);

	/** Cancels the previous in-flight load when a new one starts (avoids stale `finally` leaving `loading` stuck). */
	let loadAbort: AbortController | null = null;
	let loadSeq = 0;

	// ---------------------------------------------------------------------------
	// Derived
	// ---------------------------------------------------------------------------

	function factKey(f: FactEntry): string {
		return `${f.predicate}(${f.terms.join(',')})${f.validFrom ? `@[${f.validFrom},${f.validTo ?? 'inf'}]` : ''}`;
	}

	const predicateNames = $derived(
		[...new Set(facts.map((f) => f.predicate))].sort()
	);

	const filteredFacts = $derived(
		facts.filter((f) => {
			if (kindFilter !== 'all' && f.kind !== kindFilter) return false;
			if (predicateFilter && f.predicate !== predicateFilter) return false;
			if (searchQuery) {
				const q = searchQuery.toLowerCase();
				if (
					!f.predicate.toLowerCase().includes(q) &&
					!f.terms.some((t) => t.toLowerCase().includes(q))
				) {
					return false;
				}
			}
			return true;
		})
	);

	const allVisibleSelected = $derived(
		filteredFacts.length > 0 &&
		filteredFacts.every((f) => selectedIds.has(factKey(f)))
	);

	const selectedCount = $derived(selectedIds.size);
	const editDraft = $derived(parseDraftFact(editRawText));

	function parseDraftFact(rawText: string): { fact: FactEntry | null; error: string | null } {
		const cleaned = rawText.trim();
		if (!cleaned) {
			return { fact: null, error: 'Enter a single fact in Rayfall form, like parent(alice, bob).' };
		}
		const parsed = parseFactsFromExport(`${cleaned}\n`);
		if (parsed.length === 0) {
			return { fact: null, error: 'Enter a single fact in Rayfall form, like parent(alice, bob).' };
		}
		if (parsed.length > 1) {
			return { fact: null, error: 'The editor must contain exactly one fact.' };
		}
		return { fact: parsed[0], error: null };
	}

	function openEditFact(fact: FactEntry) {
		editFact = fact;
		editRawText = formatFact(fact.predicate, fact.terms);
		editError = null;
	}

	function cancelEditFact() {
		editFact = null;
		editRawText = '';
		editError = null;
	}

	// ---------------------------------------------------------------------------
	// Data loading
	// ---------------------------------------------------------------------------

	$effect(() => {
		if (!browser) return;
		app.selectedExom;
		void loadFacts({ silent: true, showTableSpinner: true });
		return () => {
			loadAbort?.abort();
		};
	});

	async function loadFacts(
		{ silent = false, showTableSpinner = false }: { silent?: boolean; showTableSpinner?: boolean } = {}
	) {
		const seq = ++loadSeq;
		loadAbort?.abort();
		const ac = new AbortController();
		loadAbort = ac;
		const signal = ac.signal;

		if (showTableSpinner) loading = true;
		if (!silent) refreshing = true;
		errorMessage = null;

		try {
			const exom = app.selectedExom;
			const [dlText, schemaRes] = await Promise.all([
				exportBackupText(exom, signal),
				fetchExomemSchema(exom, signal)
			]);
			if (seq !== loadSeq) return;

			schema = schemaRes;

			// Parse facts from export, then enrich with kind from schema
			const parsedFacts = parseFactsFromExport(dlText);
			const schemaFacts = schemaToFacts(schemaRes);

			// Build a set of derived predicates for kind enrichment
			const derivedPredicates = new Set(
				schemaRes.relations
					.filter((r) => r.kind === 'derived')
					.map((r) => r.name)
			);

			// Merge: use parsed facts as base, add any schema-only derived facts
			const parsedKeys = new Set(parsedFacts.map(factKey));
			const enriched = parsedFacts.map((f) => ({
				...f,
				kind: derivedPredicates.has(f.predicate) ? 'derived' as const : 'base' as const
			}));

			// Add derived facts from schema that weren't in the export
			for (const sf of schemaFacts) {
				if (sf.kind === 'derived' && !parsedKeys.has(factKey(sf))) {
					enriched.push(sf);
				}
			}

			if (seq !== loadSeq) return;
			facts = enriched;
		} catch (error) {
			if (error instanceof Error && error.name === 'AbortError') return;
			if (seq !== loadSeq) return;
			errorMessage =
				error instanceof Error ? error.message : 'Failed to load facts.';
		} finally {
			if (seq === loadSeq) {
				loading = false;
				refreshing = false;
			}
		}
	}

	// ---------------------------------------------------------------------------
	// Actions
	// ---------------------------------------------------------------------------

	function toggleSelect(key: string) {
		if (selectedIds.has(key)) {
			selectedIds.delete(key);
		} else {
			selectedIds.add(key);
		}
	}

	function toggleSelectAll() {
		if (allVisibleSelected) {
			selectedIds.clear();
		} else {
			for (const f of filteredFacts) {
				selectedIds.add(factKey(f));
			}
		}
	}

	function promptDeleteFact(fact: FactEntry) {
		deleteConfirmKey = factKey(fact);
	}

	function cancelDeleteFact() {
		deleteConfirmKey = null;
	}

	async function handleDeleteFact(fact: FactEntry) {
		const key = factKey(fact);
		deleting = key;
		try {
			await retractFact(fact.predicate, fact.terms, app.selectedExom);
			facts = facts.filter((f) => factKey(f) !== key);
			selectedIds.delete(key);
			deleteConfirmKey = null;
		} catch (error) {
			errorMessage = error instanceof Error ? error.message : 'Delete failed.';
		} finally {
			deleting = null;
		}
	}

	async function handleSaveEdit() {
		if (!editFact) return;
		const draft = parseDraftFact(editRawText);
		if (!draft.fact) {
			editError = draft.error;
			return;
		}
		editing = true;
		editError = null;
		try {
			const oldFact = editFact;
			await updateFact(
				{
					predicate: oldFact.predicate,
					terms: oldFact.terms,
				},
				{
					predicate: draft.fact.predicate,
					terms: draft.fact.terms,
					validFrom: draft.fact.validFrom ?? undefined,
					validTo: draft.fact.validTo ?? undefined,
				},
				app.selectedExom
			);
			facts = facts
				.filter((f) => factKey(f) !== factKey(oldFact))
				.concat({
					...draft.fact,
					kind: draft.fact.kind
				});
			cancelEditFact();
			await loadFacts();
		} catch (error) {
			editError = error instanceof Error ? error.message : 'Edit failed.';
		} finally {
			editing = false;
		}
	}

	async function handleDeleteSelected() {
		const toDelete = facts.filter((f) => selectedIds.has(factKey(f)));
		deleting = '__bulk__';
		try {
			await Promise.all(
				toDelete.map((f) => retractFact(f.predicate, f.terms, app.selectedExom))
			);
			const deletedKeys = new Set(toDelete.map(factKey));
			facts = facts.filter((f) => !deletedKeys.has(factKey(f)));
			selectedIds.clear();
		} catch (error) {
			errorMessage = error instanceof Error ? error.message : 'Bulk delete failed.';
		} finally {
			deleting = null;
		}
	}

	async function handleAddFact() {
		if (!newPredicate.trim() || newTerms.every((t) => !t.trim())) return;
		submitting = true;
		errorMessage = null;

		try {
			const terms = newTerms.map((t) => t.trim()).filter(Boolean);
			const options: { validFrom?: string; validTo?: string } = {};
			if (newIntervalFrom) options.validFrom = newIntervalFrom;
			if (newIntervalTo) options.validTo = newIntervalTo;
			await assertFact(newPredicate.trim(), terms, options, app.selectedExom);

			// Reset form
			newPredicate = '';
			newTerms = [''];
			newIntervalFrom = '';
			newIntervalTo = '';
			showAddPanel = false;

			// Reload
			await loadFacts();
		} catch (error) {
			errorMessage = error instanceof Error ? error.message : 'Failed to add fact.';
		} finally {
			submitting = false;
		}
	}

	function addTermField() {
		newTerms = [...newTerms, ''];
	}

	function removeTermField(index: number) {
		if (newTerms.length <= 1) return;
		newTerms = newTerms.filter((_, i) => i !== index);
	}

	function updateTerm(index: number, value: string) {
		newTerms = newTerms.map((t, i) => (i === index ? value : t));
	}
</script>

<div class="flex flex-col gap-4 p-4 sm:p-6 lg:p-8">
	<!-- Header -->
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
		<div>
			<h1 class="text-2xl font-semibold tracking-tight">Facts</h1>
			<p class="text-sm text-muted-foreground">
				{filteredFacts.length} of {facts.length} facts in
				<span class="font-medium text-foreground">{app.selectedExom}</span>
				— base facts are directly asserted, derived facts are inferred from rules.
			</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			<Button variant="outline" size="sm" onclick={() => loadFacts()} disabled={refreshing}>
				<RefreshCw data-icon="inline-start" class="size-3.5 {refreshing ? 'animate-spin' : ''}" />
				Refresh
			</Button>
			<Button size="sm" onclick={() => { showAddPanel = !showAddPanel; }}>
				{#if showAddPanel}
					<X data-icon="inline-start" class="size-3.5" />
					Cancel
				{:else}
					<Plus data-icon="inline-start" class="size-3.5" />
					Add Fact
				{/if}
			</Button>
		</div>
	</div>

	<!-- Error banner -->
	{#if errorMessage}
		<div
			class="flex gap-3 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
			role="alert"
		>
			<CircleAlert class="mt-0.5 size-4 shrink-0" />
			<div class="flex-1">
				<p>{errorMessage}</p>
			</div>
			<button class="shrink-0 text-destructive/60 hover:text-destructive" onclick={() => { errorMessage = null; }}>
				<X class="size-4" />
			</button>
		</div>
	{/if}

	<!-- Add Fact Panel -->
	{#if showAddPanel}
		<div class="rounded-lg border border-border/60 bg-card p-4">
			<h3 class="mb-3 text-sm font-medium">Assert Fact</h3>
			<div class="grid gap-3 sm:grid-cols-[1fr_2fr]">
				<div>
					<label for="new-predicate" class="mb-1 block text-xs text-muted-foreground">Predicate</label>
					<Input
						id="new-predicate"
						placeholder="relation_name"
						class="font-mono text-sm"
						value={newPredicate}
						oninput={(e: Event) => { newPredicate = (e.target as HTMLInputElement).value; }}
					/>
				</div>
				<div>
					<span class="mb-1 block text-xs text-muted-foreground">Terms</span>
					<div class="flex flex-col gap-1.5">
						{#each newTerms as term, i (i)}
							<div class="flex items-center gap-1.5">
								<Input
									placeholder="term {i + 1}"
									class="font-mono text-sm"
									value={term}
									oninput={(e: Event) => updateTerm(i, (e.target as HTMLInputElement).value)}
								/>
								{#if newTerms.length > 1}
									<Button variant="ghost" size="icon-sm" onclick={() => removeTermField(i)}>
										<Minus class="size-3.5" />
									</Button>
								{/if}
								{#if i === newTerms.length - 1}
									<Button variant="ghost" size="icon-sm" onclick={addTermField}>
										<Plus class="size-3.5" />
									</Button>
								{/if}
							</div>
						{/each}
					</div>
				</div>
			</div>

			<div class="mt-3 grid gap-3 sm:grid-cols-[1fr_1fr_auto]">
				<div>
					<label for="interval-from" class="mb-1 block text-xs text-muted-foreground">Valid from (optional, ISO 8601)</label>
					<Input
						id="interval-from"
						placeholder="2024-01-01"
						class="font-mono text-sm"
						value={newIntervalFrom}
						oninput={(e: Event) => { newIntervalFrom = (e.target as HTMLInputElement).value; }}
					/>
				</div>
				<div>
					<label for="interval-to" class="mb-1 block text-xs text-muted-foreground">Valid to (optional, ISO 8601)</label>
					<Input
						id="interval-to"
						placeholder="2024-12-31"
						class="font-mono text-sm"
						value={newIntervalTo}
						oninput={(e: Event) => { newIntervalTo = (e.target as HTMLInputElement).value; }}
					/>
				</div>
				<div class="flex items-end gap-2">
					<Button size="sm" onclick={handleAddFact} disabled={submitting || !newPredicate.trim()}>
						{#if submitting}
							<Loader2 data-icon="inline-start" class="size-3.5 animate-spin" />
							Adding...
						{:else}
							<Plus data-icon="inline-start" class="size-3.5" />
							Add
						{/if}
					</Button>
				</div>
			</div>
		</div>
	{/if}

	{#if editFact}
		<div class="rounded-lg border border-border/60 bg-card p-4">
			<div class="mb-3 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
				<div>
					<h3 class="text-sm font-medium">Edit fact</h3>
					<p class="text-xs text-muted-foreground">
						Change the raw Rayfall text, then preview the parsed fact before saving.
					</p>
				</div>
				<Button variant="ghost" size="sm" onclick={cancelEditFact} disabled={editing}>
					<X class="mr-1 size-3.5" />
					Close
				</Button>
			</div>

			<div class="grid gap-3 lg:grid-cols-[1.4fr_1fr]">
				<div>
					<label for="edit-raw-rayfall" class="mb-1 block text-xs text-muted-foreground">Raw Rayfall</label>
					<textarea
						id="edit-raw-rayfall"
						class="min-h-28 w-full rounded-md border border-border/60 bg-background p-3 font-mono text-sm text-foreground outline-none focus:ring-1 focus:ring-ring"
						bind:value={editRawText}
						disabled={editing}
					></textarea>
				</div>

				<div class="rounded-md border border-border/50 bg-muted/20 p-3 text-sm">
					<div class="mb-2 flex items-center gap-2">
						<span class="text-xs font-medium uppercase tracking-wide text-muted-foreground">Preview</span>
						{#if editDraft.fact}
							<Badge variant="outline" class="h-5 px-2 text-[10px]">valid</Badge>
						{:else}
							<Badge variant="destructive" class="h-5 px-2 text-[10px]">invalid</Badge>
						{/if}
					</div>

					{#if editDraft.fact}
						<div class="space-y-2 font-mono text-xs text-foreground/90">
							<div><span class="text-muted-foreground">Predicate:</span> {editDraft.fact.predicate}</div>
							<div><span class="text-muted-foreground">Terms:</span> {editDraft.fact.terms.join(', ')}</div>
							<div>
								<span class="text-muted-foreground">Validity:</span>
								{editDraft.fact.validFrom ? `${editDraft.fact.validFrom} → ${editDraft.fact.validTo ?? 'open'}` : '—'}
							</div>
							<div class="rounded border border-border/40 bg-background px-2 py-1.5 text-[11px] text-muted-foreground">
								{formatFact(editDraft.fact.predicate, editDraft.fact.terms)}
							</div>
						</div>
					{:else}
						<p class="text-xs text-contra">{editDraft.error}</p>
					{/if}
				</div>
			</div>

			{#if editError}
				<p class="mt-3 text-xs text-contra">{editError}</p>
			{/if}

			<div class="mt-3 flex flex-wrap items-center gap-2">
				<Button size="sm" onclick={handleSaveEdit} disabled={editing}>
					{#if editing}
						<Loader2 class="mr-1 size-3.5 animate-spin" />
						Saving...
					{:else}
						<Check class="mr-1 size-3.5" />
						Save changes
					{/if}
				</Button>
				<Button variant="outline" size="sm" onclick={cancelEditFact} disabled={editing}>Cancel</Button>
			</div>
		</div>
	{/if}

	<!-- Filter bar -->
	<div class="flex flex-wrap items-center gap-2">
		<div class="relative flex-1 min-w-[200px] max-w-sm">
			<Search class="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
			<Input
				placeholder="Search predicates or terms..."
				class="pl-8 text-sm"
				value={searchQuery}
				oninput={(e: Event) => { searchQuery = (e.target as HTMLInputElement).value; }}
			/>
		</div>

		<div class="flex items-center gap-1.5">
			<Filter class="size-3.5 text-muted-foreground" />
			<select
				class="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground outline-none focus:ring-1 focus:ring-ring"
				value={predicateFilter}
				onchange={(e: Event) => { predicateFilter = (e.target as HTMLSelectElement).value; }}
			>
				<option value="">All predicates</option>
				{#each predicateNames as name (name)}
					<option value={name}>{name}</option>
				{/each}
			</select>
		</div>

		<div class="flex items-center rounded-md border border-input">
			{#each ['all', 'base', 'derived'] as kind (kind)}
				<button
					class="px-2.5 py-1 text-xs font-medium transition-colors first:rounded-l-md last:rounded-r-md {kindFilter === kind ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground hover:bg-muted/50'}"
					onclick={() => { kindFilter = kind as 'all' | 'base' | 'derived'; }}
				>
					{kind.charAt(0).toUpperCase() + kind.slice(1)}
				</button>
			{/each}
		</div>
	</div>

	<!-- Bulk action bar -->
	{#if selectedCount > 0}
		<div class="flex items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-2">
			<span class="text-sm font-medium">{selectedCount} selected</span>
			<Button
				variant="destructive"
				size="xs"
				onclick={handleDeleteSelected}
				disabled={deleting === '__bulk__'}
			>
				{#if deleting === '__bulk__'}
					<Loader2 data-icon="inline-start" class="size-3 animate-spin" />
					Deleting...
				{:else}
					<Trash2 data-icon="inline-start" class="size-3" />
					Delete selected
				{/if}
			</Button>
			<button
				class="ml-auto text-xs text-muted-foreground hover:text-foreground"
				onclick={() => { selectedIds.clear(); }}
			>
				Clear selection
			</button>
		</div>
	{/if}

	<!-- Facts table -->
	{#if loading}
		<div class="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
			<Loader2 class="size-4 animate-spin" />
			Loading facts...
		</div>
	{:else if filteredFacts.length === 0}
		<div class="flex flex-col items-center justify-center gap-2 py-16 text-center text-sm text-muted-foreground">
			{#if facts.length === 0}
				<p>No facts in this exom yet.</p>
				<p class="text-xs">Use "Add Fact" to assert a new fact.</p>
			{:else}
				<p>No facts match the current filters.</p>
			{/if}
		</div>
	{:else}
		<div class="overflow-x-auto rounded-lg border border-border/60">
			<table class="w-full text-sm">
				<thead>
					<tr class="border-b border-border/40 bg-muted/30">
						<th class="w-10 px-3 py-2">
							<input
								type="checkbox"
								class="size-3.5 rounded border-muted-foreground/40 accent-primary"
								checked={allVisibleSelected}
								onchange={toggleSelectAll}
							/>
						</th>
						<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">Predicate</th>
						<th class="px-3 py-2 text-left text-xs font-medium text-muted-foreground">Terms</th>
						<th class="w-24 px-3 py-2 text-left text-xs font-medium text-muted-foreground">Kind</th>
						<th class="w-44 px-3 py-2 text-left text-xs font-medium text-muted-foreground">Validity</th>
						<th class="w-12 px-3 py-2"></th>
					</tr>
				</thead>
				<tbody class="divide-y divide-border/30">
					{#each filteredFacts as fact (factKey(fact))}
						{@const key = factKey(fact)}
						{@const isSelected = selectedIds.has(key)}
						<tr class="group transition-colors hover:bg-muted/20 {isSelected ? 'bg-primary/5' : ''}">
							<td class="px-3 py-1.5">
								<input
									type="checkbox"
									class="size-3.5 rounded border-muted-foreground/40 accent-primary"
									checked={isSelected}
									onchange={() => toggleSelect(key)}
								/>
							</td>
							<td class="px-3 py-1.5 font-mono text-xs {fact.kind === 'derived' ? 'text-fact-derived' : 'text-fact-base'}">
								{fact.predicate}
							</td>
							<td class="max-w-md px-3 py-1.5">
								<span class="font-mono text-xs text-foreground/80">
									{fact.terms.join(', ')}
								</span>
							</td>
							<td class="px-3 py-1.5">
								<Badge
									variant={fact.kind === 'derived' ? 'secondary' : 'outline'}
									class="text-[0.6rem] px-1.5 h-4"
								>{fact.kind}</Badge>
							</td>
							<td class="px-3 py-1.5 font-mono text-xs text-muted-foreground">
								{#if fact.validFrom}
									{fact.validFrom} &rarr; {fact.validTo ?? 'open'}
								{:else}
									<span class="text-muted-foreground/40">&mdash;</span>
								{/if}
							</td>
					<td class="px-3 py-1.5">
						{#if fact.kind === 'base'}
							<div class="flex items-center justify-end gap-1">
								<button
									class="rounded p-1 text-muted-foreground hover:bg-primary/10 hover:text-primary"
									onclick={() => openEditFact(fact)}
									title="Edit fact"
								>
									<Pencil class="size-3.5" />
								</button>
								{#if deleteConfirmKey === key}
									<button
										class="rounded p-1 text-muted-foreground hover:bg-muted/70 hover:text-foreground"
										onclick={cancelDeleteFact}
										title="Cancel delete"
									>
										<X class="size-3.5" />
									</button>
									<button
										class="rounded p-1 text-contra hover:bg-contra/10"
										onclick={() => handleDeleteFact(fact)}
										disabled={deleting === key}
										title="Confirm delete"
									>
										{#if deleting === key}
											<Loader2 class="size-3.5 animate-spin" />
										{:else}
											<Check class="size-3.5" />
										{/if}
									</button>
								{:else}
									<button
										class="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-contra"
										onclick={() => promptDeleteFact(fact)}
										disabled={deleting === key}
										title="Delete fact"
									>
										{#if deleting === key}
											<Loader2 class="size-3.5 animate-spin" />
										{:else}
											<Trash2 class="size-3.5" />
										{/if}
									</button>
								{/if}
							</div>
						{/if}
					</td>
</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</div>
