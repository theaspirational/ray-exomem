<script lang="ts">
	import { Loader2 } from '@lucide/svelte';

	let { message = 'Loading…' }: { message?: string } = $props();
</script>

<p class="flex items-center gap-2 text-sm text-muted-foreground">
	<Loader2 class="size-4 animate-spin text-foreground/60" aria-hidden="true" />
	{message}
</p>
