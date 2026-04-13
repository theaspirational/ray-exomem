<script lang="ts">
	import { browser } from '$app/environment';
	import { SvelteSet } from 'svelte/reactivity';
	import {
		Check,
		CircleAlert,
		Filter,
		Loader2,
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
		formatAssertFactLine,
		parseFactsFromExport,
		retractFact,
		schemaToFacts,
		updateFact
	} from '$lib/exomem.svelte';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import type { ExomemSchemaResponse, FactEntry } from '$lib/types';

	let { exomPath }: { exomPath: string } = $props();

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
	let newFactId = $state('');
	let newPredicate = $state('');
	let newValue = $state('');
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
		const core = f.factId
			? `${f.factId}:${f.predicate}:${f.terms.join(',')}`
			: `${f.predicate}(${f.terms.join(',')})`;
		return `${core}${f.validFrom ? `@[${f.validFrom},${f.validTo ?? 'inf'}]` : ''}`;
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
		editRawText = formatAssertFactLine(fact, exomPath);
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
		exomPath;
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
			const exom = exomPath;
			const [exportText, schemaRes] = await Promise.all([
				exportBackupText(exom, signal),
				fetchExomemSchema(exom, signal)
			]);
			if (seq !== loadSeq) return;

			schema = schemaRes;
			const builtinViewNames = new Set(schemaRes.ontology?.builtin_views.map((view) => view.name) ?? []);

			const baseFacts = parseFactsFromExport(exportText);
			const schemaFacts = schemaToFacts(schemaRes).filter(
				(f) => f.kind === 'derived' && !builtinViewNames.has(f.predicate)
			);
			const baseKeys = new Set(baseFacts.map(factKey));
			const merged = [...baseFacts];
			for (const sf of schemaFacts) {
				if (!baseKeys.has(factKey(sf))) merged.push(sf);
			}

			if (seq !== loadSeq) return;
			facts = merged;
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

	function handleDeleteFact(fact: FactEntry) {
		if (fact.kind === 'derived' || !fact.factId) {
			errorMessage = 'Only base facts with a server fact id can be deleted.';
			deleteConfirmKey = null;
			return;
		}
		const key = factKey(fact);
		actorPrompt.run(async () => {
			deleting = key;
			try {
				await retractFact(fact.factId!, exomPath);
				facts = facts.filter((f) => factKey(f) !== key);
				selectedIds.delete(key);
				deleteConfirmKey = null;
			} catch (error) {
				errorMessage = error instanceof Error ? error.message : 'Delete failed.';
			} finally {
				deleting = null;
			}
		});
	}

	function handleSaveEdit() {
		if (!editFact) return;
		const draft = parseDraftFact(editRawText);
		if (!draft.fact) {
			editError = draft.error;
			return;
		}
		const oldFact = editFact;
		actorPrompt.run(async () => {
			editing = true;
			editError = null;
			try {
				await updateFact(
					{
						factId: oldFact.factId,
						predicate: oldFact.predicate,
						terms: oldFact.terms,
					},
					{
						predicate: draft.fact!.predicate,
						terms: draft.fact!.terms,
						validFrom: draft.fact!.validFrom ?? undefined,
						validTo: draft.fact!.validTo ?? undefined,
					},
					exomPath
				);
				facts = facts
					.filter((f) => factKey(f) !== factKey(oldFact))
					.concat({
						...draft.fact!,
						kind: draft.fact!.kind
					});
				cancelEditFact();
				await loadFacts();
			} catch (error) {
				editError = error instanceof Error ? error.message : 'Edit failed.';
			} finally {
				editing = false;
			}
		});
	}

	function handleDeleteSelected() {
		const toDelete = facts.filter((f) => selectedIds.has(factKey(f)));
		actorPrompt.run(async () => {
			deleting = '__bulk__';
			try {
				await Promise.all(
					toDelete.map((f) => {
						if (f.kind === 'derived' || !f.factId) {
							throw new Error('Cannot delete derived facts in bulk');
						}
						return retractFact(f.factId, exomPath);
					})
				);
				const deletedKeys = new Set(toDelete.map(factKey));
				facts = facts.filter((f) => !deletedKeys.has(factKey(f)));
				selectedIds.clear();
			} catch (error) {
				errorMessage = error instanceof Error ? error.message : 'Bulk delete failed.';
			} finally {
				deleting = null;
			}
		});
	}

	function handleAddFact() {
		if (!newPredicate.trim() || !newValue.trim()) return;
		actorPrompt.run(async () => {
			submitting = true;
			errorMessage = null;

			try {
				const options: { factId?: string; validFrom?: string; validTo?: string } = {};
				if (newIntervalFrom) options.validFrom = newIntervalFrom;
				if (newIntervalTo) options.validTo = newIntervalTo;
				if (newFactId.trim()) options.factId = newFactId.trim();
				await assertFact(newPredicate.trim(), [newValue], options, exomPath);

				// Reset form
				newFactId = '';
				newPredicate = '';
				newValue = '';
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
		});
	}

</script>

<div class="flex flex-col gap-4">
	<!-- Toolbar -->
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-end">
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

	{#if schema}
		<div class="grid gap-3 sm:grid-cols-3">
			<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
				<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">User predicates</p>
				<p class="mt-1 text-lg font-semibold">{schema.ontology?.user_predicates.length ?? 0}</p>
				<p class="mt-1 text-xs text-muted-foreground">Directly asserted predicates visible in exports and CRUD flows.</p>
			</div>
			<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
				<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">Built-in views</p>
				<p class="mt-1 text-lg font-semibold">{schema.ontology?.builtin_views.length ?? 0}</p>
				<p class="mt-1 text-xs text-muted-foreground">System-derived views like <span class="font-mono">fact-row</span> and <span class="font-mono">tx-row</span>.</p>
			</div>
			<div class="rounded-lg border border-border/60 bg-card px-4 py-3">
				<p class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">System attrs</p>
				<p class="mt-1 text-lg font-semibold">{schema.ontology?.system_attributes.length ?? 0}</p>
				<p class="mt-1 text-xs text-muted-foreground">Queryable metadata such as provenance, tx actor, valid-time, and branch state.</p>
			</div>
		</div>
	{/if}

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
			<div class="grid gap-3 sm:grid-cols-[1fr_1fr]">
				<div>
					<label for="new-fact-id" class="mb-1 block text-xs text-muted-foreground">Fact ID (optional but recommended)</label>
					<Input
						id="new-fact-id"
						placeholder="user/editor"
						class="font-mono text-sm"
						value={newFactId}
						oninput={(e: Event) => { newFactId = (e.target as HTMLInputElement).value; }}
					/>
				</div>
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
			</div>

			<div class="mt-3">
				<label for="new-value" class="mb-1 block text-xs text-muted-foreground">Value</label>
				<Input
					id="new-value"
					placeholder="vim"
					class="font-mono text-sm"
					value={newValue}
					oninput={(e: Event) => { newValue = (e.target as HTMLInputElement).value; }}
				/>
				<p class="mt-1 text-xs text-muted-foreground">
					The durable write model is <span class="font-mono">fact_id + predicate + value</span>. Use a stable fact ID when the same claim will be revised later.
				</p>
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
					<Button size="sm" onclick={handleAddFact} disabled={submitting || !newPredicate.trim() || !newValue.trim()}>
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
								{formatAssertFactLine(editDraft.fact, exomPath)}
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
						<th class="w-28 px-3 py-2 text-left text-xs font-medium text-muted-foreground">Branch</th>
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
							<td class="px-3 py-1.5">
								{#if fact.branchRole && fact.branchRole !== 'local'}
									<Badge
										variant="outline"
										class="h-3.5 px-1 text-[0.55rem] capitalize text-muted-foreground"
									>
										{fact.branchRole}
									</Badge>
								{:else}
									<span class="text-muted-foreground/40">&mdash;</span>
								{/if}
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
