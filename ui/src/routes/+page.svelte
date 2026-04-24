<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { auth } from '$lib/auth.svelte';

	onMount(() => {
		const defaultPath = auth.user?.email ? `${auth.user.email}/main` : '';
		void goto(`${base}/tree/${defaultPath}`, { replaceState: true });
	});
</script>

<p class="p-4 text-sm text-zinc-400">Redirecting…</p>
