<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';

	onMount(() => {
		void goto(`${base}/tree/`, { replaceState: true });
	});
</script>

<p class="p-4 text-sm text-zinc-400">Redirecting…</p>
