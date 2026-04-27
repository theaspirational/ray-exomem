<script lang="ts">
	import { browser } from '$app/environment';
	import { Check, Copy, LoaderCircle, Pencil, Plus, Search, Trash2, X } from '@lucide/svelte';

	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { addRule, exportBackupText, importBackup, parseRulesFromExport } from '$lib/exomem.svelte';
	import { toCli } from '$lib/path.svelte';
	import type { RuleEntry } from '$lib/types';

	let { exomPath }: { exomPath: string } = $props();

	let rules = $state<RuleEntry[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let searchQuery = $state('');
	let showAddForm = $state(false);
	let newRuleText = $state('');
	let submitting = $state(false);
	let submitResult = $state<string | null>(null);
	let submitError = $state<string | null>(null);
	let deleteConfirmIndex = $state<number | null>(null);
	let editingRule = $state<RuleEntry | null>(null);
	let editRawText = $state('');
	let editError = $state<string | null>(null);
	let editing = $state(false);
	let deleting = $state(false);
	let copiedSnippet = $state<string | null>(null);
	let copyTimer: ReturnType<typeof setTimeout> | null = null;

	const filteredRules = $derived(
		searchQuery.trim() === ''
			? rules
			: rules.filter((r) => {
					const q = searchQuery.toLowerCase();
					return (
						r.head_predicate.toLowerCase().includes(q) ||
						r.raw.toLowerCase().includes(q) ||
						r.body_atoms.some((a) => a.toLowerCase().includes(q))
					);
				})
	);
	const editDraft = $derived(parseDraftRule(editRawText));

	const exampleRule = $derived(
		`(rule ${toCli(exomPath)} (active-projects ?p) ((entity/type ?p "project") (project/status ?p "active")))`
	);

	function ruleHeadQuery(rule: RuleEntry): string {
		const vars = Array.from({ length: Math.max(1, rule.body_atoms.length + 1) }, (_, i) => `?v${i + 1}`);
		return `(query ${exomPath} (find ${vars.join(' ')}) (where (${rule.head_predicate} ${vars.join(' ')})))`;
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

	function parseDraftRule(rawText: string): { rule: RuleEntry | null; error: string | null } {
		const cleaned = rawText.trim();
		if (!cleaned) return { rule: null, error: 'Enter a single rule in Rayfall form.' };
		const parsed = parseRulesFromExport(`${cleaned}\n`);
		if (parsed.length !== 1) {
			return { rule: null, error: 'The editor must contain exactly one rule.' };
		}
		return { rule: parsed[0], error: null };
	}

	function openEditRule(rule: RuleEntry) {
		editingRule = rule;
		editRawText = rule.raw;
		editError = null;
	}

	function closeEditRule() {
		editingRule = null;
		editRawText = '';
		editError = null;
	}

	function replaceRuleInText(source: string, oldRaw: string, newRaw?: string): string {
		const lines = source.split('\n');
		const target = oldRaw.trim();
		const idx = lines.findIndex((line) => line.trim() === target);
		if (idx === -1) throw new Error('Could not find the rule in the exported source.');
		if (newRaw === undefined) {
			lines.splice(idx, 1);
		} else {
			lines[idx] = newRaw;
		}
		return lines.join('\n');
	}

	function highlightRule(raw: string): string {
		const separatorIdx = raw.indexOf(':-');
		if (separatorIdx === -1) {
			return highlightAtom(raw);
		}

		const head = raw.slice(0, separatorIdx).trim();
		const body = raw.slice(separatorIdx + 2).trim();

		const headHtml = `<span class="text-rule-accent">${escapeHtml(head)}</span>`;
		const sepHtml = `<span class="text-muted-foreground"> :- </span>`;
		const bodyHtml = highlightBody(body);

		return headHtml + sepHtml + bodyHtml;
	}

	function highlightBody(body: string): string {
		const cleaned = body.replace(/\.\s*$/, '');
		const atoms = splitBodyAtoms(cleaned);

		return (
			atoms
				.map((atom) => {
					const trimmed = atom.trim();
					if (!trimmed) return '';

					if (trimmed.startsWith('!') || trimmed.startsWith('\\+')) {
						const rest = trimmed.startsWith('\\+') ? trimmed.slice(2) : trimmed.slice(1);
						return `<span class="text-contra">!</span>${highlightAtom(rest.trim())}`;
					}

					return highlightAtom(trimmed);
				})
				.join('<span class="text-muted-foreground">, </span>') +
			'<span class="text-muted-foreground">.</span>'
		);
	}

	function splitBodyAtoms(body: string): string[] {
		const atoms: string[] = [];
		let depth = 0;
		let current = '';
		for (const ch of body) {
			if (ch === '(') depth++;
			else if (ch === ')') depth--;
			if (ch === ',' && depth === 0) {
				atoms.push(current);
				current = '';
			} else {
				current += ch;
			}
		}
		if (current.trim()) atoms.push(current);
		return atoms;
	}

	function highlightAtom(atom: string): string {
		const match = atom.match(/^([a-z_][a-z0-9_]*)\((.+)\)$/i);
		if (!match) return escapeHtml(atom);

		const [, pred, args] = match;
		const highlightedArgs = args
			.split(',')
			.map((arg) => {
				const a = arg.trim();
				if (/^[A-Z]/.test(a)) {
					return `<span class="text-foreground">${escapeHtml(a)}</span>`;
				}
				return `<span class="text-muted-foreground">${escapeHtml(a)}</span>`;
			})
			.join('<span class="text-muted-foreground">, </span>');

		return (
			`<span class="text-fact-base">${escapeHtml(pred)}</span>` +
			`<span class="text-muted-foreground">(</span>` +
			highlightedArgs +
			`<span class="text-muted-foreground">)</span>`
		);
	}

	function escapeHtml(s: string): string {
		return s
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;');
	}

	function extractPredicateFromAtom(atom: string): string {
		const match = atom.match(/^!?\s*\\?\+?\s*([a-z_][a-z0-9_]*)/i);
		return match ? match[1] : atom;
	}

	async function loadRules() {
		loading = true;
		error = null;
		try {
			const dlText = await exportBackupText(exomPath);
			rules = parseRulesFromExport(dlText);
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	async function handleAddRule() {
		if (!newRuleText.trim()) return;
		submitting = true;
		submitResult = null;
		submitError = null;
		try {
			const result = await addRule(newRuleText.trim(), exomPath);
			submitResult = result.ok ? 'Rule sent to eval successfully.' : 'Eval returned not ok.';
			newRuleText = '';
			await loadRules();
		} catch (e) {
			submitError = e instanceof Error ? e.message : String(e);
		} finally {
			submitting = false;
		}
	}

	async function handleSaveEditRule() {
		if (!editingRule) return;
		const draft = editDraft;
		if (!draft.rule) {
			editError = draft.error;
			return;
		}
		editing = true;
		editError = null;
		try {
			const source = await exportBackupText(exomPath);
			const updated = replaceRuleInText(source, editingRule.raw, draft.rule.raw);
			await importBackup(updated, exomPath);
			await loadRules();
			closeEditRule();
		} catch (e) {
			editError = e instanceof Error ? e.message : String(e);
		} finally {
			editing = false;
		}
	}

	function promptDeleteRule(rule: RuleEntry) {
		deleteConfirmIndex = rule.index;
	}

	function cancelDeleteRule() {
		deleteConfirmIndex = null;
	}

	async function handleDeleteRule(rule: RuleEntry) {
		deleting = true;
		try {
			const source = await exportBackupText(exomPath);
			const updated = replaceRuleInText(source, rule.raw);
			await importBackup(updated, exomPath);
			await loadRules();
			deleteConfirmIndex = null;
		} catch (e) {
			submitError = e instanceof Error ? e.message : String(e);
		} finally {
			deleting = false;
		}
	}

	function closeAddForm() {
		showAddForm = false;
		newRuleText = '';
		submitResult = null;
		submitError = null;
	}

	$effect(() => {
		if (!browser) return;
		exomPath;
		void loadRules();
		return () => {
			if (copyTimer) clearTimeout(copyTimer);
		};
	});
</script>

<div class="flex flex-col gap-3">
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
		<Button size="sm" onclick={() => (showAddForm = !showAddForm)}>
			{#if showAddForm}
				<X class="mr-1 size-3.5" />
				Cancel
			{:else}
				<Plus class="mr-1 size-3.5" />
				Add rule
			{/if}
		</Button>
	</div>

	{#if showAddForm}
		<div class="rounded-lg border border-border/60 bg-card p-3">
			<textarea
				class="w-full rounded-md border border-border/40 bg-background p-2 font-mono text-sm text-foreground placeholder:text-muted-foreground/50 focus:outline-none focus:ring-1 focus:ring-ring"
				rows={3}
				placeholder="(rule main (colleague ?x ?y) (works_at ?x ?z) (works_at ?y ?z))"
				bind:value={newRuleText}
				disabled={submitting}
			></textarea>
			<div class="mt-2 flex flex-wrap items-center gap-2">
				<Button size="sm" onclick={handleAddRule} disabled={submitting || !newRuleText.trim()}>
					{#if submitting}
						<LoaderCircle class="mr-1 size-3.5 animate-spin" />
						Adding…
					{:else}
						Submit
					{/if}
				</Button>
				<Button size="sm" variant="ghost" onclick={closeAddForm} disabled={submitting}>
					Cancel
				</Button>
			</div>
			{#if submitResult}
				<p class="mt-2 text-xs text-fact-base">{submitResult}</p>
			{/if}
			{#if submitError}
				<p class="mt-2 text-xs text-contra">{submitError}</p>
			{/if}
		</div>
	{/if}

	{#if editingRule}
		<div class="rounded-lg border border-border/60 bg-card p-3">
			<div class="mb-2 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
				<Button variant="ghost" size="sm" onclick={closeEditRule} disabled={editing}>
					<X class="mr-1 size-3.5" />
					Close
				</Button>
			</div>

			<div class="grid gap-3 lg:grid-cols-[1.4fr_1fr]">
				<div>
					<label for="edit-rule-raw" class="mb-1 block text-xs font-medium text-muted-foreground"
						>Raw Rayfall</label
					>
					<textarea
						id="edit-rule-raw"
						class="min-h-28 w-full rounded-md border border-border/40 bg-background p-2 font-mono text-sm text-foreground placeholder:text-muted-foreground/50 focus:outline-none focus:ring-1 focus:ring-ring"
						bind:value={editRawText}
						disabled={editing}
					></textarea>
				</div>
				<div class="rounded-md border border-border/40 bg-muted/20 p-3 text-sm">
					<div class="mb-2 flex items-center gap-2 text-xs">
						Preview
					</div>
					{#if editDraft.rule}
						<div class="space-y-2 font-mono text-xs text-foreground/90">
							<div>Head: {editDraft.rule.head_predicate}</div>
							<div>Body atoms: {editDraft.rule.body_atoms.join(', ') || '—'}</div>
						</div>
					{:else}
						<p class="text-xs text-contra">{editDraft.error}</p>
					{/if}
				</div>
			</div>
			{#if editError}
				<p class="mt-2 text-xs text-contra">{editError}</p>
			{/if}
			<div class="mt-3 flex flex-wrap items-center gap-2">
				<Button size="sm" onclick={handleSaveEditRule} disabled={editing}>
					{#if editing}
						<LoaderCircle class="mr-1 size-3.5 animate-spin" />
						Saving…
					{:else}
						<Check class="mr-1 size-3.5" />
						Save changes
					{/if}
				</Button>
				<Button size="sm" variant="outline" onclick={closeEditRule} disabled={editing}>Cancel</Button>
			</div>
		</div>
	{/if}

	<div class="relative">
		<Search class="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
		<Input
			class="pl-8 text-sm"
			placeholder="Filter rules…"
			bind:value={searchQuery}
		/>
	</div>

	{#if loading}
		<div class="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
			<LoaderCircle class="size-4 animate-spin" />
			Loading rules…
		</div>
	{:else if error}
		<div class="rounded-lg border border-contra/30 bg-contra/5 p-3 text-sm text-contra">
			{error}
		</div>
	{:else if filteredRules.length === 0}
		<div class="flex flex-col gap-2 py-8">
			{#if searchQuery.trim()}
				<p class="text-sm text-muted-foreground">No rules match your filter.</p>
			{:else}
				<p class="font-serif text-sm text-muted-foreground">
					No rules yet — derivations let you compute things from your facts.
				</p>
				<pre
					class="mt-1 overflow-x-auto rounded border border-border/50 bg-background p-2 font-mono text-[11px] leading-relaxed text-muted-foreground"
				>{exampleRule}</pre>
			{/if}
		</div>
	{:else}
		<div class="flex flex-col gap-1.5">
			{#each filteredRules as rule (rule.index)}
				<div
					class="border border-border/60 bg-card/50 px-3 py-2"
				>
					<div class="mb-1 flex items-center justify-between gap-2">
						<div class="flex flex-wrap items-center gap-2">
							<span class="inline-block size-1.5 rounded-full bg-rule-accent" aria-hidden="true"></span>
							<span class="text-xs font-semibold text-rule-accent">{rule.head_predicate}</span>
							{#if rule.uses_negation}
								<span class="text-[10px] text-contra">negation</span>
							{/if}
							{#if rule.uses_temporal}
								<span class="text-[10px] text-fact-derived">temporal</span>
							{/if}
						</div>
						<div class="flex items-center gap-1">
							<Button
								variant="ghost"
								size="sm"
								class="h-7 px-2 text-[0.7rem]"
								onclick={() => copySnippet(`rule:${rule.index}`, ruleHeadQuery(rule))}
							>
								{#if copiedSnippet === `rule:${rule.index}`}
									<Check class="mr-1 size-3.5" />
									Copy query
								{:else}
									<Copy class="mr-1 size-3.5" />
									Copy query
								{/if}
							</Button>
							<button
								class="rounded p-1 text-muted-foreground hover:bg-primary/10 hover:text-primary"
								onclick={() => openEditRule(rule)}
								type="button"
								title="Edit rule"
							>
								<Pencil class="size-3.5" />
							</button>
							{#if deleteConfirmIndex === rule.index}
								<button
									class="rounded p-1 text-muted-foreground hover:bg-muted/70 hover:text-foreground"
									onclick={cancelDeleteRule}
									type="button"
									title="Cancel delete"
								>
									<X class="size-3.5" />
								</button>
								<button
									class="rounded p-1 text-contra hover:bg-contra/10"
									onclick={() => handleDeleteRule(rule)}
									disabled={deleting}
									type="button"
									title="Confirm delete"
								>
									{#if deleting}
										<LoaderCircle class="size-3.5 animate-spin" />
									{:else}
										<Check class="size-3.5" />
									{/if}
								</button>
							{:else}
								<button
									class="rounded p-1 text-muted-foreground hover:bg-destructive/10 hover:text-contra"
									onclick={() => promptDeleteRule(rule)}
									type="button"
									title="Delete rule"
								>
									<Trash2 class="size-3.5" />
								</button>
							{/if}
						</div>
					</div>

					<div class="font-mono text-sm leading-relaxed">
						{@html highlightRule(rule.raw)}
					</div>

					<div
						class="mt-2 rounded-md border border-border/40 bg-muted/20 px-2.5 py-2 font-mono text-[11px] leading-relaxed text-muted-foreground"
					>
						{ruleHeadQuery(rule)}
					</div>

					{#if rule.body_atoms.length > 0}
						<div class="mt-1.5 flex flex-wrap gap-1">
							{#each rule.body_atoms as atom, i (i)}
								<span
									class="h-4 rounded border border-border/40 bg-muted/20 px-1.5 font-mono text-[10px] text-fact-base"
									>{extractPredicateFromAtom(atom)}</span
								>
							{/each}
						</div>
					{/if}
				</div>
			{/each}
		</div>
	{/if}
</div>
