<script lang="ts">
	import { Package, type Icon } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';

	let {
		icon: IconComponent = Package,
		message,
		actionLabel,
		onAction
	}: {
		icon?: typeof Icon;
		message: string;
		actionLabel?: string;
		onAction?: () => void;
	} = $props();
</script>

<div class="flex flex-col items-center justify-center gap-2 py-12 text-center">
	<IconComponent class="size-8 text-muted-foreground/30" />
	<p class="text-sm text-zinc-500">{message}</p>
	{#if actionLabel && onAction}
		<Button variant="outline" size="sm" onclick={onAction}>{actionLabel}</Button>
	{/if}
</div>
