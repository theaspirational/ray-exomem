<script lang="ts">
	import { DEFAULT_EXOM, fetchExomemStatus, getExomemBaseUrl } from '$lib/exomem.svelte';
	import type { ExomemStatus } from '$lib/types';

	let health = $state<'ok' | 'unreachable'>('unreachable');
	let treeRoot = $state<string | null>(null);
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
			const res = await fetch(url);
			if (!res.ok) return null;
			const node: unknown = await res.json();
			return countExomsInTree(node);
		} catch {
			return null;
		}
	}

	$effect(() => {
		let cancelled = false;
		(async () => {
			let status: ExomemStatus | null = null;
			try {
				status = await fetchExomemStatus(DEFAULT_EXOM);
			} catch {
				status = null;
			}
			if (cancelled) return;
			if (status?.ok) {
				health = 'ok';
				treeRoot = status.server.tree_root ?? null;
			} else {
				health = 'unreachable';
				treeRoot = null;
			}
			const n = await loadTreeCount();
			if (!cancelled) exomCount = n;
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
	<span class="hidden min-w-0 truncate sm:inline" title={treeRoot ?? undefined}>
		<span class="text-zinc-600">tree</span>
		<span class="ml-1 font-mono text-zinc-400">{treeRoot ?? '—'}</span>
	</span>
	<span>
		<span class="text-zinc-600">exoms</span>
		<span class="ml-1 font-mono tabular-nums text-zinc-400">{exomCount ?? '—'}</span>
	</span>
</footer>
