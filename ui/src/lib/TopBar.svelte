<script lang="ts">
	import { browser } from '$app/environment';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { ChevronRight, MoreHorizontal } from '@lucide/svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';

	let actor = $state('—');
	let overflowOpen = $state(false);

	const branchLabel = $derived(page.url.searchParams.get('branch') ?? 'main');

	const crumbs = $derived.by(() => {
		let pathname = String(page.url.pathname);
		if (base && pathname.startsWith(base)) {
			pathname = pathname.slice(base.length) || '/';
		}
		if (!pathname.startsWith('/tree')) {
			return [{ label: 'tree', href: `${base}/tree/` }];
		}
		const rest = pathname.slice('/tree'.length).replace(/^\/+/, '');
		if (!rest) return [{ label: 'tree', href: `${base}/tree/` }];
		const segments = rest.split('/').filter(Boolean);
		return [
			{ label: 'tree', href: `${base}/tree/` },
			...segments.map((seg, i) => ({
				label: seg,
				href: `${base}/tree/${segments.slice(0, i + 1).join('/')}`
			}))
		];
	});

	const MAX_VISIBLE = 3;
	const needsCollapse = $derived(crumbs.length > MAX_VISIBLE);
	const visibleCrumbs = $derived.by((): (typeof crumbs[0] | null)[] => {
		if (!needsCollapse) return crumbs;
		return [crumbs[0], null, crumbs[crumbs.length - 1]];
	});
	const collapsedCrumbs = $derived.by(() => {
		if (!needsCollapse) return [];
		return crumbs.slice(1, -1);
	});

	$effect(() => {
		if (!browser) return;
		actorPrompt.refreshSignal;
		actor = localStorage.getItem('ray-exomem-actor')?.trim() || '—';
	});

	$effect(() => {
		if (!overflowOpen || !browser) return;
		const handler = () => {
			overflowOpen = false;
		};
		setTimeout(() => document.addEventListener('click', handler, { once: true }), 0);
		return () => document.removeEventListener('click', handler);
	});
</script>

<header
	class="sticky top-0 z-20 flex h-11 shrink-0 items-center gap-2 border-b border-zinc-700 bg-zinc-900 px-3 font-sans text-zinc-100"
>
	<nav
		class="flex min-w-0 flex-1 items-center gap-1 overflow-hidden text-xs"
		aria-label="Path breadcrumb"
		title={crumbs.map((c) => c.label).join(' / ')}
	>
		{#each visibleCrumbs as crumb, i}
			{#if i > 0}
				<ChevronRight class="size-3.5 shrink-0 text-zinc-500" aria-hidden="true" />
			{/if}
			{#if crumb === null}
				<div class="relative">
					<button
						type="button"
						class="rounded bg-zinc-800 px-1.5 py-0.5 text-[11px] text-zinc-400 hover:bg-zinc-700 hover:text-zinc-200"
						onclick={(e) => {
							e.stopPropagation();
							overflowOpen = !overflowOpen;
						}}
						aria-label="Show full path"
					>
						<MoreHorizontal class="size-3.5" />
					</button>
					{#if overflowOpen}
						<div
							class="absolute left-0 top-full z-30 mt-1 min-w-[10rem] rounded-md border border-zinc-700 bg-zinc-900 py-1 shadow-lg"
						>
							{#each collapsedCrumbs as c (c.href)}
								<a
									href={c.href}
									class="block px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100"
									onclick={() => {
										overflowOpen = false;
									}}
								>{c.label}</a>
							{/each}
						</div>
					{/if}
				</div>
			{:else}
				{@const isLast = i === visibleCrumbs.length - 1}
				<a
					href={crumb.href}
					class="min-w-0 shrink truncate font-mono text-[11px] underline-offset-2 hover:underline {isLast
						? 'text-zinc-100'
						: 'text-zinc-400'}"
				>{crumb.label}</a>
			{/if}
		{/each}
	</nav>

	<Badge
		variant="secondary"
		class="shrink-0 border border-zinc-600 bg-zinc-800 font-mono text-[11px] text-zinc-100"
	>
		{branchLabel}
	</Badge>

	<div class="min-w-0 shrink-0 text-right">
		<span class="inline-block max-w-[20vw] truncate font-sans text-xs text-zinc-300" title={actor}>
			<span class="text-zinc-500">actor</span>
			<span class="ml-1.5 font-mono text-zinc-100">{actor}</span>
		</span>
	</div>
</header>
