<script lang="ts">
	import { Loader2, RefreshCw } from '@lucide/svelte';
	import { marked } from 'marked';
	import { Button } from '$lib/components/ui/button/index.js';
	import { fetchGuideMarkdown } from '$lib/exomem.svelte';

	let html = $state('');
	let loading = $state(true);
	let err = $state<string | null>(null);
	let loadToken = $state(0);

	$effect(() => {
		loadToken;
		loading = true;
		err = null;
		const ac = new AbortController();
		void fetchGuideMarkdown(ac.signal)
			.then((md) => {
				html = marked.parse(md, { async: false }) as string;
			})
			.catch((e) => {
				html = '';
				err = e instanceof Error ? e.message : 'Failed to load guide';
			})
			.finally(() => {
				if (!ac.signal.aborted) loading = false;
			});
		return () => ac.abort();
	});
</script>

<div class="min-h-full p-4 sm:p-8">
	<h1 class="mb-6 font-sans text-lg font-semibold">Guide</h1>

	{#if loading}
		<p class="flex items-center gap-2 text-sm text-muted-foreground">
			<Loader2 class="size-4 animate-spin text-foreground/60" aria-hidden="true" />
			Loading…
		</p>
	{:else if err}
		<div class="flex flex-col gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
			<p>{err}</p>
			<Button
				variant="outline"
				size="sm"
				class="w-fit border-destructive/50 text-destructive"
				onclick={() => loadToken++}
			>
				<RefreshCw class="mr-1 size-3" />
				Retry
			</Button>
		</div>
	{:else}
		<article class="guide-md max-w-3xl font-sans">
			<!-- API-served markdown is trusted (same origin daemon). -->
			{@html html}
		</article>
	{/if}
</div>

<style>
	.guide-md :global(h1) { margin-top: 1.75rem; margin-bottom: 0.75rem; font-size: 1.25rem; font-weight: 600; color: var(--foreground); }
	.guide-md :global(h2) { margin-top: 1.5rem; margin-bottom: 0.5rem; font-size: 1.1rem; font-weight: 600; color: var(--foreground); }
	.guide-md :global(h3) { margin-top: 1.25rem; margin-bottom: 0.4rem; font-size: 1rem; font-weight: 600; color: var(--foreground); }
	.guide-md :global(p) { margin-bottom: 0.75rem; line-height: 1.65; font-size: 0.875rem; color: var(--muted-foreground); }
	.guide-md :global(ul), .guide-md :global(ol) { margin-bottom: 0.75rem; padding-left: 1.25rem; font-size: 0.875rem; line-height: 1.6; color: var(--muted-foreground); }
	.guide-md :global(li) { margin-bottom: 0.35rem; }
	.guide-md :global(pre) { margin: 0.75rem 0; overflow-x: auto; border-radius: 0.375rem; border: 1px solid var(--border); background: var(--background); padding: 0.75rem 1rem; font-size: 0.8rem; line-height: 1.5; color: var(--foreground); }
	.guide-md :global(code) { font-family: var(--font-mono); font-size: 0.85em; }
	.guide-md :global(p code), .guide-md :global(li code) { border-radius: 0.25rem; background: var(--muted); padding: 0.1rem 0.35rem; color: var(--rule-accent); }
	.guide-md :global(a) { color: var(--primary); text-decoration: underline; text-underline-offset: 2px; }
	.guide-md :global(blockquote) { margin: 0.75rem 0; border-left: 3px solid var(--border); padding-left: 1rem; color: var(--muted-foreground); }
</style>
