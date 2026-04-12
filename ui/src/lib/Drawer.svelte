<script lang="ts">
	import { Search, Settings, TreePine } from '@lucide/svelte';
	import { Sheet, SheetContent, SheetHeader, SheetTitle } from '$lib/components/ui/sheet/index.js';
	import { Separator } from '$lib/components/ui/separator/index.js';
	import { Tooltip, TooltipContent, TooltipTrigger } from '$lib/components/ui/tooltip/index.js';

	type Panel = 'tree' | 'search' | 'settings';

	let sheetOpen = $state(false);
	let panel = $state<Panel>('tree');

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
					onclick={openTree}
					aria-label="Open tree"
					{...props}
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
					onclick={openSearch}
					aria-label="Open search"
					{...props}
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
					onclick={openSettings}
					aria-label="Open settings"
					{...props}
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
		class="w-[min(100vw,22rem)] border-r border-zinc-700 bg-zinc-900 text-zinc-100 sm:max-w-md"
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
		<div class="min-h-0 flex-1 overflow-y-auto px-1 py-2 font-sans text-sm text-zinc-300">
			{#if panel === 'tree'}
				<p class="font-mono text-xs text-zinc-400">Tree placeholder — Phase 8 fills this</p>
			{:else if panel === 'search'}
				<p class="text-zinc-400">Search placeholder — Phase 8 fills this</p>
			{:else}
				<p class="text-zinc-400">Settings placeholder — Phase 8 fills this</p>
			{/if}
		</div>
	</SheetContent>
</Sheet>
