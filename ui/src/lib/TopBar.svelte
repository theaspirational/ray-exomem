<script lang="ts">
	import { browser } from '$app/environment';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { ChevronRight } from '@lucide/svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';

	let actor = $state('—');

	const branchLabel = $derived(page.url.searchParams.get('branch') ?? 'main');

	const crumbs = $derived.by(() => {
		let pathname = String(page.url.pathname);
		if (base && pathname.startsWith(base)) {
			pathname = pathname.slice(base.length) || '/';
		}
		if (!pathname.startsWith('/tree')) {
			return ['tree'];
		}
		const rest = pathname.slice('/tree'.length).replace(/^\/+/, '');
		if (!rest) return ['tree'];
		return ['tree', ...rest.split('/').filter(Boolean)];
	});

	$effect(() => {
		if (!browser) return;
		actor = localStorage.getItem('ray-exomem-actor')?.trim() || '—';
	});
</script>

<header
	class="sticky top-0 z-20 flex h-11 shrink-0 items-center border-b border-zinc-700 bg-zinc-900 px-3 font-sans text-zinc-100"
>
	<div class="min-w-0 flex-1">
		<nav class="flex min-w-0 flex-wrap items-center gap-1 text-xs" aria-label="Path breadcrumb">
			{#each crumbs as seg, i (i + seg)}
				{#if i > 0}
					<ChevronRight class="size-3.5 shrink-0 text-zinc-500" aria-hidden="true" />
				{/if}
				<span class="truncate font-mono text-[11px] text-zinc-200">{seg}</span>
			{/each}
		</nav>
	</div>

	<div class="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2">
		<Badge variant="secondary" class="pointer-events-auto border border-zinc-600 bg-zinc-800 font-mono text-[11px] text-zinc-100">
			{branchLabel}
		</Badge>
	</div>

	<div class="min-w-0 flex-1 text-right">
		<span class="inline-block max-w-[40vw] truncate font-sans text-xs text-zinc-300" title={actor}>
			<span class="text-zinc-500">actor</span>
			<span class="ml-1.5 font-mono text-zinc-100">{actor}</span>
		</span>
	</div>
</header>
