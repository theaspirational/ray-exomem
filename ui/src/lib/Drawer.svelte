<script lang="ts">
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { Loader2, Search, Settings, TreePine } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Separator } from '$lib/components/ui/separator/index.js';
	import { Sheet, SheetContent, SheetHeader, SheetTitle } from '$lib/components/ui/sheet/index.js';
	import { Tooltip, TooltipContent, TooltipTrigger } from '$lib/components/ui/tooltip/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import {
		apiInitFolder,
		apiNewBareExom,
		apiSessionNew
	} from '$lib/exomem.svelte';
	import { parent } from '$lib/path.svelte';
	import RenameModal from '$lib/RenameModal.svelte';
	import SessionLabelModal from '$lib/SessionLabelModal.svelte';
	import { treeModals } from '$lib/treeModals.svelte';
	import TreeDrawer from '$lib/TreeDrawer.svelte';

	type Panel = 'tree' | 'search' | 'settings';

	let sheetOpen = $state(false);
	let panel = $state<Panel>('tree');

	const currentTreePath = $derived.by((): string => {
		let pathname = String(page.url.pathname);
		if (base && pathname.startsWith(base)) {
			pathname = pathname.slice(base.length) || '/';
		}
		if (!pathname.startsWith('/tree')) return '';
		return pathname.slice('/tree'.length).replace(/^\/+/, '');
	});

	function openTree() {
		panel = 'tree';
		sheetOpen = true;
	}

	function openSearch() {
		panel = 'search';
		sheetOpen = true;
	}

	function openSettings() {
		panel = 'settings';
		sheetOpen = true;
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
</script>

<div
	class="flex h-full w-10 shrink-0 flex-col items-center gap-1 border-r border-zinc-700 bg-zinc-900 py-2"
	aria-label="Navigation rail"
>
	<Tooltip>
		<TooltipTrigger>
			{#snippet child({ props })}
				<button
					type="button"
					class="flex size-8 items-center justify-center rounded-md text-zinc-300 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
					aria-label="Open tree"
					{...props}
					onclick={openTree}
				>
					<TreePine class="size-4" />
				</button>
			{/snippet}
		</TooltipTrigger>
		<TooltipContent side="right">Tree</TooltipContent>
	</Tooltip>

	<Tooltip>
		<TooltipTrigger>
			{#snippet child({ props })}
				<button
					type="button"
					class="flex size-8 items-center justify-center rounded-md text-zinc-300 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
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
					class="flex size-8 items-center justify-center rounded-md text-zinc-300 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
					aria-label="Open settings"
					{...props}
					onclick={openSettings}
				>
					<Settings class="size-4" />
				</button>
			{/snippet}
		</TooltipTrigger>
		<TooltipContent side="right">Settings</TooltipContent>
	</Tooltip>
</div>

<Sheet bind:open={sheetOpen}>
	<SheetContent
		side="left"
		showCloseButton={true}
		class="h-full min-h-0 w-[min(100vw,22rem)] border-r border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md"
	>
		<SheetHeader>
			<SheetTitle class="font-sans text-zinc-100">
				{#if panel === 'tree'}
					Tree
				{:else if panel === 'search'}
					Search
				{:else}
					Settings
				{/if}
			</SheetTitle>
		</SheetHeader>
		<Separator class="bg-zinc-700" />
		<div class="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden px-1 py-2 font-sans text-sm text-zinc-300">
			{#if sheetOpen && panel === 'tree'}
				<TreeDrawer
					currentPath={currentTreePath}
					refreshSignal={treeModals.refreshTree}
					onNavigate={(path) => goto(`${base}/tree/${path.replace(/^\/+/, '')}`)}
				/>
			{:else if panel === 'search'}
				<p class="text-zinc-400">Search placeholder — Phase 8 fills this</p>
			{:else}
				<p class="text-zinc-400">Settings placeholder — Phase 8 fills this</p>
			{/if}
		</div>
	</SheetContent>
</Sheet>

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
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Init project here</Dialog.Title>
			<Dialog.Description class="text-zinc-400">
				Scaffolds <span class="font-mono">main</span> plus <span class="font-mono">sessions/</span> at this path.
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-zinc-500" for="drawer-init-path">Path (slash-separated)</label>
			<Input
				id="drawer-init-path"
				bind:value={treeModals.initPath}
				class="border-zinc-700 bg-zinc-950 font-mono text-sm"
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
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New bare exom</Dialog.Title>
			<Dialog.Description class="text-zinc-400">
				Creates an exom leaf at the given path (folders are created as needed).
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-zinc-500" for="drawer-exom-path">Path</label>
			<Input
				id="drawer-exom-path"
				bind:value={treeModals.exomPathField}
				class="border-zinc-700 bg-zinc-950 font-mono text-sm"
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
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New session</Dialog.Title>
			<Dialog.Description class="text-zinc-400">
				Project path must be the folder that contains <span class="font-mono">main</span> (not the exom leaf).
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-3 py-2">
			<div>
				<label class="text-xs text-zinc-500" for="drawer-sess-proj">Project path</label>
				<Input
					id="drawer-sess-proj"
					bind:value={treeModals.sessionProjectPath}
					class="mt-1 border-zinc-700 bg-zinc-950 font-mono text-sm"
				/>
			</div>
			<div>
				<label class="text-xs text-zinc-500" for="drawer-sess-lbl">Label</label>
				<Input
					id="drawer-sess-lbl"
					bind:value={treeModals.sessionLabelField}
					class="mt-1 border-zinc-700 bg-zinc-950 text-sm"
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
