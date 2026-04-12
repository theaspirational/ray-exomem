<script lang="ts">
	import ExomView from './ExomView.svelte';
	import type { TreeExom } from '$lib/exomem.svelte';

	let { node }: { node: TreeExom } = $props();
</script>

<ExomView {node} readOnly contentDimmed showUnarchive />
