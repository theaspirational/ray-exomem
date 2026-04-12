<script lang="ts">
	import {
		ChevronRight,
		Circle,
		FolderOpen,
		GitBranch,
		Loader2,
		RefreshCw
	} from '@lucide/svelte';
	import { tick } from 'svelte';
	import { toast } from 'svelte-sonner';

	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import {
		Collapsible,
		CollapsibleContent,
		CollapsibleTrigger
	} from '$lib/components/ui/collapsible/index.js';
	import {
		ContextMenu,
		ContextMenuContent,
		ContextMenuItem,
		ContextMenuTrigger
	} from '$lib/components/ui/context-menu/index.js';
	import { fetchTree, type TreeExom, type TreeNode } from '$lib/exomem.svelte';
	import { treeModals } from '$lib/treeModals.svelte';

	let {
		currentPath = '',
		refreshSignal = 0,
		onNavigate
	}: {
		currentPath?: string;
		refreshSignal?: number;
		onNavigate: (path: string) => void;
	} = $props();

	let root = $state<TreeNode | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	/** Folder open state; only explicit `true` opens (default closed). Reset on currentPath change; manual toggles persist until then. */
	let folderOpen = $state<Record<string, boolean>>({});
	/** Last currentPath we applied auto-reveal for (tree refresh does not reset manual toggles). */
	let lastRevealForPath = $state<string | undefined>(undefined);

	function parentFolderPrefixes(path: string): string[] {
		const parts = path.split('/').filter(Boolean);
		const out: string[] = [];
		let acc = '';
		for (let i = 0; i < parts.length - 1; i++) {
			acc = acc ? `${acc}/${parts[i]}` : parts[i];
			out.push(acc);
		}
		return out;
	}

	function allFolderPrefixes(path: string): string[] {
		const parts = path.split('/').filter(Boolean);
		const out: string[] = [];
		let acc = '';
		for (let i = 0; i < parts.length; i++) {
			acc = acc ? `${acc}/${parts[i]}` : parts[i];
			out.push(acc);
		}
		return out;
	}

	function findTreeNode(n: TreeNode, target: string): TreeNode | null {
		if (n.path === target) return n;
		if (n.kind !== 'folder') return null;
		for (const c of n.children) {
			const r = findTreeNode(c, target);
			if (r) return r;
		}
		return null;
	}

	/** Paths that must be open to reveal `targetPath` in the tree (VS Code–style). */
	function revealFolderPaths(tree: TreeNode, targetPath: string): string[] {
		if (!targetPath) return [];
		const node = findTreeNode(tree, targetPath);
		if (!node) return parentFolderPrefixes(targetPath);
		if (node.kind === 'folder') return allFolderPrefixes(node.path);
		return parentFolderPrefixes(node.path);
	}

	function folderIsOpen(path: string): boolean {
		return folderOpen[path] === true;
	}

	function setFolderOpen(path: string, value: boolean) {
		folderOpen = { ...folderOpen, [path]: value };
	}

	async function loadTree() {
		loading = true;
		error = null;
		try {
			root = await fetchTree(undefined, { depth: 10, branches: true, archived: true });
		} catch (e) {
			root = null;
			error = e instanceof Error ? e.message : 'Failed to load tree';
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		refreshSignal;
		void loadTree();
	});

	$effect(() => {
		const path = currentPath;
		root;
		if (!root) return;
		if (path === lastRevealForPath) return;
		lastRevealForPath = path;
		const reveal = revealFolderPaths(root, path);
		folderOpen = Object.fromEntries(reveal.map((p) => [p, true]));
	});

	$effect(() => {
		currentPath;
		root;
		void tick().then(() => {
			document.querySelector('[data-tree-active]')?.scrollIntoView({ block: 'nearest' });
		});
	});

	function notImplemented() {
		toast('Not implemented yet');
	}

	function exomDotClass(kind: string): string {
		if (kind === 'project_main') return 'fill-emerald-500 text-emerald-500';
		if (kind === 'session') return 'fill-sky-500 text-sky-500';
		return 'fill-zinc-500 text-zinc-500';
	}

	function isActivePath(path: string): boolean {
		return Boolean(currentPath && path === currentPath);
	}

	function activeRowClass(): string {
		return 'border-l-2 border-blue-400 bg-zinc-700/50 pl-0.5';
	}

	function labelForSession(node: TreeExom): string {
		const s = node.session as { label?: string } | null | undefined;
		return s?.label?.trim() || node.name;
	}
</script>

<div class="flex min-h-0 flex-1 flex-col gap-1 font-mono text-xs text-zinc-300">
	{#if loading}
		<div class="space-y-2 px-1 py-1" aria-busy="true">
			<div class="flex items-center gap-2 text-zinc-500">
				<Loader2 class="size-4 shrink-0 animate-spin text-zinc-400" aria-hidden="true" />
				<span class="text-[11px]">Loading tree…</span>
			</div>
			{#each Array.from({ length: 6 }) as _, i (i)}
				<div class="flex items-center gap-2">
					<div class="h-3.5 w-3.5 shrink-0 animate-pulse rounded-sm bg-zinc-700"></div>
					<div class="h-3 max-w-[70%] flex-1 animate-pulse rounded bg-zinc-700/80"></div>
					<div class="h-4 w-8 shrink-0 animate-pulse rounded bg-zinc-700/60"></div>
				</div>
			{/each}
		</div>
	{:else if error}
		<div class="flex flex-col gap-2 px-1 py-2 text-zinc-400">
			<p class="text-[11px] leading-relaxed">Failed to load tree</p>
			<p class="text-[10px] text-zinc-500">{error}</p>
			<Button
				variant="outline"
				size="sm"
				class="h-7 border-zinc-600 bg-zinc-800/80 text-zinc-200"
				onclick={() => void loadTree()}
			>
				<RefreshCw class="mr-1 size-3" />
				Retry
			</Button>
		</div>
	{:else if root}
		<div class="min-h-0 flex-1 overflow-y-auto thin-scrollbar px-0.5 py-0.5">
			{#snippet treeNodes(nodes: TreeNode[], depth: number)}
				{#each nodes as node (node.kind + '\0' + node.path)}
					<div style="padding-left: {depth * 12}px">
						{#if node.kind === 'folder'}
							<ContextMenu>
								<ContextMenuTrigger class="block w-full text-left">
									<Collapsible
										open={folderIsOpen(node.path)}
										onOpenChange={(v) => setFolderOpen(node.path, v)}
										class="w-full"
									>
										<div
											data-tree-active={isActivePath(node.path) ? '' : undefined}
											class="flex min-w-0 items-stretch rounded-sm border border-transparent {isActivePath(
												node.path
											)
												? activeRowClass()
												: ''}"
										>
											<CollapsibleTrigger
												class="flex min-w-0 flex-1 items-center gap-1 rounded-sm px-0.5 py-0.5 text-left text-zinc-300 hover:bg-zinc-800/80 [&[data-state=open]>svg:first-of-type]:rotate-90"
											>
												<ChevronRight
													class="size-3.5 shrink-0 text-zinc-500 transition-transform"
												/>
												<FolderOpen class="size-3.5 shrink-0 text-amber-600/90" />
												<span class="min-w-0 truncate text-[11px] text-zinc-200">{node.name || '/'}</span>
											</CollapsibleTrigger>
										</div>
										<CollapsibleContent>
											{@render treeNodes(node.children, depth + 1)}
										</CollapsibleContent>
									</Collapsible>
								</ContextMenuTrigger>
								<ContextMenuContent class="border-zinc-700 bg-zinc-900 text-zinc-100">
									<ContextMenuItem
										class="text-xs focus:bg-zinc-800"
										onclick={() => treeModals.openInit(node.path)}
									>Init here</ContextMenuItem>
									<ContextMenuItem
										class="text-xs focus:bg-zinc-800"
										onclick={() =>
											treeModals.openNewExom(node.path ? `${node.path}/notes` : 'notes')}
									>New exom</ContextMenuItem>
									<ContextMenuItem
										class="text-xs focus:bg-zinc-800"
										onclick={() => treeModals.openNewSession(node.path)}
									>New session</ContextMenuItem>
									<ContextMenuItem
										class="text-xs focus:bg-zinc-800"
										onclick={() => treeModals.openRename(node.path)}
									>Rename</ContextMenuItem>
								</ContextMenuContent>
							</ContextMenu>
						{:else}
							<ContextMenu>
								<ContextMenuTrigger class="block w-full text-left">
									<button
										type="button"
										data-tree-active={isActivePath(node.path) ? '' : undefined}
										class="flex w-full min-w-0 items-center gap-1.5 rounded-sm border border-transparent px-0.5 py-0.5 text-left hover:bg-zinc-800/80 {isActivePath(node.path)
											? activeRowClass()
											: ''} {node.archived ? 'opacity-50' : ''}"
										onclick={() => onNavigate(node.path)}
									>
										<Circle
											class="size-2.5 shrink-0 {exomDotClass(node.exom_kind)}"
											aria-hidden="true"
										/>
										<span class="min-w-0 flex-1 truncate text-[11px] text-zinc-100">{node.name}</span>
										{#if node.branches && node.branches.length > 0}
											<GitBranch
												class="size-3 shrink-0 text-zinc-600"
												aria-hidden="true"
											/>
										{/if}
										<Badge
											variant="secondary"
											class="h-4 shrink-0 px-1 font-mono text-[9px] tabular-nums text-zinc-300"
										>{node.fact_count}</Badge>
									</button>
								</ContextMenuTrigger>
								<ContextMenuContent class="border-zinc-700 bg-zinc-900 text-zinc-100">
									{#if node.exom_kind !== 'session'}
										<ContextMenuItem
											class="text-xs focus:bg-zinc-800"
											onclick={() => treeModals.openRename(node.path)}
										>Rename</ContextMenuItem>
									{:else}
										<ContextMenuItem
											class="text-xs focus:bg-zinc-800"
											onclick={() =>
												treeModals.openSessionLabel(node.path, labelForSession(node))}
										>Rename label…</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-zinc-800"
											onclick={notImplemented}
										>Close</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-zinc-800"
											onclick={notImplemented}
										>Archive</ContextMenuItem>
									{/if}
								</ContextMenuContent>
							</ContextMenu>
						{/if}
					</div>
				{/each}
			{/snippet}

			{#if root.kind === 'folder'}
				{#if root.children.length === 0}
					<p class="px-1 py-3 text-[11px] leading-relaxed text-zinc-500">
						Empty tree — run <span class="font-mono text-zinc-400">ray-exomem init</span> to create a project.
					</p>
				{:else}
					{@render treeNodes(root.children, 0)}
				{/if}
			{:else}
				<p class="px-1 py-2 text-[11px] text-zinc-500">Unexpected tree root (not a folder).</p>
			{/if}
		</div>
	{/if}
</div>
