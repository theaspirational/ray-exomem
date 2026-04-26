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
			refs: string[];
		}[];
	} = $props();

	function tail(row: (typeof items)[0]): string {
		const n = row.note?.trim();
		if (n) return n;
		if (row.refs?.length) return row.refs.join(', ');
		return '—';
	}
</script>

<section class="space-y-3">
	<h2 class="font-sans text-xs font-medium uppercase tracking-wide text-foreground/60">
		Latest changes
	</h2>
	<ul class="space-y-2 border border-border bg-card/60 p-3 font-mono text-xs text-foreground/90">
		{#each items as row (row.exom + ':' + row.tx_id)}
			<li class="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
				<time class="shrink-0 text-foreground/60" datetime={row.tx_time}
					>{formatRelativeTime(row.tx_time)}</time
				>
				<span class="shrink-0 text-foreground">{row.actor}</span>
				<span class="shrink-0 text-primary">{row.action}</span>
				<span class="min-w-0 break-all text-foreground/80">{tail(row)}</span>
			</li>
		{/each}
	</ul>
</section>
