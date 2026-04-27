<script lang="ts">
	import type { Icon } from '@lucide/svelte';
	import type { Snippet } from 'svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';

	let {
		icon: IconComponent,
		label,
		badges = [],
		trailing,
		onclick
	}: {
		icon?: typeof Icon;
		label: string;
		badges?: Array<{ text: string; variant?: 'default' | 'secondary' | 'outline' }>;
		trailing?: Snippet | string;
		onclick?: () => void;
	} = $props();
</script>

{#if onclick}
	<button
		type="button"
		class="flex w-full items-center justify-between gap-2 border border-border/60 bg-background/40 px-3 py-2 text-left transition-colors cursor-pointer hover:border-primary/50 hover:bg-secondary/60"
		{onclick}
	>
		<div class="flex min-w-0 flex-1 items-center gap-2">
			{#if IconComponent}
				<IconComponent class="size-4 shrink-0 text-foreground/60" />
			{/if}
			<span class="truncate font-mono text-sm text-foreground">{label}</span>
			{#each badges as b (b.text)}
				<Badge variant={b.variant ?? 'default'} class="text-[10px]">{b.text}</Badge>
			{/each}
		</div>
		{#if trailing}
			<div class="shrink-0 text-xs text-foreground/60">
				{#if typeof trailing === 'string'}
					{trailing}
				{:else}
					{@render trailing()}
				{/if}
			</div>
		{/if}
	</button>
{:else}
	<div class="flex w-full items-center justify-between gap-2 border border-border/60 bg-background/40 px-3 py-2">
		<div class="flex min-w-0 flex-1 items-center gap-2">
			{#if IconComponent}
				<IconComponent class="size-4 shrink-0 text-foreground/60" />
			{/if}
			<span class="truncate font-mono text-sm text-foreground">{label}</span>
			{#each badges as b (b.text)}
				<Badge variant={b.variant ?? 'default'} class="text-[10px]">{b.text}</Badge>
			{/each}
		</div>
		{#if trailing}
			<div class="shrink-0 text-xs text-foreground/60">
				{#if typeof trailing === 'string'}
					{trailing}
				{:else}
					{@render trailing()}
				{/if}
			</div>
		{/if}
	</div>
{/if}
