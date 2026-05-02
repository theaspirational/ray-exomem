<script lang="ts">
	import { base } from '$app/paths';
	import { rendererFor, type RendererKind } from '$lib/predicateRendering.svelte';
	import type { FactEntry } from '$lib/types';

	let {
		entityKey,
		facts,
		exomPath
	}: { entityKey: string; facts: FactEntry[]; exomPath: string } = $props();

	const bins = $derived.by(() => {
		const sorted = [...facts].sort((a, b) => {
			const ap = a.predicate.localeCompare(b.predicate);
			if (ap !== 0) return ap;
			return (a.factId ?? '').localeCompare(b.factId ?? '');
		});
		const b: Record<RendererKind, FactEntry[]> = {
			heading: [],
			tag: [],
			lead: [],
			'doc-link': [],
			relation: [],
			'status-tag': [],
			kv: []
		};
		for (const f of sorted) {
			b[rendererFor(f.predicate)].push(f);
		}
		return b;
	});

	const headingFact = $derived(
		bins.heading.find((f) => f.predicate === 'entity/name') ?? bins.heading[0] ?? null
	);

	const displayHeading = $derived(
		headingFact?.terms[0]?.trim() ? (headingFact!.terms[0]!.trim() as string) : null
	);

	const primaryFactId = $derived(headingFact?.factId ?? facts[0]?.factId ?? entityKey);

	function valueStr(f: FactEntry): string {
		return f.terms.join(', ');
	}

	function treeHrefForValue(v: string): string {
		const q = new URLSearchParams();
		q.set('fact', v);
		return `${base}/tree/${exomPath
			.split('/')
			.map((p) => encodeURIComponent(p))
			.join('/')}?${q.toString()}`;
	}
</script>

<article
	class="rounded border border-border/50 bg-card/30 px-4 py-3"
	data-entity={entityKey}
>
	<div class="flex flex-wrap items-baseline justify-between gap-2">
		<div class="min-w-0 flex flex-wrap items-baseline gap-2">
			{#if displayHeading}
				<h2 class="font-serif text-2xl text-foreground">{displayHeading}</h2>
			{:else}
				<h2 class="font-mono text-base text-foreground/90">{entityKey}</h2>
			{/if}
			<div class="flex flex-wrap items-center gap-1.5">
				{#each bins.tag as t (t.factId ?? t.predicate + valueStr(t))}
					<span
						class="rounded-sm bg-muted px-1.5 py-0.5 font-mono text-[11px] text-foreground/90"
					>
						{valueStr(t)}
					</span>
				{/each}
				{#each bins['status-tag'] as t (t.factId ?? t.predicate + valueStr(t))}
					<span
						class="rounded-sm border border-primary/25 px-1.5 py-0.5 font-mono text-[11px] text-foreground/90"
					>
						{valueStr(t)}
					</span>
				{/each}
			</div>
		</div>
		<span class="shrink-0 font-mono text-[10px] text-muted-foreground" title={primaryFactId}>
			{primaryFactId}
		</span>
	</div>

	{#each bins.lead as f (f.factId ?? f.predicate + valueStr(f))}
		<p class="mt-2 font-serif leading-relaxed text-foreground/95">{valueStr(f)}</p>
	{/each}

	{#each bins['doc-link'] as f (f.factId ?? f.predicate + valueStr(f))}
		<div class="mt-2 flex flex-wrap items-baseline gap-2 text-sm">
			<span class="font-mono text-[11px] text-muted-foreground">docs</span>
			<a
				href={f.terms[0]}
				class="text-primary hover:underline [overflow-wrap:anywhere]"
				target="_blank"
				rel="noreferrer">{f.terms[0]}</a
			>
		</div>
	{/each}

	{#each bins.kv as f (f.factId ?? f.predicate + valueStr(f))}
		<div class="mt-2 flex flex-wrap gap-x-2 gap-y-0.5 font-mono text-sm">
			<span class="text-muted-foreground">{f.predicate}</span>
			<span class="text-foreground/90">{valueStr(f)}</span>
		</div>
	{/each}

	{#if bins.relation.length}
		<h4
			class="mb-1 mt-3 font-mono text-[10px] uppercase tracking-wide text-muted-foreground"
		>
			Connections
		</h4>
		<div class="space-y-1.5">
			{#each bins.relation as f (f.factId ?? f.predicate + valueStr(f))}
				<div class="flex flex-wrap items-baseline gap-2 text-sm">
					<span class="font-mono text-[12px] text-foreground/80">{f.predicate}</span>
					<a
						href={treeHrefForValue(f.terms[0] ?? '')}
						class="font-mono text-[12px] text-primary hover:underline [overflow-wrap:anywhere]"
					>
						{f.terms[0] ?? '—'}
					</a>
				</div>
			{/each}
		</div>
	{/if}
</article>
