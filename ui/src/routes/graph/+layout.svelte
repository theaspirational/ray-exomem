<script lang="ts">
	import type { Snippet } from 'svelte';

	let { children }: { children: Snippet } = $props();
</script>

<!-- Fill main; parent main is flex column flex-1 min-h-0 -->
<div class="flex min-h-0 flex-1 flex-col overflow-hidden">
	{@render children()}
</div>
