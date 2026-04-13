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

<div class="flex flex-col gap-2 rounded-md border border-red-900/40 bg-red-950/25 px-3 py-2 text-sm text-red-200">
	<p>{message}</p>
	{#if onRetry}
		<Button
			variant="outline"
			size="sm"
			class="w-fit border-red-800/60 text-red-100"
			onclick={onRetry}
		>
			<RefreshCw class="mr-1 size-3" />
			Retry
		</Button>
	{/if}
</div>
