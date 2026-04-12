<script lang="ts">
	import { marked } from 'marked';
	import { fetchGuideMarkdown } from '$lib/exomem.svelte';

	let html = $state('');
	let loading = $state(true);
	let err = $state<string | null>(null);

	$effect(() => {
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

<div class="min-h-full bg-zinc-900 p-4 text-zinc-100 sm:p-8">
	<h1 class="mb-6 font-sans text-lg font-semibold text-zinc-100">Guide</h1>

	{#if loading}
		<p class="text-sm text-zinc-500">Loading…</p>
	{:else if err}
		<p class="text-sm text-red-300">{err}</p>
	{:else}
		<article class="guide-md max-w-3xl font-sans">
			<!-- API-served markdown is trusted (same origin daemon). -->
			{@html html}
		</article>
	{/if}
</div>

<style>
	.guide-md :global(h1) {
		margin-top: 1.75rem;
		margin-bottom: 0.75rem;
		font-size: 1.25rem;
		font-weight: 600;
		color: rgb(244 244 245);
	}
	.guide-md :global(h2) {
		margin-top: 1.5rem;
		margin-bottom: 0.5rem;
		font-size: 1.1rem;
		font-weight: 600;
		color: rgb(228 228 231);
	}
	.guide-md :global(h3) {
		margin-top: 1.25rem;
		margin-bottom: 0.4rem;
		font-size: 1rem;
		font-weight: 600;
		color: rgb(212 212 216);
	}
	.guide-md :global(p) {
		margin-bottom: 0.75rem;
		line-height: 1.65;
		font-size: 0.875rem;
		color: rgb(212 212 216);
	}
	.guide-md :global(ul),
	.guide-md :global(ol) {
		margin-bottom: 0.75rem;
		padding-left: 1.25rem;
		font-size: 0.875rem;
		line-height: 1.6;
		color: rgb(212 212 216);
	}
	.guide-md :global(li) {
		margin-bottom: 0.35rem;
	}
	.guide-md :global(pre) {
		margin: 0.75rem 0;
		overflow-x: auto;
		border-radius: 0.375rem;
		border: 1px solid rgb(39 39 42);
		background: rgb(9 9 11);
		padding: 0.75rem 1rem;
		font-size: 0.8rem;
		line-height: 1.5;
		color: rgb(228 228 231);
	}
	.guide-md :global(code) {
		font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
		font-size: 0.85em;
	}
	.guide-md :global(p code),
	.guide-md :global(li code) {
		border-radius: 0.25rem;
		background: rgb(24 24 27);
		padding: 0.1rem 0.35rem;
		color: rgb(253 224 71);
	}
	.guide-md :global(a) {
		color: rgb(147 197 253);
		text-decoration: underline;
		text-underline-offset: 2px;
	}
	.guide-md :global(blockquote) {
		margin: 0.75rem 0;
		border-left: 3px solid rgb(63 63 70);
		padding-left: 1rem;
		color: rgb(161 161 170);
	}
</style>
