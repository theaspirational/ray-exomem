<script lang="ts">
	import type { Icon } from '@lucide/svelte';

	let {
		label,
		value,
		icon: IconComponent
	}: {
		label: string;
		value: string | number;
		icon?: typeof Icon;
	} = $props();
</script>

<div class="border border-border/60 bg-background/40 px-3 py-2">
	<div class="text-[0.65rem] uppercase tracking-wide text-muted-foreground">{label}</div>
	<div class="mt-1 flex items-center gap-1.5 font-mono text-lg text-foreground">
		{#if IconComponent}
			<IconComponent class="size-4 opacity-70" />
		{/if}
		{value}
	</div>
</div>
