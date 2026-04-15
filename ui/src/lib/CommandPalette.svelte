<script lang="ts">
	import { base } from '$app/paths';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { toast } from 'svelte-sonner';
	import { auth } from '$lib/auth.svelte';
	import {
		CommandDialog,
		CommandEmpty,
		CommandGroup,
		CommandInput,
		CommandItem,
		CommandList
	} from '$lib/components/ui/command/index.js';
	import { fetchBranches, fetchTree, type TreeNode } from '$lib/exomem.svelte';
	import { treeModals } from '$lib/treeModals.svelte';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	let query = $state('');
	let paths = $state<string[]>([]);
	let branches = $state<string[]>([]);
	let loading = $state(false);

	const currentTreePath = $derived.by((): string => {
		let pathname = String(page.url.pathname);
		if (base && pathname.startsWith(base)) {
			pathname = pathname.slice(base.length) || '/';
		}
		if (!pathname.startsWith('/tree')) return '';
		return pathname.slice('/tree'.length).replace(/^\/+/, '');
	});

	const initTargetPath = $derived(currentTreePath || auth.user?.email || '');

	function fuzzyMatch(q: string, text: string): boolean {
		const ql = q.trim().toLowerCase();
		const t = text.toLowerCase();
		if (!ql) return true;
		if (t.includes(ql)) return true;
		let ti = 0;
		for (const c of ql) {
			ti = t.indexOf(c, ti);
			if (ti === -1) return false;
			ti++;
		}
		return true;
	}

	function collectPaths(n: TreeNode): string[] {
		if (n.kind === 'folder') {
			const self = n.path ? [n.path] : [];
			return [...self, ...n.children.flatMap(collectPaths)];
		}
		return [n.path];
	}

	const filteredPaths = $derived(paths.filter((p) => fuzzyMatch(query, p)));
	const filteredBranches = $derived(branches.filter((b) => fuzzyMatch(query, b)));

	const showGuide = $derived(fuzzyMatch(query, 'open guide') || fuzzyMatch(query, 'guide'));
	const showInit = $derived(fuzzyMatch(query, 'init'));
	const showRename = $derived(fuzzyMatch(query, 'rename'));
	const showQuery = $derived(fuzzyMatch(query, 'query') || fuzzyMatch(query, 'open query'));

	$effect(() => {
		if (!open) return;
		loading = true;
		const ac = new AbortController();
		void (async () => {
			try {
				const root = await fetchTree(undefined, {
					depth: 10,
					archived: true,
					signal: ac.signal
				});
				paths = collectPaths(root).filter((p) => p.length > 0);
				branches = [];
				const p = currentTreePath;
				if (p) {
					const node = await fetchTree(p, {
						depth: 1,
						branches: true,
						archived: true,
						signal: ac.signal
					});
					if (node.kind === 'exom' && node.branches?.length) {
						branches = node.branches;
					}
				}
			} catch (e) {
				if (!ac.signal.aborted) {
					paths = [];
					branches = [];
					toast.error(e instanceof Error ? e.message : 'Failed to load tree');
				}
			} finally {
				if (!ac.signal.aborted) loading = false;
			}
		})();
		return () => ac.abort();
	});

	function goTree(path: string) {
		const clean = path.replace(/^\/+/, '');
		open = false;
		void goto(`${base}/tree/${clean}`);
	}

	function switchBranch(name: string) {
		const u = new URL(page.url.href);
		u.searchParams.set('branch', name);
		open = false;
		void goto(`${u.pathname}${u.search}${u.hash}`);
	}

	function doGuide() {
		open = false;
		void goto(`${base}/guide`);
	}

	function doInit() {
		open = false;
		treeModals.openInit(initTargetPath);
	}

	function doRename() {
		if (!currentTreePath) {
			toast.message('Open a tree path first');
			return;
		}
		open = false;
		treeModals.openRename(currentTreePath);
	}

	function doQuery() {
		open = false;
		void goto(`${base}/query`);
	}
</script>

<CommandDialog
	bind:open
	bind:value={query}
	shouldFilter={false}
	title="Command palette"
	description="Navigate the tree or run actions"
	class="border-zinc-700 bg-zinc-950 text-zinc-100"
	showCloseButton={false}
>
	<CommandInput
		placeholder="Filter paths (full /slash/path)…"
		class="border-zinc-800 bg-zinc-950 text-zinc-100 placeholder:text-zinc-600"
	/>
	<CommandList class="max-h-[min(50vh,24rem)] thin-scrollbar">
		<CommandEmpty class="py-6 text-center text-sm text-zinc-500">
			{loading ? 'Loading tree…' : 'No matches.'}
		</CommandEmpty>

		{#if filteredPaths.length > 0}
			<CommandGroup heading="Go to" class="text-zinc-500 [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5">
				{#each filteredPaths as p (p)}
					<CommandItem
						value={p}
						class="cursor-pointer font-mono text-xs text-zinc-200 aria-selected:bg-zinc-800"
						onSelect={() => goTree(p)}
					>
						<span class="truncate">{p}</span>
					</CommandItem>
				{/each}
			</CommandGroup>
		{/if}

		{#if filteredBranches.length > 0}
			<CommandGroup heading="Switch branch" class="text-zinc-500 [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5">
				{#each filteredBranches as b (b)}
					<CommandItem
						value={`branch ${b}`}
						class="cursor-pointer text-sm text-zinc-200 aria-selected:bg-zinc-800"
						onSelect={() => switchBranch(b)}
					>
						Switch branch {b}
					</CommandItem>
				{/each}
			</CommandGroup>
		{/if}

		{#if showGuide || showInit || showRename || showQuery}
			<CommandGroup heading="Actions" class="text-zinc-500 [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5">
				{#if showGuide}
					<CommandItem
						value="open guide"
						class="cursor-pointer text-sm text-zinc-200 aria-selected:bg-zinc-800"
						onSelect={doGuide}
					>
						Open guide
					</CommandItem>
				{/if}
				{#if showInit}
					<CommandItem
						value="init here"
						class="cursor-pointer text-sm text-zinc-200 aria-selected:bg-zinc-800"
						onSelect={doInit}
					>
						Init here{initTargetPath ? ` (${initTargetPath})` : ''}
					</CommandItem>
				{/if}
				{#if showRename}
					<CommandItem
						value="rename"
						class="cursor-pointer text-sm text-zinc-200 aria-selected:bg-zinc-800"
						onSelect={doRename}
					>
						Rename (current path)
					</CommandItem>
				{/if}
				{#if showQuery}
					<CommandItem
						value="open query editor"
						class="cursor-pointer text-sm text-zinc-200 aria-selected:bg-zinc-800"
						onSelect={doQuery}
					>
						Open Query Editor
					</CommandItem>
				{/if}
			</CommandGroup>
		{/if}
	</CommandList>
</CommandDialog>
