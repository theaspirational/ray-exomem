<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import {
		ChevronDown,
		FilePlus,
		FolderPlus,
		Loader2,
		PanelLeftClose,
		PanelLeftOpen,
		RefreshCw,
		Waypoints
	} from '@lucide/svelte';
	import { toast } from 'svelte-sonner';

	import { auth } from '$lib/auth.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Tooltip, TooltipContent, TooltipTrigger } from '$lib/components/ui/tooltip/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import { initGraphExomVisFromTree } from '$lib/graphExomVisibility.svelte';
	import {
		apiDeletePath,
		apiInitFolder,
		apiNewBareExom,
		apiNewFolder,
		apiSessionNew,
		fetchTree,
		type TreeNode
	} from '$lib/exomem.svelte';
	import { parent } from '$lib/path.svelte';
	import RenameModal from '$lib/RenameModal.svelte';
	import SessionLabelModal from '$lib/SessionLabelModal.svelte';
	import TreeDrawer from '$lib/TreeDrawer.svelte';
	import { treeModals } from '$lib/treeModals.svelte';

	const COLLAPSE_KEY = 'ray-exomem:sidebar-collapsed';

	function stripBasePath(pathname: string): string {
		if (base && pathname.startsWith(base)) {
			return pathname.slice(base.length) || '/';
		}
		return pathname;
	}

	const pathnameNorm = $derived.by(() => stripBasePath(String(page.url.pathname)));
	const isGraph = $derived(pathnameNorm === '/graph' || pathnameNorm.startsWith('/graph/'));
	const currentTreePath = $derived.by((): string => {
		if (!pathnameNorm.startsWith('/tree')) return '';
		return pathnameNorm.slice('/tree'.length).replace(/^\/+/, '');
	});

	let collapsed = $state(false);

	$effect(() => {
		if (!browser) return;
		try {
			const raw = localStorage.getItem(COLLAPSE_KEY);
			if (raw === '1') collapsed = true;
		} catch {
			// ignore
		}
	});

	function setCollapsed(value: boolean) {
		collapsed = value;
		if (!browser) return;
		try {
			localStorage.setItem(COLLAPSE_KEY, value ? '1' : '0');
		} catch {
			// ignore
		}
	}

	function toggleCollapsed() {
		setCollapsed(!collapsed);
	}

	function isEditableKeyTarget(t: EventTarget | null): boolean {
		if (!(t instanceof HTMLElement)) return false;
		if (t.isContentEditable) return true;
		const el = t.closest('input, textarea, [contenteditable="true"]');
		return el != null;
	}

	onMount(() => {
		const h = (e: KeyboardEvent) => {
			if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'b') return;
			if (isEditableKeyTarget(e.target)) return;
			e.preventDefault();
			toggleCollapsed();
		};
		window.addEventListener('keydown', h);
		return () => window.removeEventListener('keydown', h);
	});

	// Tree fetch -----------------------------------------------------------------

	let root = $state<TreeNode | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

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
		treeModals.refreshTree;
		void loadTree();
	});

	$effect(() => {
		if (root && isGraph) initGraphExomVisFromTree(root);
	});

	// Track last-touched tree path so header actions default to where the user
	// last was, not just to their email root.
	$effect(() => {
		const p = currentTreePath;
		if (p) treeModals.setLastTouched(p);
	});

	// Section partitioning -------------------------------------------------------

	const userEmail = $derived(auth.user?.email ?? '');

	function makeSyntheticRoot(children: TreeNode[]): TreeNode {
		return { kind: 'folder', name: '', path: '', children };
	}

	const personalRoot = $derived.by((): TreeNode | null => {
		if (!root || root.kind !== 'folder' || !userEmail) return null;
		const node = root.children.find((n) => n.path === userEmail);
		if (!node || node.kind !== 'folder') return makeSyntheticRoot([]);
		return makeSyntheticRoot(node.children);
	});

	const publicRoot = $derived.by((): TreeNode | null => {
		if (!root || root.kind !== 'folder') return null;
		const node = root.children.find((n) => n.path === 'public');
		if (!node || node.kind !== 'folder' || node.children.length === 0) return null;
		return makeSyntheticRoot(node.children);
	});

	const otherRoot = $derived.by((): TreeNode | null => {
		if (!root || root.kind !== 'folder') return null;
		const match = root.children.filter((n) => {
			if (n.path === userEmail) return false;
			if (n.path === 'public') return false;
			return true;
		});
		return match.length ? makeSyntheticRoot(match) : null;
	});

	const sectionOpen = $state({ personal: true, public: true, other: true });

	// Action handlers ------------------------------------------------------------

	const defaultWritableBase = $derived(
		currentTreePath || treeModals.lastTouchedPath || userEmail || ''
	);

	function suggestedNewExomPath(): string {
		const baseP = defaultWritableBase.trim();
		return baseP ? `${baseP}/notes` : 'notes';
	}

	function navigateTo(path: string) {
		void goto(`${base}/tree/${path.replace(/^\/+/, '')}`);
	}

	let busy = $state(false);

	function doInit() {
		actorPrompt.run(async () => {
			busy = true;
			try {
				await apiInitFolder(treeModals.initPath.trim());
				toast.success('Initialized');
				treeModals.initOpen = false;
				treeModals.bumpTree();
				void goto(`${base}/tree/${treeModals.initPath.replace(/^\//, '')}`, {
					invalidateAll: true
				});
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Init failed');
			} finally {
				busy = false;
			}
		});
	}

	function doFolder() {
		actorPrompt.run(async () => {
			busy = true;
			try {
				const target = treeModals.folderPathField.trim();
				await apiNewFolder(target);
				toast.success('Folder created');
				treeModals.folderOpen = false;
				treeModals.bumpTree();
				void goto(`${base}/tree/${target.replace(/^\//, '')}`, { invalidateAll: true });
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Create folder failed');
			} finally {
				busy = false;
			}
		});
	}

	function doExom() {
		actorPrompt.run(async () => {
			busy = true;
			try {
				await apiNewBareExom(treeModals.exomPathField.trim());
				toast.success('Exom created');
				treeModals.exomOpen = false;
				treeModals.bumpTree();
				void goto(`${base}/tree/${treeModals.exomPathField.replace(/^\//, '')}`, {
					invalidateAll: true
				});
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Create failed');
			} finally {
				busy = false;
			}
		});
	}

	function doSession() {
		actorPrompt.run(async () => {
			busy = true;
			try {
				const r = await apiSessionNew({
					project_path: treeModals.sessionProjectPath.trim(),
					type: 'multi',
					label: treeModals.sessionLabelField.trim()
				});
				toast.success('Session created');
				treeModals.sessionOpen = false;
				treeModals.bumpTree();
				const sp = r.session_path.replace(/^\//, '');
				void goto(`${base}/tree/${sp}`, { invalidateAll: true });
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Session failed');
			} finally {
				busy = false;
			}
		});
	}

	function onRenameConfirm(newSegment: string) {
		const p = treeModals.renamePath;
		const par = parent(p);
		const newSlash = par ? `${par}/${newSegment}` : newSegment;
		treeModals.bumpTree();
		void goto(`${base}/tree/${newSlash.replace(/^\//, '')}`, { invalidateAll: true });
	}

	function doDelete() {
		const target = treeModals.deletePath.trim();
		if (!target) return;
		actorPrompt.run(async () => {
			busy = true;
			try {
				const r = await apiDeletePath(target);
				toast.success(`Deleted ${target}`);
				treeModals.deleteOpen = false;
				treeModals.bumpTree();
				// If we were viewing the deleted subtree (or one of its exoms),
				// fall back to the tree root so the page doesn't keep trying to
				// query a vanished exom.
				const here = currentTreePath;
				const removed = new Set([target, ...(r.removed_exoms ?? [])]);
				const onDeletedPath =
					here === target ||
					here.startsWith(`${target}/`) ||
					Array.from(removed).some((p) => here === p || here.startsWith(`${p}/`));
				if (onDeletedPath) {
					void goto(`${base}/tree/`, { invalidateAll: true });
				}
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Delete failed');
			} finally {
				busy = false;
			}
		});
	}
</script>

<aside
	class="flex h-full min-h-0 shrink-0 flex-col border-r border-border bg-background transition-[width] duration-200 ease-out {collapsed
		? 'w-10'
		: 'w-[260px]'}"
	aria-label="Memory sidebar"
>
	{#if collapsed}
		<div class="flex h-11 shrink-0 items-center justify-center border-b border-border">
			<Tooltip>
				<TooltipTrigger>
					{#snippet child({ props })}
						<button
							type="button"
							class="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
							aria-label="Expand sidebar"
							{...props}
							onclick={() => setCollapsed(false)}
						>
							<PanelLeftOpen class="size-4" />
						</button>
					{/snippet}
				</TooltipTrigger>
				<TooltipContent side="right">Expand sidebar (⌘B)</TooltipContent>
			</Tooltip>
		</div>
		<div class="flex flex-1 flex-col items-center gap-1 py-2">
			<Tooltip>
				<TooltipTrigger>
					{#snippet child({ props })}
						<a
							class="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
							aria-label="Graph view"
							href="{base}/graph"
							{...props}
						>
							<Waypoints class="size-4" />
						</a>
					{/snippet}
				</TooltipTrigger>
				<TooltipContent side="right">Graph</TooltipContent>
			</Tooltip>
		</div>
	{:else}
		<div
			class="flex h-11 shrink-0 items-center gap-1 border-b border-border px-3 font-sans text-foreground"
		>
			<a
				href="{base}/"
				class="shrink-0 font-serif text-sm tracking-tight text-foreground hover:text-primary"
				title="ray-exomem · welcome">ray-exomem</a
			>
			<span class="flex-1"></span>
			<Tooltip>
				<TooltipTrigger>
					{#snippet child({ props })}
						<button
							type="button"
							class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground"
							aria-label="New exom"
							{...props}
							onclick={() => treeModals.openNewExom(suggestedNewExomPath())}
						>
							<FilePlus class="size-3.5" />
						</button>
					{/snippet}
				</TooltipTrigger>
				<TooltipContent side="bottom">New exom</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger>
					{#snippet child({ props })}
						<button
							type="button"
							class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground"
							aria-label="New folder"
							{...props}
							onclick={() =>
								treeModals.openNewFolder(
									defaultWritableBase ? `${defaultWritableBase}/new-folder` : 'new-folder'
								)}
						>
							<FolderPlus class="size-3.5" />
						</button>
					{/snippet}
				</TooltipTrigger>
				<TooltipContent side="bottom">New folder</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger>
					{#snippet child({ props })}
						<button
							type="button"
							class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground"
							aria-label="Refresh tree"
							{...props}
							onclick={() => void loadTree()}
						>
							<RefreshCw class="size-3.5 {loading ? 'animate-spin' : ''}" />
						</button>
					{/snippet}
				</TooltipTrigger>
				<TooltipContent side="bottom">Refresh</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger>
					{#snippet child({ props })}
						<button
							type="button"
							class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground"
							aria-label="Collapse sidebar"
							{...props}
							onclick={() => setCollapsed(true)}
						>
							<PanelLeftClose class="size-3.5" />
						</button>
					{/snippet}
				</TooltipTrigger>
				<TooltipContent side="bottom">Collapse (⌘B)</TooltipContent>
			</Tooltip>
		</div>

		<div class="min-h-0 flex-1 overflow-y-auto thin-scrollbar px-1 py-1">
			{#if loading}
				<div class="space-y-2 px-1 py-1" aria-busy="true">
					<div class="flex items-center gap-2 text-muted-foreground">
						<Loader2 class="size-4 shrink-0 animate-spin" aria-hidden="true" />
						<span class="text-[11px]">Loading tree…</span>
					</div>
					{#each Array.from({ length: 5 }) as _, i (i)}
						<div class="flex items-center gap-2">
							<div class="h-3.5 w-3.5 shrink-0 animate-pulse rounded-sm bg-border"></div>
							<div class="h-3 max-w-[70%] flex-1 animate-pulse rounded bg-border/80"></div>
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
						class="h-7 border-border bg-background text-foreground"
						onclick={() => void loadTree()}
					>
						<RefreshCw class="mr-1 size-3" />
						Retry
					</Button>
				</div>
			{:else}
				{#snippet section(label: string, openKey: 'personal' | 'public' | 'other', sectionRoot: TreeNode | null, emptyHint: string | null = null, emptyAction: { label: string; path: string } | null = null, tooltip: string | null = null)}
					{#if sectionRoot}
						{@const isEmpty = sectionRoot.kind === 'folder' && sectionRoot.children.length === 0}
						<div class="mt-1 first:mt-0">
							<button
								type="button"
								class="group flex w-full items-center gap-1 rounded px-1 py-1 text-left text-[10px] font-medium uppercase tracking-wider text-muted-foreground hover:bg-secondary/40 hover:text-foreground"
								aria-expanded={sectionOpen[openKey]}
								title={tooltip ?? undefined}
								onclick={() => (sectionOpen[openKey] = !sectionOpen[openKey])}
							>
								<ChevronDown
									class="size-3 shrink-0 transition-transform {sectionOpen[openKey]
										? ''
										: '-rotate-90'}"
									aria-hidden="true"
								/>
								<span class="flex-1 truncate">{label}</span>
							</button>
							{#if sectionOpen[openKey]}
								{#if isEmpty}
									<div class="px-2 pb-2 pt-1">
										{#if emptyHint}
											<p class="text-[11px] leading-relaxed text-muted-foreground/80">
												{emptyHint}
											</p>
										{/if}
										{#if emptyAction}
											<button
												type="button"
												class="mt-1.5 inline-flex items-center gap-1 rounded border border-border bg-background px-2 py-1 text-[11px] text-foreground hover:bg-secondary"
												onclick={() => treeModals.openNewFolder(emptyAction.path)}
											>
												<FolderPlus class="size-3" />
												{emptyAction.label}
											</button>
										{/if}
									</div>
								{:else}
									<div class="px-0.5">
										<TreeDrawer
											root={sectionRoot}
											currentPath={currentTreePath}
											onNavigate={navigateTo}
											inlineEyeIcons={isGraph}
										/>
									</div>
								{/if}
							{/if}
						</div>
					{/if}
				{/snippet}

				{@render section(
					'Personal',
					'personal',
					personalRoot,
					'Empty — your private memory lives here.',
					userEmail ? { label: 'New folder', path: `${userEmail}/notes` } : null
				)}
				{@render section(
					'Public',
					'public',
					publicRoot,
					null,
					null,
					'Read by everyone. Fork to contribute.'
				)}
				{@render section('Other', 'other', otherRoot)}
			{/if}
		</div>

		<div class="shrink-0 border-t border-border px-2 py-1.5">
			<a
				href="{base}/graph"
				class="flex items-center gap-2 rounded px-1 py-1 text-[11px] text-muted-foreground hover:bg-secondary hover:text-foreground"
			>
				<Waypoints class="size-3.5" />
				<span>Graph view</span>
			</a>
		</div>
	{/if}
</aside>

<RenameModal
	bind:open={treeModals.renameOpen}
	path={treeModals.renamePath}
	onClose={() => {
		treeModals.renameOpen = false;
	}}
	onConfirm={onRenameConfirm}
/>

<SessionLabelModal
	bind:open={treeModals.sessionLabelOpen}
	sessionPath={treeModals.sessionLabelPath}
	currentLabel={treeModals.sessionLabelCurrent}
	onClose={() => {
		treeModals.sessionLabelOpen = false;
		treeModals.bumpTree();
	}}
/>

<Dialog.Root bind:open={treeModals.initOpen}>
	<Dialog.Content class="border-border bg-card text-foreground sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Init project here</Dialog.Title>
			<Dialog.Description class="text-muted-foreground">
				Scaffolds <span class="font-mono">main</span> plus <span class="font-mono">sessions/</span> at
				this path.
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-muted-foreground" for="sidebar-init-path">Path (slash-separated)</label
			>
			<Input
				id="sidebar-init-path"
				bind:value={treeModals.initPath}
				class="border-border bg-background font-mono text-sm"
			/>
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (treeModals.initOpen = false)}>Cancel</Button>
			<Button disabled={busy || !treeModals.initPath.trim()} onclick={() => void doInit()}>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Run init
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={treeModals.folderOpen}>
	<Dialog.Content class="border-border bg-card text-foreground sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New folder</Dialog.Title>
			<Dialog.Description class="text-muted-foreground">
				Creates an empty folder at the given path. Use right-click → <span class="font-mono"
					>Init project</span
				> on a folder to scaffold <span class="font-mono">main</span> + <span class="font-mono"
					>sessions/</span
				>.
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-muted-foreground" for="sidebar-folder-path">Path</label>
			<Input
				id="sidebar-folder-path"
				bind:value={treeModals.folderPathField}
				class="border-border bg-background font-mono text-sm"
			/>
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (treeModals.folderOpen = false)}>Cancel</Button>
			<Button
				disabled={busy || !treeModals.folderPathField.trim()}
				onclick={() => void doFolder()}
			>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Create
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={treeModals.exomOpen}>
	<Dialog.Content class="border-border bg-card text-foreground sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New bare exom</Dialog.Title>
			<Dialog.Description class="text-muted-foreground">
				Creates an exom leaf at the given path (folders are created as needed).
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-muted-foreground" for="sidebar-exom-path">Path</label>
			<Input
				id="sidebar-exom-path"
				bind:value={treeModals.exomPathField}
				class="border-border bg-background font-mono text-sm"
			/>
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (treeModals.exomOpen = false)}>Cancel</Button>
			<Button disabled={busy || !treeModals.exomPathField.trim()} onclick={() => void doExom()}>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Create
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={treeModals.sessionOpen}>
	<Dialog.Content class="border-border bg-card text-foreground sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New session</Dialog.Title>
			<Dialog.Description class="text-muted-foreground">
				Project path must be the folder that contains <span class="font-mono">main</span> (not the exom
				leaf).
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-3 py-2">
			<div>
				<label class="text-xs text-muted-foreground" for="sidebar-sess-proj">Project path</label>
				<Input
					id="sidebar-sess-proj"
					bind:value={treeModals.sessionProjectPath}
					class="mt-1 border-border bg-background font-mono text-sm"
				/>
			</div>
			<div>
				<label class="text-xs text-muted-foreground" for="sidebar-sess-lbl">Label</label>
				<Input
					id="sidebar-sess-lbl"
					bind:value={treeModals.sessionLabelField}
					class="mt-1 border-border bg-background text-sm"
				/>
			</div>
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (treeModals.sessionOpen = false)}>Cancel</Button>
			<Button
				disabled={busy ||
					!treeModals.sessionProjectPath.trim() ||
					!treeModals.sessionLabelField.trim()}
				onclick={() => void doSession()}
			>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Create session
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={treeModals.deleteOpen}>
	<Dialog.Content class="border-border bg-card text-foreground sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Delete {treeModals.deleteKind}?</Dialog.Title>
			<Dialog.Description class="text-muted-foreground">
				Are you sure you want to delete
				<span class="font-mono text-foreground">{treeModals.deletePath}</span>?
				{#if treeModals.deleteKind === 'folder'}
					<br />This permanently removes the folder and every exom inside it.
				{:else}
					<br />This permanently removes the exom and all of its facts, observations, beliefs, and
					branches.
				{/if}
				<br />This cannot be undone.
			</Dialog.Description>
		</Dialog.Header>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (treeModals.deleteOpen = false)}>Cancel</Button>
			<Button
				variant="destructive"
				disabled={busy || !treeModals.deletePath.trim()}
				onclick={() => doDelete()}
			>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Delete
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
