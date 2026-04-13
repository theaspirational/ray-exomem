<script lang="ts">
	import { base } from '$app/paths';
	import { goto } from '$app/navigation';
	import { Folder, Brain, Loader2 } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import EmptyState from '$lib/components/EmptyState.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Card } from '$lib/components/ui/card/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import {
		apiInitFolder,
		apiNewBareExom,
		apiSessionNew,
		type TreeNode
	} from '$lib/exomem.svelte';

	let { node }: { node: Extract<TreeNode, { kind: 'folder' }> } = $props();

	let openInit = $state(false);
	let openExom = $state(false);
	let openSession = $state(false);
	let busy = $state(false);
	let fieldInit = $state('');
	let fieldExom = $state('');
	let fieldSessionLabel = $state('');
	let fieldProjectPath = $state('');

	$effect(() => {
		node.path;
		fieldInit = node.path;
		fieldExom = node.path ? `${node.path}/notes` : 'notes';
		fieldProjectPath = node.path;
		fieldSessionLabel = 'adhoc';
	});

	const sortedChildren = $derived.by(() => {
		const ch = [...node.children];
		const exoms = ch.filter((c) => c.kind === 'exom');
		const folders = ch.filter((c) => c.kind === 'folder');
		const byName = (a: TreeNode, b: TreeNode) => a.name.localeCompare(b.name);
		exoms.sort(byName);
		folders.sort(byName);
		return [...exoms, ...folders];
	});

	function goChild(n: TreeNode) {
		const p = n.path.startsWith('/') ? n.path.slice(1) : n.path;
		goto(`${base}/tree/${p}`);
	}

	function doInit() {
		actorPrompt.run(async () => {
			busy = true;
			try {
				await apiInitFolder(fieldInit.trim());
				toast.success('Initialized');
				openInit = false;
				goto(`${base}/tree/${fieldInit.replace(/^\//, '')}`, { invalidateAll: true });
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
				await apiNewBareExom(fieldExom.trim());
				toast.success('Exom created');
				openExom = false;
				goto(`${base}/tree/${fieldExom.replace(/^\//, '')}`, { invalidateAll: true });
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
					project_path: fieldProjectPath.trim(),
					type: 'multi',
					label: fieldSessionLabel.trim()
				});
				toast.success('Session created');
				openSession = false;
				const sp = r.session_path.replace(/^\//, '');
				goto(`${base}/tree/${sp}`, { invalidateAll: true });
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Session failed');
			} finally {
				busy = false;
			}
		});
	}
</script>

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap gap-2">
		<Button size="sm" variant="secondary" onclick={() => (openInit = true)}>Init here</Button>
		<Button size="sm" variant="secondary" onclick={() => (openExom = true)}>New exom</Button>
		<Button size="sm" variant="secondary" onclick={() => (openSession = true)}>New session</Button>
	</div>

	{#if sortedChildren.length === 0}
		<EmptyState icon={Folder} message="No children" />
	{:else}
		<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
			{#each sortedChildren as ch (ch.path)}
				<button
					type="button"
					class="text-left"
					onclick={() => goChild(ch)}
				>
					<Card
						class="border-zinc-700 bg-zinc-900/50 transition-colors hover:border-zinc-500 hover:bg-zinc-800/40"
						size="sm"
					>
						<div class="flex flex-col gap-2">
							<div class="flex items-start gap-2">
								{#if ch.kind === 'folder'}
									<Folder class="mt-0.5 size-4 shrink-0 text-amber-400/90" />
								{:else}
									<Brain class="mt-0.5 size-4 shrink-0 text-emerald-400/90" />
								{/if}
								<div class="min-w-0 flex-1">
									<p class="truncate font-medium text-zinc-100">{ch.name}</p>
									<p class="mt-0.5 text-[11px] text-zinc-500">
										{#if ch.kind === 'exom'}
											{ch.fact_count} facts
										{:else}
											{ch.children?.length ?? 0} children
										{/if}
									</p>
								</div>
							</div>
						</div>
					</Card>
				</button>
			{/each}
		</div>
	{/if}
</div>

<Dialog.Root bind:open={openInit}>
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Init project here</Dialog.Title>
			<Dialog.Description class="text-zinc-400">
				Scaffolds <span class="font-mono">main</span> plus <span class="font-mono">sessions/</span> at this path.
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-zinc-500" for="init-path">Path (slash-separated)</label>
			<Input id="init-path" bind:value={fieldInit} class="border-zinc-700 bg-zinc-950 font-mono text-sm" />
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (openInit = false)}>Cancel</Button>
			<Button disabled={busy || !fieldInit.trim()} onclick={() => void doInit()}>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Run init
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={openExom}>
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New bare exom</Dialog.Title>
			<Dialog.Description class="text-zinc-400">Creates an exom leaf at the given path (folders are created as needed).</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-2 py-2">
			<label class="text-xs text-zinc-500" for="exom-path">Path</label>
			<Input id="exom-path" bind:value={fieldExom} class="border-zinc-700 bg-zinc-950 font-mono text-sm" />
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (openExom = false)}>Cancel</Button>
			<Button disabled={busy || !fieldExom.trim()} onclick={() => void doExom()}>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Create
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root bind:open={openSession}>
	<Dialog.Content class="border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>New session</Dialog.Title>
			<Dialog.Description class="text-zinc-400">
				Project path must be the folder that contains <span class="font-mono">main</span> (not the exom leaf).
			</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-3 py-2">
			<div>
				<label class="text-xs text-zinc-500" for="sess-proj">Project path</label>
				<Input id="sess-proj" bind:value={fieldProjectPath} class="mt-1 border-zinc-700 bg-zinc-950 font-mono text-sm" />
			</div>
			<div>
				<label class="text-xs text-zinc-500" for="sess-lbl">Label</label>
				<Input id="sess-lbl" bind:value={fieldSessionLabel} class="mt-1 border-zinc-700 bg-zinc-950 text-sm" />
			</div>
		</div>
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (openSession = false)}>Cancel</Button>
			<Button
				disabled={busy || !fieldProjectPath.trim() || !fieldSessionLabel.trim()}
				onclick={() => void doSession()}
			>
				{#if busy}<Loader2 class="mr-1 size-3 animate-spin" />{/if}
				Create session
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
