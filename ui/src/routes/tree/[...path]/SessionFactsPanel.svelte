<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { Loader2 } from '@lucide/svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { ScrollArea } from '$lib/components/ui/scroll-area/index.js';
	import { fetchBranches, fetchFactsList, type BranchRow, type ListedFact } from '$lib/exomem.svelte';
	import FactsDataTable from './FactsDataTable.svelte';

	let { exomPath }: { exomPath: string } = $props();

	type Mode = 'switcher' | 'kanban' | 'timeline';

	const PALETTE = [
		'border-l-blue-500/80 bg-blue-950/25',
		'border-l-emerald-500/80 bg-emerald-950/25',
		'border-l-amber-500/80 bg-amber-950/25',
		'border-l-violet-500/80 bg-violet-950/25',
		'border-l-rose-500/80 bg-rose-950/25',
		'border-l-cyan-500/80 bg-cyan-950/25'
	];

	function branchColor(name: string) {
		let h = 0;
		for (let i = 0; i < name.length; i++) h = (h + name.charCodeAt(i) * (i + 1)) % PALETTE.length;
		return PALETTE[h]!;
	}

	function readMode(): Mode {
		const m = page.url.searchParams.get('mode');
		if (m === 'kanban' || m === 'timeline' || m === 'switcher') return m;
		return 'switcher';
	}

	let mode = $state<Mode>(readMode());
	let branches = $state<BranchRow[]>([]);
	let branchesLoading = $state(true);
	let selectedBranch = $state<string>('');

	let branchFilter = $state<Record<string, boolean>>({});

	let switcherFacts = $state<ListedFact[]>([]);
	let switcherLoading = $state(false);

	let kanbanFacts = $state<Record<string, ListedFact[]>>({});
	let kanbanLoading = $state(false);

	let timelineFacts = $state<ListedFact[]>([]);
	let timelineLoading = $state(false);

	function pushQuery(updates: Record<string, string | null | undefined>) {
		const u = new URL(page.url.href);
		for (const [k, v] of Object.entries(updates)) {
			if (v === undefined) continue;
			if (v === null || v === '') u.searchParams.delete(k);
			else u.searchParams.set(k, v);
		}
		goto(`${u.pathname}${u.search}`, { replaceState: true, keepFocus: true, noScroll: true });
	}

	$effect(() => {
		page.url.search;
		mode = readMode();
		const br = page.url.searchParams.get('branch');
		if (br) selectedBranch = br;
	});

	$effect(() => {
		exomPath;
		let cancelled = false;
		branchesLoading = true;
		fetchBranches(exomPath)
			.then((rows) => {
				if (cancelled) return;
				branches = rows.filter((b) => !b.archived);
				const filt: Record<string, boolean> = { ...branchFilter };
				for (const b of branches) {
					if (filt[b.name] === undefined) filt[b.name] = true;
				}
				branchFilter = filt;
				const want = page.url.searchParams.get('branch');
				const names = branches.map((b) => b.name);
				if (want && names.includes(want)) {
					selectedBranch = want;
				} else if (!selectedBranch || !names.includes(selectedBranch)) {
					selectedBranch = names[0] ?? '';
				}
			})
			.finally(() => {
				if (!cancelled) branchesLoading = false;
			});
		return () => {
			cancelled = true;
		};
	});

	function setMode(m: Mode) {
		mode = m;
		pushQuery({ mode: m });
	}

	function selectBranch(name: string) {
		selectedBranch = name;
		pushQuery({ branch: name });
	}

	$effect(() => {
		exomPath;
		mode;
		selectedBranch;
		if (mode !== 'switcher' || !selectedBranch) return;
		let cancelled = false;
		switcherLoading = true;
		fetchFactsList(exomPath, { branch: selectedBranch })
			.then((rows) => {
				if (!cancelled) switcherFacts = rows;
			})
			.finally(() => {
				if (!cancelled) switcherLoading = false;
			});
		return () => {
			cancelled = true;
		};
	});

	$effect(() => {
		exomPath;
		mode;
		branches;
		if (mode !== 'kanban' || branches.length === 0) return;
		let cancelled = false;
		kanbanLoading = true;
		Promise.all(branches.map((b) => fetchFactsList(exomPath, { branch: b.name })))
			.then((rowsList) => {
				if (cancelled) return;
				const next: Record<string, ListedFact[]> = {};
				branches.forEach((b, i) => {
					next[b.name] = rowsList[i] ?? [];
				});
				kanbanFacts = next;
			})
			.finally(() => {
				if (!cancelled) kanbanLoading = false;
			});
		return () => {
			cancelled = true;
		};
	});

	$effect(() => {
		exomPath;
		mode;
		if (mode !== 'timeline') return;
		let cancelled = false;
		timelineLoading = true;
		fetchFactsList(exomPath, { allBranches: true })
			.then((rows) => {
				if (!cancelled) timelineFacts = rows;
			})
			.finally(() => {
				if (!cancelled) timelineLoading = false;
			});
		return () => {
			cancelled = true;
		};
	});

	function toggleBranchFilter(name: string) {
		branchFilter = { ...branchFilter, [name]: !branchFilter[name] };
	}

	const timelineVisible = $derived.by(() =>
		timelineFacts.filter((f) => branchFilter[f.branch_name ?? ''] !== false)
	);
</script>

<div class="flex flex-col gap-3">
	<div class="flex flex-wrap items-center gap-2">
		<span class="text-[0.65rem] uppercase tracking-wide text-zinc-500">Mode</span>
		{#each ['switcher', 'kanban', 'timeline'] as m (m)}
			<Button
				size="sm"
				variant={mode === m ? 'default' : 'outline'}
				class="h-7 text-xs capitalize"
				onclick={() => setMode(m as Mode)}
			>
				{m}
			</Button>
		{/each}
	</div>

	{#if branchesLoading}
		<p class="flex items-center gap-2 text-sm text-zinc-500">
			<Loader2 class="size-4 animate-spin" /> Loading branches…
		</p>
	{:else if branches.length === 0}
		<p class="text-sm text-zinc-500">No branches for this session.</p>
	{:else if mode === 'switcher'}
		<div class="flex flex-wrap gap-2">
			{#each branches as b (b.branch_id)}
				<button type="button" onclick={() => selectBranch(b.name)}>
					<Badge
						variant={selectedBranch === b.name ? 'default' : 'outline'}
						class="cursor-pointer font-mono text-[11px]"
					>
						{b.name}
					</Badge>
				</button>
			{/each}
		</div>
		{#if selectedBranch}
			<FactsDataTable facts={switcherFacts} loading={switcherLoading} />
		{/if}
	{:else if mode === 'kanban'}
		{#if kanbanLoading}
			<p class="flex items-center gap-2 text-sm text-zinc-500">
				<Loader2 class="size-4 animate-spin" /> Loading branches…
			</p>
		{:else}
			<div class="flex min-h-[280px] flex-row gap-3 overflow-x-auto pb-2">
				{#each branches as b (b.branch_id)}
					<div class="flex w-[min(100%,280px)] shrink-0 flex-col rounded-md border border-zinc-700 bg-zinc-950/50">
						<div class="border-b border-zinc-700 px-2 py-2">
							<p class="font-mono text-xs text-zinc-100">{b.name}</p>
							<p class="text-[10px] text-zinc-500">
								{b.claimed_by ? `owner ${b.claimed_by}` : 'unclaimed'}
							</p>
						</div>
						<ScrollArea class="h-[min(50vh,420px)] p-2">
							<ul class="space-y-1.5 text-[11px] font-mono text-zinc-300">
								{#each kanbanFacts[b.name] ?? [] as f (f.fact_id)}
									<li class="rounded border border-zinc-800/80 bg-zinc-900/40 px-2 py-1">
										<span class="text-zinc-500">{f.predicate}</span>
										<span class="text-zinc-200"> · {f.value}</span>
									</li>
								{/each}
							</ul>
							{#if (kanbanFacts[b.name] ?? []).length === 0}
								<p class="text-xs text-zinc-600">No facts</p>
							{/if}
						</ScrollArea>
					</div>
				{/each}
			</div>
		{/if}
	{:else}
		<div class="flex flex-wrap gap-2">
			<span class="text-[0.65rem] uppercase tracking-wide text-zinc-500">Branches</span>
			{#each branches as b (b.branch_id)}
				<button type="button" onclick={() => toggleBranchFilter(b.name)}>
					<Badge
						variant={branchFilter[b.name] ? 'default' : 'outline'}
						class="cursor-pointer font-mono text-[10px]"
					>
						{b.name}
					</Badge>
				</button>
			{/each}
		</div>
		{#if timelineLoading}
			<p class="text-sm text-zinc-500">Loading timeline…</p>
		{:else}
			<div class="space-y-1">
				{#each timelineVisible as f (f.fact_id + (f.tx_time ?? ''))}
					<div
						class="flex flex-col gap-0.5 rounded-md border border-zinc-800 border-l-4 px-2 py-1.5 font-mono text-[11px] {branchColor(
							f.branch_name ?? '—'
						)}"
					>
						<div class="flex flex-wrap items-center gap-2 text-[10px] text-zinc-500">
							<span>{f.tx_time ?? '—'}</span>
							<Badge variant="outline" class="h-5 text-[9px]">{f.branch_name ?? '—'}</Badge>
						</div>
						<div class="text-zinc-200">
							<span class="text-zinc-500">{f.predicate}</span>
							<span> · {f.value}</span>
						</div>
					</div>
				{/each}
				{#if timelineVisible.length === 0}
					<p class="text-sm text-zinc-500">No facts for the selected branches.</p>
				{/if}
			</div>
		{/if}
	{/if}
</div>
