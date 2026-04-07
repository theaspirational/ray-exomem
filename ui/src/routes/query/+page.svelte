<script lang="ts">
	import {
		Play,
		Trash2,
		BookOpen,
		Clock,
		Eye,
		EyeOff,
		ChevronRight,
		Terminal,
		CircleAlert,
		CircleCheck,
		LoaderCircle
	} from '@lucide/svelte';

	import { Button } from '$lib/components/ui/button';
	import { Badge } from '$lib/components/ui/badge';
	import { importBackup, exportBackupText } from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';

	// ---------------------------------------------------------------------------
	// State
	// ---------------------------------------------------------------------------

	let queryText = $state('');
	let executing = $state(false);
	let result = $state<{
		ok: boolean;
		facts_added: number;
		rules_added: number;
		total_tuples: number;
	} | null>(null);
	let error = $state<string | null>(null);
	let history = $state<Array<{ text: string; timestamp: string; success: boolean }>>([]);
	let exomPreview = $state('');
	let showPreview = $state(false);
	let loadingPreview = $state(false);

	const hasQuery = $derived(queryText.trim().length > 0);
	const historyCount = $derived(history.length);

	// ---------------------------------------------------------------------------
	// Example queries
	// ---------------------------------------------------------------------------

	const examples = [
		{
			label: 'Assert a fact',
			text: `(assert-fact ${app.selectedExom} "bob" 'works_at "acme")`
		},
		{
			label: 'Add a rule',
			text: `(rule ${app.selectedExom} (colleague ?x ?y) (works_at ?x ?z) (works_at ?y ?z))`
		},
		{
			label: 'Arithmetic',
			text: '(+ 21 21)'
		}
	];

	// ---------------------------------------------------------------------------
	// Actions
	// ---------------------------------------------------------------------------

	async function execute() {
		const text = queryText.trim();
		if (!text || executing) return;

		executing = true;
		error = null;
		result = null;

		try {
			const res = await importBackup(text, app.selectedExom);
			result = res;
			history = [
				{ text, timestamp: new Date().toLocaleTimeString(), success: true },
				...history
			].slice(0, 50);
			if (showPreview) {
				await refreshPreview();
			}
		} catch (e) {
			const msg = e instanceof Error ? e.message : String(e);
			error = msg;
			history = [
				{ text, timestamp: new Date().toLocaleTimeString(), success: false },
				...history
			].slice(0, 50);
		} finally {
			executing = false;
		}
	}

	async function refreshPreview() {
		loadingPreview = true;
		try {
			exomPreview = await exportBackupText(app.selectedExom);
		} catch (e) {
			exomPreview = `% Error loading preview: ${e instanceof Error ? e.message : String(e)}`;
		} finally {
			loadingPreview = false;
		}
	}

	async function togglePreview() {
		showPreview = !showPreview;
		if (showPreview && !exomPreview) {
			await refreshPreview();
		}
	}

	function loadExample(example: { label: string; text: string }) {
		queryText = example.text;
	}

	function loadFromHistory(item: { text: string }) {
		queryText = item.text;
	}

	function clearHistory() {
		history = [];
	}

	function clearEditor() {
		queryText = '';
		result = null;
		error = null;
	}

	function handleKeydown(event: KeyboardEvent) {
		if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
			event.preventDefault();
			execute();
		}
	}
</script>

<div class="flex h-full flex-col lg:flex-row">
	<!-- Main content -->
	<div class="flex flex-1 flex-col gap-5 overflow-y-auto p-4 sm:p-6 lg:p-8">
		<!-- Header -->
		<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
			<div class="flex items-center gap-3">
				<div class="flex size-8 items-center justify-center rounded-md bg-primary/15 text-primary">
					<Terminal class="size-4" />
				</div>
				<div>
					<h1 class="text-2xl font-semibold tracking-tight">Query Console</h1>
					<p class="text-sm text-muted-foreground">
						Execute Rayfall against <span class="font-medium text-foreground">{app.selectedExom}</span>
					</p>
				</div>
			</div>
			<Button variant="outline" size="sm" onclick={togglePreview}>
				{#if showPreview}
					<EyeOff data-icon="inline-start" class="size-3.5" />
					Hide Snapshot
				{:else}
					<Eye data-icon="inline-start" class="size-3.5" />
					Exom Snapshot
				{/if}
			</Button>
		</div>

		<!-- Editor area -->
		<section class="flex flex-col gap-3">
			<div class="relative">
				<textarea
					class="w-full resize-none rounded-lg border border-border/60 bg-muted/30 px-4 py-3.5 font-mono text-sm leading-relaxed text-foreground placeholder:text-muted-foreground/50 focus:border-primary/50 focus:outline-none focus:ring-1 focus:ring-primary/30"
					rows="10"
					placeholder=";; Rayfall list-style syntax&#10;(assert-fact main &quot;alice&quot; 'role &quot;engineer&quot;)&#10;&#10;;; Add a rule&#10;(rule main (colleague ?x ?y) (role ?x ?r) (role ?y ?r))"
					bind:value={queryText}
					onkeydown={handleKeydown}
					disabled={executing}
					spellcheck="false"
				></textarea>
				{#if executing}
					<div class="absolute right-3 top-3">
						<LoaderCircle class="size-4 animate-spin text-primary" />
					</div>
				{/if}
			</div>

			<!-- Action buttons -->
			<div class="flex flex-wrap items-center gap-2">
				<Button size="sm" onclick={execute} disabled={!hasQuery || executing}>
					<Play data-icon="inline-start" class="size-3.5" />
					Execute
				</Button>
				<Button variant="ghost" size="sm" onclick={clearEditor} disabled={!hasQuery && !result && !error}>
					<Trash2 data-icon="inline-start" class="size-3.5" />
					Clear
				</Button>

				<div class="flex flex-wrap items-center gap-1 sm:ml-auto">
					<span class="text-xs text-muted-foreground">Examples:</span>
					{#each examples as example (example.label)}
						<Button
							variant="outline"
							size="sm"
							class="h-7 text-xs"
							onclick={() => loadExample(example)}
						>
							{example.label}
						</Button>
					{/each}
				</div>
			</div>

			<p class="text-[0.65rem] text-muted-foreground/60">
				Press <kbd class="rounded border border-border/60 bg-muted/50 px-1 py-0.5 font-mono text-[0.6rem]">Ctrl+Enter</kbd> to execute
			</p>
		</section>

		<!-- Results area -->
		{#if result}
			<section class="flex flex-col gap-3">
				<div class="flex items-center gap-2 rounded-lg border border-fact-base/20 bg-fact-base/5 px-4 py-3">
					<CircleCheck class="size-4 shrink-0 text-fact-base" />
					<div class="flex flex-1 flex-wrap items-center gap-x-4 gap-y-1 text-sm">
						<span class="font-medium text-fact-base">Executed successfully</span>
						<div class="flex items-center gap-3 text-xs text-muted-foreground">
							<span>{result.facts_added} facts added</span>
							<span>{result.rules_added} rules added</span>
							<span>{result.total_tuples} total tuples</span>
						</div>
					</div>
				</div>
			</section>
		{/if}

		{#if error}
			<section>
				<div class="flex items-start gap-2 rounded-lg border border-contra/20 bg-contra/5 px-4 py-3">
					<CircleAlert class="mt-0.5 size-4 shrink-0 text-contra" />
					<div class="flex flex-col gap-0.5">
						<span class="text-sm font-medium text-contra">Execution failed</span>
						<span class="text-xs text-contra/80">{error}</span>
					</div>
				</div>
			</section>
		{/if}

		<!-- Exom Preview -->
		{#if showPreview}
			<section class="flex flex-col gap-2">
				<div class="flex items-center justify-between gap-2">
					<h2 class="text-sm font-medium text-muted-foreground">Exom Snapshot</h2>
					<Button variant="ghost" size="sm" class="h-7 text-xs" onclick={refreshPreview} disabled={loadingPreview}>
						{#if loadingPreview}
							<LoaderCircle data-icon="inline-start" class="size-3 animate-spin" />
						{/if}
						Refresh
					</Button>
				</div>
				<div class="max-h-80 overflow-auto rounded-lg border border-border/60 bg-muted/20">
					<pre class="px-4 py-3 font-mono text-xs leading-relaxed text-muted-foreground">{exomPreview || '% Empty exom — no facts asserted yet'}</pre>
				</div>
			</section>
		{/if}
	</div>

	<!-- Sidebar: Query History -->
	<aside class="flex w-full flex-col border-t border-border/60 lg:w-72 lg:border-l lg:border-t-0">
		<div class="flex items-center justify-between gap-2 border-b border-border/60 px-4 py-3">
			<div class="flex items-center gap-2">
				<Clock class="size-3.5 text-muted-foreground" />
				<h2 class="text-sm font-medium">Recent Queries</h2>
				{#if historyCount > 0}
					<Badge variant="secondary" class="h-4 px-1.5 text-[0.6rem]">{historyCount}</Badge>
				{/if}
			</div>
			{#if historyCount > 0}
				<Button variant="ghost" size="sm" class="h-6 text-[0.65rem]" onclick={clearHistory}>
					Clear
				</Button>
			{/if}
		</div>

		<div class="flex-1 overflow-y-auto no-scrollbar">
			{#if history.length === 0}
				<div class="flex flex-col items-center justify-center gap-2 px-6 py-12 text-center">
					<BookOpen class="size-5 text-muted-foreground/30" />
					<p class="text-xs text-muted-foreground/60">Query sessions will appear here as you query the exom</p>
				</div>
			{:else}
				<div class="flex flex-col">
					{#each history as item, i (i)}
						<button
							class="group flex items-start gap-2 border-b border-border/30 px-4 py-2.5 text-left transition-colors hover:bg-muted/30"
							onclick={() => loadFromHistory(item)}
						>
							<ChevronRight class="mt-0.5 size-3 shrink-0 text-muted-foreground/40 transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
							<div class="flex min-w-0 flex-1 flex-col gap-1">
								<pre class="truncate font-mono text-xs text-foreground/80">{item.text.split('\n').filter((l: string) => !l.startsWith(';')).join(' ').slice(0, 60)}</pre>
								<div class="flex items-center gap-2">
									<span class="text-[0.6rem] text-muted-foreground/60">{item.timestamp}</span>
									<span class="size-1.5 rounded-full {item.success ? 'bg-fact-base/60' : 'bg-contra/60'}"></span>
								</div>
							</div>
						</button>
					{/each}
				</div>
			{/if}
		</div>
	</aside>
</div>
