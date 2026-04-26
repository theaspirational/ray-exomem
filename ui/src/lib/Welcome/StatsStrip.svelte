<script lang="ts">
	import { formatRelativeTime } from '$lib/formatRelativeTime';

	let {
		facts,
		exoms,
		branches,
		lastChange,
		loading
	}: {
		facts: number;
		exoms: number;
		branches: number;
		lastChange: { tx_time: string; actor: string } | null;
		loading: boolean;
	} = $props();
</script>

<p class="font-mono text-sm text-foreground/90">
	{#if loading}
		<span class="text-foreground/50">Loading stats…</span>
	{:else}
		{facts} facts · {exoms} exoms · {branches} branches ·
		{#if lastChange}
			last change {formatRelativeTime(lastChange.tx_time)} by
			<span class="text-foreground">{lastChange.actor}</span>
		{:else}
			last change —
		{/if}
	{/if}
</p>
