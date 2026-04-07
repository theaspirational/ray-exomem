<script lang="ts">
	import { onMount } from 'svelte';
	import {
		Archive,
		ArchiveRestore,
		ChevronDown,
		ChevronRight,
		CircleAlert,
		Database,
		Download,
		LoaderCircle,
		Merge,
		Pencil,
		Plus,
		Trash2,
		X
	} from '@lucide/svelte';

	import { Badge } from '$lib/components/ui/badge';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import {
		createExom,
		manageExom,
		mergeExoms,
		exportBackup,
		clearDatabase
	} from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';
	import type { ExomEntry } from '$lib/types';

	// ---------------------------------------------------------------------------
	// State
	// ---------------------------------------------------------------------------

	let errorMessage = $state<string | null>(null);
	let successMessage = $state<string | null>(null);

	// Create form
	let showCreateForm = $state(false);
	let createName = $state('');
	let createDescription = $state('');
	let createCopyFrom = $state('');
	let creating = $state(false);

	// Merge form
	let showMergeForm = $state(false);
	let mergeSources = $state<string[]>([]);
	let mergeTarget = $state('');
	let mergeDescription = $state('');
	let mergeStrategy = $state<'union' | 'prefer_left' | 'prefer_right' | 'flag_conflicts'>('union');
	let merging = $state(false);

	// Inline rename
	let renamingExom = $state<string | null>(null);
	let renameValue = $state('');
	let renaming = $state(false);

	// Confirm delete
	let confirmDeleteExom = $state<string | null>(null);
	let deleting = $state(false);

	// Archived section
	let showArchived = $state(false);

	// Danger zone
	let confirmClear = $state(false);
	let clearing = $state(false);

	// Busy states for individual actions
	let busyAction = $state<string | null>(null);

	// ---------------------------------------------------------------------------
	// Derived
	// ---------------------------------------------------------------------------

	const activeExoms = $derived(app.exoms.filter((e) => !e.archived));
	const archivedExoms = $derived(app.exoms.filter((e) => e.archived));

	// ---------------------------------------------------------------------------
	// Helpers
	// ---------------------------------------------------------------------------

	function formatDate(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString('en-US', {
			month: 'short',
			day: 'numeric',
			year: 'numeric'
		});
	}

	function flash(msg: string, type: 'success' | 'error') {
		if (type === 'success') {
			successMessage = msg;
			errorMessage = null;
		} else {
			errorMessage = msg;
			successMessage = null;
		}
		setTimeout(() => {
			successMessage = null;
			errorMessage = null;
		}, 4000);
	}

	// ---------------------------------------------------------------------------
	// Actions
	// ---------------------------------------------------------------------------

	async function handleCreate() {
		if (!createName.trim()) return;
		creating = true;
		try {
			await createExom(createName.trim(), createDescription.trim(), createCopyFrom || undefined);
			await app.refreshExoms();
			flash(`Exom "${createName.trim()}" created`, 'success');
			createName = '';
			createDescription = '';
			createCopyFrom = '';
			showCreateForm = false;
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			creating = false;
		}
	}

	async function handleRename(oldName: string) {
		if (!renameValue.trim() || renameValue.trim() === oldName) {
			renamingExom = null;
			return;
		}
		renaming = true;
		try {
			await manageExom(oldName, 'rename', { new_name: renameValue.trim() });
			if (app.selectedExom === oldName) {
				app.switchExom(renameValue.trim());
			}
			await app.refreshExoms();
			flash(`Renamed "${oldName}" to "${renameValue.trim()}"`, 'success');
			renamingExom = null;
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			renaming = false;
		}
	}

	async function handleExport(name: string) {
		busyAction = `export-${name}`;
		try {
			await exportBackup(name);
			flash(`Exported "${name}"`, 'success');
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			busyAction = null;
		}
	}

	async function handleArchive(name: string) {
		busyAction = `archive-${name}`;
		try {
			await manageExom(name, 'archive');
			if (app.selectedExom === name) {
				const remaining = activeExoms.filter((e) => e.name !== name);
				if (remaining.length > 0) {
					app.switchExom(remaining[0].name);
				}
			}
			await app.refreshExoms();
			flash(`Archived "${name}"`, 'success');
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			busyAction = null;
		}
	}

	async function handleUnarchive(name: string) {
		busyAction = `unarchive-${name}`;
		try {
			await manageExom(name, 'unarchive');
			await app.refreshExoms();
			flash(`Restored "${name}"`, 'success');
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			busyAction = null;
		}
	}

	async function handleDelete(name: string) {
		deleting = true;
		try {
			await manageExom(name, 'delete', { confirm: true });
			if (app.selectedExom === name) {
				const remaining = app.exoms.filter((e) => e.name !== name && !e.archived);
				if (remaining.length > 0) {
					app.switchExom(remaining[0].name);
				}
			}
			await app.refreshExoms();
			flash(`Deleted "${name}"`, 'success');
			confirmDeleteExom = null;
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			deleting = false;
		}
	}

	async function handleMerge() {
		if (mergeSources.length < 2 || !mergeTarget.trim()) return;
		merging = true;
		try {
			await mergeExoms(mergeSources, mergeTarget.trim(), mergeDescription.trim(), mergeStrategy);
			await app.refreshExoms();
			flash(`Merged into "${mergeTarget.trim()}"`, 'success');
			mergeSources = [];
			mergeTarget = '';
			mergeDescription = '';
			mergeStrategy = 'union';
			showMergeForm = false;
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			merging = false;
		}
	}

	function toggleMergeSource(name: string) {
		if (mergeSources.includes(name)) {
			mergeSources = mergeSources.filter((s) => s !== name);
		} else {
			mergeSources = [...mergeSources, name];
		}
	}

	async function handleClearDatabase() {
		clearing = true;
		try {
			const result = await clearDatabase(app.selectedExom);
			flash(`Cleared ${result.tuples_removed} tuples from "${app.selectedExom}"`, 'success');
			confirmClear = false;
		} catch (e) {
			flash(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			clearing = false;
		}
	}

	function startRename(exom: ExomEntry) {
		renamingExom = exom.name;
		renameValue = exom.name;
	}

	// ---------------------------------------------------------------------------
	// Lifecycle
	// ---------------------------------------------------------------------------

	onMount(() => {
		app.refreshExoms();
	});
</script>

<div class="mx-auto flex max-w-3xl flex-col gap-6 p-6">
	<!-- Flash messages -->
	{#if errorMessage}
		<div class="flex items-center gap-2 rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
			<CircleAlert class="size-4 shrink-0" />
			{errorMessage}
		</div>
	{/if}
	{#if successMessage}
		<div class="flex items-center gap-2 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-400">
			{successMessage}
		</div>
	{/if}

	<!-- Header -->
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
		<div class="flex items-center gap-3">
			<Database class="size-5 text-muted-foreground" />
			<h1 class="text-xl font-semibold">Exoms</h1>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			<Button
				variant="outline"
				size="sm"
				onclick={() => {
					showMergeForm = !showMergeForm;
					if (showMergeForm) showCreateForm = false;
				}}
			>
				<Merge class="size-4" />
				Merge
			</Button>
			<Button
				size="sm"
				onclick={() => {
					showCreateForm = !showCreateForm;
					if (showCreateForm) showMergeForm = false;
				}}
			>
				<Plus class="size-4" />
				New Exom
			</Button>
		</div>
	</div>

	<!-- Create KB form -->
	{#if showCreateForm}
		<div class="flex flex-col gap-3 rounded-lg border border-border/60 bg-card p-4">
			<h2 class="text-sm font-medium">Create Exom</h2>
			<Input
				bind:value={createName}
				placeholder="lowercase-no-spaces"
				onkeydown={(e: KeyboardEvent) => {
					if (e.key === 'Enter') handleCreate();
				}}
			/>
			<Input
				bind:value={createDescription}
				placeholder="Description (optional)"
			/>
			<div class="flex flex-col gap-1">
				<label class="text-xs text-muted-foreground" for="create-copy-from">Copy from (optional)</label>
				<select
					id="create-copy-from"
					class="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
					bind:value={createCopyFrom}
				>
					<option value="">None</option>
					{#each activeExoms as exom (exom.name)}
						<option value={exom.name}>{exom.name}</option>
					{/each}
				</select>
			</div>
			<div class="flex flex-wrap items-center gap-2">
				<Button size="sm" onclick={handleCreate} disabled={creating || !createName.trim()}>
					{#if creating}
						<LoaderCircle class="size-4 animate-spin" />
					{/if}
					Create
				</Button>
				<Button variant="ghost" size="sm" onclick={() => (showCreateForm = false)}>
					Cancel
				</Button>
			</div>
		</div>
	{/if}

	<!-- Merge form -->
	{#if showMergeForm}
		<div class="flex flex-col gap-3 rounded-lg border border-border/60 bg-card p-4">
			<h2 class="text-sm font-medium">Merge Exoms</h2>

			<div class="flex flex-col gap-1">
				<span class="text-xs text-muted-foreground">Source Exoms (select at least 2)</span>
				<div class="flex flex-wrap gap-1.5">
					{#each activeExoms as exom (exom.name)}
						<button
							class="rounded-full border px-3 py-1 text-xs transition-colors {mergeSources.includes(exom.name) ? 'border-primary bg-primary/15 text-primary' : 'border-border/60 text-muted-foreground hover:border-border'}"
							onclick={() => toggleMergeSource(exom.name)}
						>
							{exom.name}
						</button>
					{/each}
				</div>
			</div>

			<Input
				bind:value={mergeTarget}
				placeholder="Target Exom name"
			/>
			<Input
				bind:value={mergeDescription}
				placeholder="Description for merged Exom"
			/>

			<div class="flex flex-col gap-1">
				<label class="text-xs text-muted-foreground" for="merge-strategy">Strategy</label>
				<select
					id="merge-strategy"
					class="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
					bind:value={mergeStrategy}
				>
					<option value="union">Union</option>
					<option value="prefer_left">Prefer Left</option>
					<option value="prefer_right">Prefer Right</option>
					<option value="flag_conflicts">Flag Conflicts</option>
				</select>
			</div>

			<div class="flex flex-wrap items-center gap-2">
				<Button
					size="sm"
					onclick={handleMerge}
					disabled={merging || mergeSources.length < 2 || !mergeTarget.trim()}
				>
					{#if merging}
						<LoaderCircle class="size-4 animate-spin" />
					{/if}
					Merge
				</Button>
				<Button variant="ghost" size="sm" onclick={() => (showMergeForm = false)}>
					Cancel
				</Button>
			</div>
		</div>
	{/if}

	<!-- Active KBs -->
	<div class="flex flex-col gap-2">
		<h2 class="text-sm font-medium text-muted-foreground">Active</h2>

		{#if activeExoms.length === 0}
			<p class="py-8 text-center text-sm text-muted-foreground">No exoms found.</p>
		{/if}

		{#each activeExoms as exom (exom.name)}
			{@const isCurrent = exom.name === app.selectedExom}
			{@const isDeleting = confirmDeleteExom === exom.name}

			<div
				class="flex flex-col gap-2 rounded-lg border border-border/60 p-3 transition-colors {isCurrent ? 'border-l-2 border-l-primary bg-primary/5' : ''}"
			>
				<!-- Main row -->
				<div class="flex items-center justify-between gap-3">
					<div class="flex min-w-0 flex-1 items-center gap-2">
						{#if renamingExom === exom.name}
							<Input
								class="h-7 max-w-48 text-sm"
								bind:value={renameValue}
								onkeydown={(e: KeyboardEvent) => {
									if (e.key === 'Enter') handleRename(exom.name);
									if (e.key === 'Escape') (renamingExom = null);
								}}
								autofocus
							/>
							<Button
								variant="ghost"
								size="xs"
								onclick={() => handleRename(exom.name)}
								disabled={renaming}
							>
								{#if renaming}
									<LoaderCircle class="size-3 animate-spin" />
								{:else}
									Save
								{/if}
							</Button>
							<Button variant="ghost" size="xs" onclick={() => (renamingExom = null)}>
								<X class="size-3" />
							</Button>
						{:else}
							<span class="truncate font-medium">{exom.name}</span>
							{#if isCurrent}
								<Badge variant="secondary" class="shrink-0 text-[10px]">current</Badge>
							{/if}
						{/if}
					</div>

					<!-- Actions -->
					<div class="flex shrink-0 items-center gap-1">
						{#if !isCurrent && renamingExom !== exom.name}
							<Button
								variant="ghost"
								size="xs"
								onclick={() => app.switchExom(exom.name)}
							>
								Switch
							</Button>
						{/if}
						{#if renamingExom !== exom.name}
							<Button variant="ghost" size="xs" onclick={() => startRename(exom)}>
								<Pencil class="size-3" />
							</Button>
							<Button
								variant="ghost"
								size="xs"
								onclick={() => handleExport(exom.name)}
								disabled={busyAction === `export-${exom.name}`}
							>
								{#if busyAction === `export-${exom.name}`}
									<LoaderCircle class="size-3 animate-spin" />
								{:else}
									<Download class="size-3" />
								{/if}
							</Button>
							<Button
								variant="ghost"
								size="xs"
								onclick={() => handleArchive(exom.name)}
								disabled={busyAction === `archive-${exom.name}`}
							>
								{#if busyAction === `archive-${exom.name}`}
									<LoaderCircle class="size-3 animate-spin" />
								{:else}
									<Archive class="size-3" />
								{/if}
							</Button>
							<Button
								variant="ghost"
								size="xs"
								onclick={() => (confirmDeleteExom = exom.name)}
							>
								<Trash2 class="size-3 text-destructive" />
							</Button>
						{/if}
					</div>
				</div>

				<!-- Description + date -->
				{#if renamingExom !== exom.name}
					<div class="flex items-center justify-between text-xs text-muted-foreground">
						<span class="truncate">{exom.description || 'No description'}</span>
						<span class="shrink-0">{formatDate(exom.created_at)}</span>
					</div>
				{/if}

				<!-- Delete confirmation -->
				{#if isDeleting}
					<div class="flex items-center gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs">
						<span class="text-destructive">Permanently delete "{exom.name}"?</span>
						<Button
							variant="destructive"
							size="xs"
							onclick={() => handleDelete(exom.name)}
							disabled={deleting}
						>
							{#if deleting}
								<LoaderCircle class="size-3 animate-spin" />
							{/if}
							Delete
						</Button>
						<Button variant="ghost" size="xs" onclick={() => (confirmDeleteExom = null)}>
							Cancel
						</Button>
					</div>
				{/if}
			</div>
		{/each}
	</div>

	<!-- Archived KBs -->
	{#if archivedExoms.length > 0}
		<div class="flex flex-col gap-2">
			<button
				class="flex items-center gap-2 text-sm font-medium text-muted-foreground hover:text-foreground"
				onclick={() => (showArchived = !showArchived)}
			>
				{#if showArchived}
					<ChevronDown class="size-4" />
				{:else}
					<ChevronRight class="size-4" />
				{/if}
				Archived ({archivedExoms.length})
			</button>

			{#if showArchived}
				<div class="flex flex-col gap-2">
					{#each archivedExoms as exom (exom.name)}
						{@const isDeleting = confirmDeleteExom === exom.name}

						<div class="flex flex-col gap-2 rounded-lg border border-border/60 p-3 opacity-70">
							<div class="flex items-center justify-between gap-3">
								<div class="flex flex-wrap items-center gap-2">
									<span class="font-medium">{exom.name}</span>
									<Badge variant="outline" class="text-[10px]">archived</Badge>
								</div>
								<div class="flex items-center gap-1">
									<Button
										variant="ghost"
										size="xs"
										onclick={() => handleUnarchive(exom.name)}
										disabled={busyAction === `unarchive-${exom.name}`}
									>
										{#if busyAction === `unarchive-${exom.name}`}
											<LoaderCircle class="size-3 animate-spin" />
										{:else}
											<ArchiveRestore class="size-3" />
										{/if}
										Restore
									</Button>
									<Button
										variant="ghost"
										size="xs"
										onclick={() => (confirmDeleteExom = exom.name)}
									>
										<Trash2 class="size-3 text-destructive" />
									</Button>
								</div>
							</div>

							<div class="flex items-center justify-between text-xs text-muted-foreground">
								<span class="truncate">{exom.description || 'No description'}</span>
								<span class="shrink-0">
									Archived {exom.archived_at ? formatDate(exom.archived_at) : ''}
								</span>
							</div>

							{#if isDeleting}
								<div class="flex items-center gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs">
									<span class="text-destructive">Permanently delete "{exom.name}"?</span>
									<Button
										variant="destructive"
										size="xs"
										onclick={() => handleDelete(exom.name)}
										disabled={deleting}
									>
										{#if deleting}
											<LoaderCircle class="size-3 animate-spin" />
										{/if}
										Delete
									</Button>
									<Button variant="ghost" size="xs" onclick={() => (confirmDeleteExom = null)}>
										Cancel
									</Button>
								</div>
							{/if}
						</div>
					{/each}
				</div>
			{/if}
		</div>
	{/if}

	<!-- Danger zone -->
	<div class="flex flex-col gap-3 rounded-lg border border-destructive/30 p-4">
		<h2 class="text-sm font-medium text-destructive">Danger Zone</h2>
		<div class="flex items-center justify-between">
			<div class="flex flex-col gap-0.5">
				<span class="text-sm">Clear current exom</span>
				<span class="text-xs text-muted-foreground">
					Remove all facts and rules from "{app.selectedExom}". This cannot be undone.
				</span>
			</div>
			{#if confirmClear}
				<div class="flex flex-wrap items-center gap-2">
					<Button
						variant="destructive"
						size="sm"
						onclick={handleClearDatabase}
						disabled={clearing}
					>
						{#if clearing}
							<LoaderCircle class="size-4 animate-spin" />
						{/if}
						Confirm Clear
					</Button>
					<Button variant="ghost" size="sm" onclick={() => (confirmClear = false)}>
						Cancel
					</Button>
				</div>
			{:else}
				<Button
					variant="destructive"
					size="sm"
					onclick={() => (confirmClear = true)}
				>
					Clear Database
				</Button>
			{/if}
		</div>
	</div>
</div>
