<script lang="ts">
	import { RefreshCw } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';

	let {
		message,
		onRetry
	}: {
		message: string;
		onRetry?: () => void;
	} = $props();
</script>

<div class="flex flex-col gap-2 border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
	<p>{message}</p>
	{#if onRetry}
		<Button
			variant="outline"
			size="sm"
			class="w-fit border-destructive/50 text-destructive hover:bg-destructive/15"
			onclick={onRetry}
		>
			<RefreshCw class="mr-1 size-3" />
			Retry
		</Button>
	{/if}
</div>
