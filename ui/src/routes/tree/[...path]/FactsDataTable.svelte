<script lang="ts">
	import { Loader2 } from '@lucide/svelte';
	import { ScrollArea } from '$lib/components/ui/scroll-area/index.js';
	import type { ListedFact } from '$lib/exomem.svelte';

	let {
		facts,
		loading,
		emptyMessage = 'No facts yet'
	}: {
		facts: ListedFact[];
		loading: boolean;
		emptyMessage?: string;
	} = $props();

	function fmtValid(v: string | null | undefined) {
		if (v == null || v === '') return '—';
		return v;
	}
</script>

{#if loading}
	<p class="flex items-center gap-2 text-sm text-zinc-500">
		<Loader2 class="size-4 animate-spin text-zinc-400" aria-hidden="true" />
		Loading facts…
	</p>
{:else if facts.length === 0}
	<p class="text-sm text-zinc-500">{emptyMessage}</p>
{:else}
	<div class="overflow-x-auto rounded-md border border-zinc-700">
		<ScrollArea class="h-[min(60vh,520px)]">
			<table class="w-full border-collapse text-left text-xs">
				<thead class="sticky top-0 z-10 bg-zinc-950/95 text-[0.65rem] uppercase tracking-wide text-zinc-500">
					<tr>
						<th class="border-b border-zinc-700 px-2 py-2 font-medium">Predicate</th>
						<th class="border-b border-zinc-700 px-2 py-2 font-medium">Value</th>
						<th class="border-b border-zinc-700 px-2 py-2 font-medium">valid_from</th>
						<th class="border-b border-zinc-700 px-2 py-2 font-medium">valid_to</th>
						<th class="border-b border-zinc-700 px-2 py-2 font-medium">Actor</th>
					</tr>
				</thead>
				<tbody class="font-mono text-[11px] text-zinc-200">
					{#each facts as f (f.fact_id)}
						<tr class="border-b border-zinc-800/80 hover:bg-zinc-800/40">
							<td class="max-w-[140px] truncate px-2 py-1.5 align-top text-zinc-300">{f.predicate}</td>
							<td class="max-w-[min(40vw,280px)] break-all px-2 py-1.5 align-top">{f.value}</td>
							<td class="whitespace-nowrap px-2 py-1.5 align-top text-zinc-400">{fmtValid(f.valid_from)}</td>
							<td class="whitespace-nowrap px-2 py-1.5 align-top text-zinc-400">{fmtValid(f.valid_to)}</td>
							<td class="px-2 py-1.5 align-top text-zinc-400">{f.actor || '—'}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</ScrollArea>
	</div>
{/if}
