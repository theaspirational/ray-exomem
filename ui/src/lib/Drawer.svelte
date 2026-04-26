<script lang="ts">
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { Loader2, Search, TreePine, Waypoints } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Tooltip, TooltipContent, TooltipTrigger } from '$lib/components/ui/tooltip/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import { commandPaletteState } from '$lib/commandPaletteState.svelte';
	import { initGraphExomVisFromTree } from '$lib/graphExomVisibility.svelte';
	import { apiInitFolder, apiNewBareExom, apiSessionNew } from '$lib/exomem.svelte';
	import { parent } from '$lib/path.svelte';
	import RenameModal from '$lib/RenameModal.svelte';
	import SessionLabelModal from '$lib/SessionLabelModal.svelte';
	import { treeModals } from '$lib/treeModals.svelte';
	import TreeDrawer from '$lib/TreeDrawer.svelte';

	const STORAGE_KEY = 'ray-exomem:drawer-state';

	function stripBasePath(pathname: string): string {
		if (base && pathname.startsWith(base)) {
			return pathname.slice(base.length) || '/';
		}
		return pathname;
	}

	function routeBucket(p: string): 'home' | 'tree' | 'graph' | 'other' {
		if (p === '/' || p === '') return 'home';
		if (p === '/graph' || p.startsWith('/graph/')) return 'graph';
		if (p.startsWith('/tree')) return 'tree';
		return 'other';
	}

	function defaultExpandedFor(b: 'home' | 'tree' | 'graph' | 'other'): boolean {
		if (b === 'home' || b === 'other') return false;
		return true;
	}

	function readPrefs(): Record<string, boolean> {
		if (!browser) return {};
		try {
			const raw = localStorage.getItem(STORAGE_KEY);
			return raw ? (JSON.parse(raw) as Record<string, boolean>) : {};
		} catch {
			return {};
		}
	}

	function writePref(bucket: string, value: boolean) {
		if (!browser) return;
		const p = { ...readPrefs(), [bucket]: value };
		localStorage.setItem(STORAGE_KEY, JSON.stringify(p));
	}

	let expanded = $state(false);

	const pathnameNorm = $derived.by(() => stripBasePath(String(page.url.pathname)));
	const isGraph = $derived(pathnameNorm === '/graph' || pathnameNorm.startsWith('/graph/'));

	$effect(() => {
		if (!browser) return;
		pathnameNorm;
		const b = routeBucket(pathnameNorm);
		const pref = readPrefs()[b];
		expanded = pref !== undefined ? pref : defaultExpandedFor(b);
	});

	function toggleDrawer() {
		if (!browser) return;
		const b = routeBucket(pathnameNorm);
		const next = !expanded;
		expanded = next;
		writePref(b, next);
	}

	const currentTreePath = $derived.by((): string => {
		if (!pathnameNorm.startsWith('/tree')) return '';
		return pathnameNorm.slice('/tree'.length).replace(/^\/+/, '');
	});

	function openSearch() {
		commandPaletteState.show();
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
				void goto(`${base}/tree/${treeModals.initPath.replace(/^\//, '')}`, { invalidateAll: true });
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Init failed');
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
				void goto(`${base}/tree/${treeModals.exomPathField.replace(/^\//, '')}`, { invalidateAll: true });
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
			toggleDrawer();
		};
		window.addEventListener('keydown', h);
		return () => window.removeEventListener('keydown', h);
	});
</script>

<div class="flex h-full min-h-0 shrink-0">
	<div
		class="flex h-full w-10 shrink-0 flex-col items-center gap-1 border-r border-border bg-background py-2"
		aria-label="Navigation rail"
	>
		<Tooltip>
			<TooltipTrigger>
				{#snippet child({ props })}
					<button
						type="button"
						class="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-card hover:text-foreground"
						aria-label={expanded ? 'Collapse tree' : 'Expand tree'}
						{...props}
						onclick={toggleDrawer}
					>
						<TreePine class="size-4" />
					</button>
				{/snippet}
			</TooltipTrigger>
			<TooltipContent side="right">Tree (⌘B)</TooltipContent>
		</Tooltip>

		<Tooltip>
			<TooltipTrigger>
				{#snippet child({ props })}
					<button
						type="button"
						class="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-card hover:text-foreground"
						aria-label="Open search"
						{...props}
						onclick={openSearch}
					>
						<Search class="size-4" />
					</button>
				{/snippet}
			</TooltipTrigger>
			<TooltipContent side="right">Search</TooltipContent>
		</Tooltip>

		<Tooltip>
			<TooltipTrigger>
				{#snippet child({ props })}
					<button
						type="button"
						class="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-card hover:text-foreground"
						aria-label="Global graph"
						{...props}
						onclick={() => goto(`${base}/graph`)}
					>
						<Waypoints class="size-4" />
					</button>
				{/snippet}
			</TooltipTrigger>
			<TooltipContent side="right">Graph</TooltipContent>
		</Tooltip>
	</div>

	<div
		class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden border-r border-border bg-card transition-[width] duration-200 ease-out {expanded
			? 'w-[min(100vw,280px)]'
			: 'w-0 border-transparent'}"
		aria-hidden={!expanded}
	>
		<div class="flex h-full min-h-0 w-[min(100vw,280px)] min-w-[280px] flex-col">
			<div
				class="shrink-0 border-b border-border px-2 py-1.5 font-sans text-sm font-medium text-foreground"
			>
				Tree
			</div>
			<div class="min-h-0 flex-1 overflow-hidden px-0.5 py-1 font-sans text-sm text-muted-foreground">
				<TreeDrawer
					currentPath={currentTreePath}
					refreshSignal={treeModals.refreshTree}
					inlineEyeIcons={isGraph}
					onTreeRoot={isGraph ? initGraphExomVisFromTree : undefined}
					onNavigate={(path) => goto(`${base}/tree/${path.replace(/^\/+/, '')}`)}
				/>
			</div>
		</div>
	</div>
</div>

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
				Scaffolds <span class="font-mono">main</span> plus <span class="font-mono">sessions/</span> at this path.
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-muted-foreground" for="drawer-init-path">Path (slash-separated)</label>
			<Input
				id="drawer-init-path"
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

<Dialog.Root bind:open={treeModals.exomOpen}>
	<Dialog.Content class="border-border bg-card text-foreground sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New bare exom</Dialog.Title>
			<Dialog.Description class="text-muted-foreground">
				Creates an exom leaf at the given path (folders are created as needed).
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-muted-foreground" for="drawer-exom-path">Path</label>
			<Input
				id="drawer-exom-path"
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
				Project path must be the folder that contains <span class="font-mono">main</span> (not the exom leaf).
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-3 py-2">
			<div>
				<label class="text-xs text-muted-foreground" for="drawer-sess-proj">Project path</label>
				<Input
					id="drawer-sess-proj"
					bind:value={treeModals.sessionProjectPath}
					class="mt-1 border-border bg-background font-mono text-sm"
				/>
			</div>
			<div>
				<label class="text-xs text-muted-foreground" for="drawer-sess-lbl">Label</label>
				<Input
					id="drawer-sess-lbl"
					bind:value={treeModals.sessionLabelField}
					class="mt-1 border-border bg-background text-sm"
				/>
			</div>
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (treeModals.sessionOpen = false)}>Cancel</Button>
			<Button
				disabled={busy || !treeModals.sessionProjectPath.trim() || !treeModals.sessionLabelField.trim()}
				onclick={() => void doSession()}
			>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Create session
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
