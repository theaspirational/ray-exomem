<script lang="ts">
	import {
		ChevronRight,
		Circle,
		ChevronsDownUp,
		Eye,
		EyeOff,
		FilePlus,
		FolderPlus,
		GitBranch,
		Loader2,
		RefreshCw
	} from '@lucide/svelte';
	import { tick } from 'svelte';
	import { toast } from 'svelte-sonner';

	import { auth } from '$lib/auth.svelte';
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
	import {
		graphFolderVisState,
		graphViz,
		toggleGraphExom,
		toggleGraphFolderVisibility
	} from '$lib/graphExomVisibility.svelte';
	import { fetchTree, type TreeExom, type TreeNode } from '$lib/exomem.svelte';
	import { treeModals } from '$lib/treeModals.svelte';

	let {
		currentPath = '',
		refreshSignal = 0,
		onNavigate,
		inlineEyeIcons = false,
		onTreeRoot
	}: {
		currentPath?: string;
		refreshSignal?: number;
		onNavigate: (path: string) => void;
		inlineEyeIcons?: boolean;
		onTreeRoot?: (root: TreeNode) => void;
	} = $props();

	let root = $state<TreeNode | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	/** Folder open state; only explicit `true` opens (default closed). Reset on currentPath change; manual toggles persist until then. */
	let folderOpen = $state<Record<string, boolean>>({});
	let lastRevealForPath = $state<string | undefined>(undefined);
	/** When true, suppress auto-reveal until currentPath actually changes. */
	let suppressReveal = $state(false);
	/** Saved folder state before collapse-all, for toggle restore. */
	let savedFolderOpen = $state<Record<string, boolean> | null>(null);

	const defaultWritableBase = $derived(currentPath || auth.user?.email || '');

	function suggestedNewExomPath(): string {
		const base = defaultWritableBase.trim();
		return base ? `${base}/notes` : 'notes';
	}

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
		if (root) onTreeRoot?.(root);
	});

	$effect(() => {
		const path = currentPath;
		root;
		if (!root) return;
		if (path === lastRevealForPath) return;
		if (suppressReveal) {
			suppressReveal = false;
			lastRevealForPath = path;
			return;
		}
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
		if (kind === 'project_main' || kind === 'project-main') return 'fill-branch-active text-branch-active';
		if (kind === 'session') return 'fill-fact-base text-fact-base';
		return 'fill-muted-foreground text-muted-foreground';
	}

	function isActivePath(path: string): boolean {
		return Boolean(currentPath && path === currentPath);
	}

	function activeRowClass(): string {
		return 'bg-primary/15 text-foreground';
	}

	function labelForSession(node: TreeExom): string {
		const s = node.session as { label?: string } | null | undefined;
		return s?.label?.trim() || node.name;
	}

	function toggleCollapseAll() {
		const isCollapsed = Object.keys(folderOpen).length === 0 || Object.values(folderOpen).every((v) => !v);
		if (isCollapsed && savedFolderOpen) {
			folderOpen = { ...savedFolderOpen };
			savedFolderOpen = null;
			suppressReveal = true;
			lastRevealForPath = undefined;
		} else {
			savedFolderOpen = { ...folderOpen };
			folderOpen = {};
			suppressReveal = true;
			lastRevealForPath = undefined;
		}
	}
</script>

<div class="flex items-center justify-end gap-0.5 px-1 py-1">
	<button
		type="button"
		class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
		title="New exom"
		onclick={() => treeModals.openNewExom(suggestedNewExomPath())}
	>
		<FilePlus class="size-3.5" />
	</button>
	<button
		type="button"
		class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
		title="Init project here"
		onclick={() => treeModals.openInit(defaultWritableBase)}
	>
		<FolderPlus class="size-3.5" />
	</button>
	<button
		type="button"
		class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
		title="Refresh tree"
		onclick={() => void loadTree()}
	>
		<RefreshCw class="size-3.5" />
	</button>
	<button
		type="button"
		class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
		title="Collapse all"
		onclick={toggleCollapseAll}
	>
		<ChevronsDownUp class="size-3.5" />
	</button>
</div>

<div class="flex min-h-0 flex-1 flex-col gap-1 font-mono text-xs text-muted-foreground">
	{#if loading}
		<div class="space-y-2 px-1 py-1" aria-busy="true">
			<div class="flex items-center gap-2 text-muted-foreground">
				<Loader2 class="size-4 shrink-0 animate-spin text-muted-foreground" aria-hidden="true" />
				<span class="text-[11px]">Loading tree…</span>
			</div>
			{#each Array.from({ length: 6 }) as _, i (i)}
				<div class="flex items-center gap-2">
					<div class="h-3.5 w-3.5 shrink-0 animate-pulse rounded-sm bg-border"></div>
					<div class="h-3 max-w-[70%] flex-1 animate-pulse rounded bg-border/80"></div>
					<div class="h-4 w-8 shrink-0 animate-pulse rounded bg-border/60"></div>
				</div>
			{/each}
		</div>
	{:else if error}
		<div class="flex flex-col gap-2 px-1 py-2 text-muted-foreground">
			<p class="text-[11px] leading-relaxed">Failed to load tree</p>
			<p class="text-[10px] text-muted-foreground/80">{error}</p>
			<Button
				variant="outline"
				size="sm"
				class="h-7 border-border bg-card text-foreground"
				onclick={() => void loadTree()}
			>
				<RefreshCw class="mr-1 size-3" />
				Retry
			</Button>
		</div>
	{:else if root}
		<div class="min-h-0 flex-1 overflow-y-auto thin-scrollbar px-0.5 py-0.5">
			{#snippet treeNodes(nodes: TreeNode[])}
				{#each nodes as node (node.kind + '\0' + node.path)}
					<div>
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
												class="group/trigger flex min-w-0 min-h-0 flex-1 items-center gap-1 rounded-sm px-0.5 py-0.5 text-left text-muted-foreground hover:bg-card/80 [&[data-state=open]>span:first-of-type]:rotate-90"
											>
												<span
													class="inline-flex size-3.5 shrink-0 select-none items-center justify-center text-muted-foreground transition-transform"
													aria-hidden="true"
												>
													<ChevronRight class="size-3.5" />
												</span>
												<span
													class="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground"
													>{node.name || '/'}</span
												>
											</CollapsibleTrigger>
											{#if inlineEyeIcons}
												{@const fv = graphFolderVisState(node.path, graphViz.exomVis)}
												<button
													type="button"
													class="flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
													aria-label="Toggle visibility for folder {node.name || '/'}"
													onclick={(e) => {
														e.stopPropagation();
														e.preventDefault();
														toggleGraphFolderVisibility(node.path);
													}}
												>
													{#if fv === 'all'}
														<Eye class="size-3" />
													{:else if fv === 'none'}
														<EyeOff class="size-3 opacity-80" />
													{:else}
														<Eye class="size-3 opacity-50" />
													{/if}
												</button>
											{/if}
										</div>
										<CollapsibleContent>
											<div class="ml-[8px] border-l border-border/60 pl-2">
												{@render treeNodes(node.children)}
											</div>
										</CollapsibleContent>
									</Collapsible>
								</ContextMenuTrigger>
								<ContextMenuContent class="border-border bg-card text-foreground">
									<ContextMenuItem
										class="text-xs focus:bg-muted"
										onclick={() => treeModals.openInit(node.path)}
									>Init here</ContextMenuItem>
									<ContextMenuItem
										class="text-xs focus:bg-muted"
										onclick={() =>
											treeModals.openNewExom(node.path ? `${node.path}/notes` : 'notes')}
									>New exom</ContextMenuItem>
									<ContextMenuItem
										class="text-xs focus:bg-muted"
										onclick={() => treeModals.openNewSession(node.path)}
									>New session</ContextMenuItem>
									<ContextMenuItem
										class="text-xs focus:bg-muted"
										onclick={() => treeModals.openRename(node.path)}
									>Rename</ContextMenuItem>
								</ContextMenuContent>
							</ContextMenu>
						{:else}
							<ContextMenu>
								<ContextMenuTrigger class="block w-full text-left">
									<div
										class="flex w-full min-w-0 items-center gap-0.5 {node.archived ? 'opacity-50' : ''}"
									>
										<button
											type="button"
											data-tree-active={isActivePath(node.path) ? '' : undefined}
											class="flex min-w-0 min-h-0 flex-1 items-center gap-1.5 rounded-sm border border-transparent px-0.5 py-0.5 text-left font-normal hover:bg-card/80 {isActivePath(
												node.path
											)
												? activeRowClass()
												: ''}"
											onclick={() => onNavigate(node.path)}
										>
											<Circle
												class="size-2.5 shrink-0 {exomDotClass(node.exom_kind)}"
												aria-hidden="true"
											/>
											<span class="min-w-0 flex-1 truncate text-[11px] text-foreground"
												>{node.name}</span
											>
											{#if node.branches && node.branches.length > 0}
												<GitBranch
													class="size-3 shrink-0 text-muted-foreground"
													aria-hidden="true"
												/>
											{/if}
											<Badge
												variant="secondary"
												class="h-4 shrink-0 px-1 font-mono text-[9px] tabular-nums"
											>{node.fact_count}</Badge>
										</button>
										{#if inlineEyeIcons}
											<button
												type="button"
												class="flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-card hover:text-foreground"
												aria-label="Toggle visibility for {node.name}"
												onclick={(e) => {
													e.stopPropagation();
													toggleGraphExom(node.path);
												}}
											>
												{#if graphViz.exomVis[node.path]}
													<Eye class="size-3" />
												{:else}
													<EyeOff class="size-3 opacity-80" />
												{/if}
											</button>
										{/if}
									</div>
								</ContextMenuTrigger>
								<ContextMenuContent class="border-border bg-card text-foreground">
									{#if node.exom_kind !== 'session'}
										<ContextMenuItem
											class="text-xs focus:bg-muted"
											onclick={() => treeModals.openRename(node.path)}
										>Rename</ContextMenuItem>
									{:else}
										<ContextMenuItem
											class="text-xs focus:bg-muted"
											onclick={() =>
												treeModals.openSessionLabel(node.path, labelForSession(node))}
										>Rename label…</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-muted"
											onclick={notImplemented}
										>Close</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-muted"
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
					<p class="px-1 py-3 text-[11px] leading-relaxed text-muted-foreground">
						Empty tree — run <span class="font-mono text-foreground/80">ray-exomem init</span> to create
						a project.
					</p>
				{:else}
					{@render treeNodes(root.children)}
				{/if}
			{:else}
				<p class="px-1 py-2 text-[11px] text-muted-foreground">Unexpected tree root (not a folder).</p>
			{/if}
		</div>
	{/if}
</div>
