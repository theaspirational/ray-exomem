<script lang="ts">
	import ExomView from './ExomView.svelte';
	import type { TreeExom } from '$lib/exomem.svelte';

	let { node }: { node: TreeExom } = $props();

	const sessionClosed = $derived(
		Boolean(node.closed || (node.session && (node.session as { closed_at?: string }).closed_at))
	);
</script>

{#if sessionClosed}
	<div
		class="mb-4 rounded-md border border-primary/40 bg-primary/10 px-3 py-2.5 text-sm text-foreground"
		role="status"
	>
		This session is closed. No further writes are accepted.
	</div>
{/if}

<ExomView {node} sessionModes />
