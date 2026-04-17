<script lang="ts">
	import { fetchExomemStatus, getExomemBaseUrl } from '$lib/exomem.svelte';
	import { auth } from '$lib/auth.svelte';
	import { app } from '$lib/stores.svelte';
	import type { ExomemStatus } from '$lib/types';

	let health = $state<'ok' | 'unreachable'>('unreachable');
	let exomCount = $state<number | null>(null);

	function countExomsInTree(node: unknown): number {
		if (!node || typeof node !== 'object') return 0;
		const o = node as Record<string, unknown>;
		if (o.kind === 'exom') return 1;
		if (o.kind === 'folder' && Array.isArray(o.children)) {
			return o.children.reduce((sum: number, ch: unknown) => sum + countExomsInTree(ch), 0);
		}
		return 0;
	}

	async function loadTreeCount(): Promise<number | null> {
		try {
			const url = `${getExomemBaseUrl()}/api/tree`;
			const res = await fetch(url, { credentials: 'include' });
			if (!res.ok) return null;
			const node: unknown = await res.json();
			return countExomsInTree(node);
		} catch {
			return null;
		}
	}

	$effect(() => {
		app.selectedExom;
		let cancelled = false;
		(async () => {
			let status: ExomemStatus | null = null;
			if (app.selectedExom) {
				try {
					status = await fetchExomemStatus(app.selectedExom);
				} catch {
					status = null;
				}
			}
			if (cancelled) return;
			const n = await loadTreeCount();
			if (cancelled) return;
			if (status?.ok || n !== null) {
				health = 'ok';
			} else {
				health = 'unreachable';
			}
			exomCount = n;
		})();
		return () => {
			cancelled = true;
		};
	});
</script>

<footer
	class="flex h-8 shrink-0 items-center gap-x-4 gap-y-1 border-t border-zinc-700 bg-zinc-900 px-3 font-sans text-[11px] text-zinc-500"
>
	<span class={health === 'ok' ? 'text-zinc-400' : 'text-zinc-500'}>
		{health === 'ok' ? 'daemon ok' : 'daemon unreachable'}
	</span>
	<span>
		<span class="text-zinc-600">exoms</span>
		<span class="ml-1 font-mono tabular-nums text-zinc-400">{exomCount ?? '—'}</span>
	</span>
	{#if auth.isAuthenticated}
		<span>
			<span class="text-zinc-600">role</span>
			<span class="ml-1 font-mono text-zinc-400">{auth.user?.role}</span>
		</span>
	{/if}
</footer>
