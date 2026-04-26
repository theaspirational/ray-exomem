<script lang="ts">
	import { ExternalLink } from '@lucide/svelte';
	import { base } from '$app/paths';

	let {
		items
	}: {
		items: {
			exom: string;
			name: string;
			type: string;
			summary: string | null;
			docs_url: string | null;
			fact_count: number;
		}[];
	} = $props();

	function treeHref(exom: string) {
		const segs = exom.split('/').filter(Boolean);
		return `${base}/tree/${segs.map(encodeURIComponent).join('/')}`;
	}
</script>

<section class="space-y-3">
	<h2 class="font-sans text-xs font-medium uppercase tracking-wide text-foreground/60">Featured</h2>
	<div class="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
		{#each items.slice(0, 6) as f (f.exom + f.name)}
			<article
				class="flex min-h-0 min-w-0 flex-col border border-border bg-card p-0 transition hover:border-primary/50"
			>
				<a
					href={treeHref(f.exom)}
					class="group flex min-h-0 flex-1 flex-col p-4 text-left outline-none"
				>
					<h3 class="font-serif text-base font-medium text-foreground group-hover:underline">
						{f.name}
					</h3>
					<span class="mt-1.5 font-mono text-[11px] text-foreground/60">{f.type}</span>
					{#if f.summary}
						<p class="mt-2 line-clamp-3 font-serif text-sm text-foreground/85">
							{f.summary}
						</p>
					{/if}
					<p class="mt-3 font-mono text-[11px] text-foreground/50">{f.fact_count} facts</p>
				</a>
				{#if f.docs_url}
					<div class="border-t border-border px-4 pb-3 pt-0">
						<a
							href={f.docs_url}
							target="_blank"
							rel="noopener noreferrer"
							class="inline-flex items-center gap-1 font-sans text-xs text-primary hover:underline"
						>
							<ExternalLink class="size-3 shrink-0" aria-hidden="true" />
							docs
						</a>
					</div>
				{/if}
			</article>
		{/each}
	</div>
</section>
