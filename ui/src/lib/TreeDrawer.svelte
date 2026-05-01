<script lang="ts">
	import {
		ChevronRight,
		Circle,
		Eye,
		EyeOff,
		GitBranch
	} from '@lucide/svelte';
	import { tick } from 'svelte';
	import { toast } from 'svelte-sonner';

	import { Badge } from '$lib/components/ui/badge/index.js';
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
	import type { TreeExom, TreeNode } from '$lib/exomem.svelte';
	import { isProjectMainExomKind, treeExomDisplayName } from '$lib/path.svelte';
	import { treeModals } from '$lib/treeModals.svelte';

	let {
		root,
		currentPath = '',
		onNavigate,
		inlineEyeIcons = false
	}: {
		root: TreeNode | null;
		currentPath?: string;
		onNavigate: (path: string) => void;
		inlineEyeIcons?: boolean;
	} = $props();

	/** Folder open state; only explicit `true` opens (default closed). */
	let folderOpen = $state<Record<string, boolean>>({});
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

	$effect(() => {
		const path = currentPath;
		if (!root) return;
		if (path === lastRevealForPath) return;
		lastRevealForPath = path;
		const reveal = revealFolderPaths(root, path);
		if (reveal.length === 0) return;
		folderOpen = { ...folderOpen, ...Object.fromEntries(reveal.map((p) => [p, true])) };
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
		if (isProjectMainExomKind(kind)) return 'fill-branch-active text-branch-active';
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

	function labelForExom(node: TreeExom): string {
		if (node.exom_kind === 'session') return labelForSession(node);
		return treeExomDisplayName(node);
	}
</script>

<div class="flex min-h-0 flex-1 flex-col gap-1 font-mono text-xs text-muted-foreground">
	{#if root && root.kind === 'folder'}
		{#if root.children.length === 0}
			<p class="px-1 py-2 text-[11px] leading-relaxed text-muted-foreground/80">Empty</p>
		{:else}
			<div class="min-h-0 flex-1 px-0.5 py-0.5">
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
											onclick={() =>
												treeModals.openNewFolder(
													node.path ? `${node.path}/new-folder` : 'new-folder'
												)}
										>New folder</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-muted"
											onclick={() =>
												treeModals.openNewExom(node.path ? `${node.path}/notes` : 'notes')}
										>New exom</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-muted"
											onclick={() => treeModals.openInit(node.path)}
										>Init project</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-muted"
											onclick={() => treeModals.openNewSession(node.path)}
										>New session</ContextMenuItem>
										<ContextMenuItem
											class="text-xs focus:bg-muted"
											onclick={() => treeModals.openRename(node.path)}
										>Rename</ContextMenuItem>
										<ContextMenuItem
											class="text-xs text-destructive focus:bg-destructive/10 focus:text-destructive"
											onclick={() => treeModals.openDelete(node.path, 'folder')}
										>Delete…</ContextMenuItem>
									</ContextMenuContent>
								</ContextMenu>
							{:else}
								{@const label = labelForExom(node)}
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
													>{label}</span
												>
												{#if node.acl_mode === 'co-edit'}
													<Badge
														variant="outline"
														class="h-4 shrink-0 border-primary/40 bg-primary/10 px-1 font-mono text-[9px] uppercase tracking-wide text-primary"
														title="co-edit · anyone with access can write the shared trunk"
													>co-edit</Badge>
												{/if}
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
													aria-label="Toggle visibility for {label}"
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
										<ContextMenuItem
											class="text-xs text-destructive focus:bg-destructive/10 focus:text-destructive"
											onclick={() => treeModals.openDelete(node.path, 'exom')}
										>Delete…</ContextMenuItem>
									</ContextMenuContent>
								</ContextMenu>
							{/if}
						</div>
					{/each}
				{/snippet}

				{@render treeNodes(root.children)}
			</div>
		{/if}
	{/if}
</div>
