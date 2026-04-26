<script lang="ts">
	let {
		id,
		title,
		children
	}: { id: string; title: string; children: import('svelte').Snippet } = $props();
</script>

<section {id} class="scroll-mt-6 space-y-4 border-t border-border/50 pt-8">
	<h3 class="font-mono text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
		— {title} —
	</h3>
	{@render children()}
</section>
