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
		class="mb-4 rounded-md border border-amber-600/50 bg-amber-950/50 px-3 py-2.5 text-sm text-amber-100"
		role="status"
	>
		This session is closed. No further writes are accepted.
	</div>
{/if}

<ExomView {node} sessionModes />
