<script lang="ts">
	import { formatRelativeTime } from '$lib/formatRelativeTime';

	let {
		items
	}: {
		items: {
			exom: string;
			tx_id: number;
			tx_time: string;
			actor: string;
			action: string;
			note: string | null;
		}[];
	} = $props();
</script>

<section class="space-y-2">
	<h2 class="font-sans text-xs font-medium uppercase tracking-wide text-foreground/60">
		Seeded knowledge
	</h2>
	<p class="font-sans text-xs text-foreground/55">Seeded from bootstrap fixtures</p>
	<ul
		class="flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] text-foreground/70"
		aria-label="Most recent seed timestamps"
	>
		{#each items as row (row.exom + ':' + row.tx_id)}
			<li>
				<time datetime={row.tx_time}>{formatRelativeTime(row.tx_time)}</time>
				<span class="text-foreground/50"> · {row.actor}</span>
			</li>
		{/each}
	</ul>
</section>
