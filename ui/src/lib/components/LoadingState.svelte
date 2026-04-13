<script lang="ts">
	import { Loader2 } from '@lucide/svelte';

	let { message = 'Loading…' }: { message?: string } = $props();
</script>

<p class="flex items-center gap-2 text-sm text-zinc-500">
	<Loader2 class="size-4 animate-spin text-zinc-400" aria-hidden="true" />
	{message}
</p>
