<script lang="ts">
	import { Tooltip, TooltipContent, TooltipTrigger } from '$lib/components/ui/tooltip/index.js';

	let {
		id,
		title,
		tooltip,
		children
	}: {
		id: string;
		title: string;
		tooltip?: string;
		children: import('svelte').Snippet;
	} = $props();
</script>

<section {id} class="scroll-mt-6 space-y-2 border-t border-border/50 pt-4">
	{#if tooltip}
		<Tooltip>
			<TooltipTrigger
				class="cursor-help font-mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground decoration-dotted underline-offset-4 hover:underline"
			>
				<h3>— {title} —</h3>
			</TooltipTrigger>
			<TooltipContent side="top">{tooltip}</TooltipContent>
		</Tooltip>
	{:else}
		<h3 class="font-mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
			— {title} —
		</h3>
	{/if}
	{@render children()}
</section>
