<script lang="ts">
	import { browser } from '$app/environment';
	import { goto, invalidateAll } from '$app/navigation';
	import { base } from '$app/paths';
	import { GitFork, Loader2 } from '@lucide/svelte';
	import { toast } from 'svelte-sonner';
	import { actorPrompt } from '$lib/actorPrompt.svelte';
	import { auth } from '$lib/auth.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import LoadingState from '$lib/components/LoadingState.svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { formatRelativeTime } from '$lib/formatRelativeTime';
	import NotebookEntity from '$lib/Notebook/NotebookEntity.svelte';
	import NotebookSection from '$lib/Notebook/NotebookSection.svelte';
	import RightRailAnchors from '$lib/Notebook/RightRailAnchors.svelte';
	import { entityForFactId } from '$lib/predicateRendering.svelte';
	import {
		ApiActionError,
		exportBackupText,
		fetchBranches,
		fetchFactsList,
		fetchRelationGraph,
		forkExom,
		parseFactsFromExport,
		setExomMode,
		unarchiveSessionExom,
		type AclMode,
		type BranchRow,
		type TreeExom
	} from '$lib/exomem.svelte';
	import type { FactEntry } from '$lib/types';
	import GraphPanel from './GraphPanel.svelte';
	import RulesPanel from './RulesPanel.svelte';
	import SessionFactsPanel from './SessionFactsPanel.svelte';
	import TimelinePanel from './TimelinePanel.svelte';

	let {
		node,
		sessionModes = false,
		readOnly = false,
		contentDimmed = false,
		showUnarchive = false
	}: {
		node: TreeExom;
		sessionModes?: boolean;
		readOnly?: boolean;
		contentDimmed?: boolean;
		showUnarchive?: boolean;
	} = $props();

	const railSections = [
		{ id: 'facts', label: 'Facts' },
		{ id: 'timeline', label: 'Timeline' },
		{ id: 'branches', label: 'Branches' },
		{ id: 'rules', label: 'Rules' },
		{ id: 'connections', label: 'Connections' }
	];

	function factGroupKey(f: FactEntry): string {
		if (f.factId) return f.factId;
		return f.predicate + '::' + f.terms.join('::');
	}

	const lastSegment = $derived.by(() => {
		const s = node.path.split('/').filter(Boolean);
		return s[s.length - 1] ?? node.name;
	});

	const titleText = $derived(
		lastSegment
			.split(/[-_]+/)
			.filter(Boolean)
			.map((p) => p.charAt(0).toUpperCase() + p.slice(1).toLowerCase())
			.join(' ')
	);

	let flatFacts = $state<FactEntry[]>([]);
	let factsLoading = $state(true);
	let factsErr = $state<string | null>(null);
	let factsRetry = $state(0);

	const headerBlurb = $derived.by(() => {
		for (const f of flatFacts) {
			if (f.predicate === 'exom/summary' && f.terms[0]) return f.terms[0]!.trim();
		}
		for (const f of flatFacts) {
			if (f.predicate === 'exom/description' && f.terms[0]) return f.terms[0]!.trim();
		}
		return null;
	});

	const bodyFacts = $derived(
		flatFacts.filter(
			(f) => f.predicate !== 'exom/summary' && f.predicate !== 'exom/description'
		)
	);

	const entityGroups = $derived.by(() => {
		const m = new Map<string, FactEntry[]>();
		for (const f of bodyFacts) {
			const g = entityForFactId(factGroupKey(f));
			if (!m.has(g)) m.set(g, []);
			m.get(g)!.push(f);
		}
		return Array.from(m.entries()).sort((a, b) => a[0].localeCompare(b[0]));
	});

	let branchesLoading = $state(false);
	let branches = $state<BranchRow[]>([]);
	let branchesErr = $state<string | null>(null);
	let branchesRetry = $state(0);

	let unarchiveBusy = $state(false);

	let graphEdges = $state(0);
	let graphLoaded = $state(false);
	let listFactsMeta = $state<{ time: string; actor: string } | null>(null);

	$effect(() => {
		if (!browser) return;
		node.path;
		factsRetry;
		let c = false;
		factsLoading = true;
		factsErr = null;
		exportBackupText(node.path)
			.then((t) => {
				if (c) return;
				flatFacts = parseFactsFromExport(t);
			})
			.catch((e: unknown) => {
				if (c) return;
				factsErr = e instanceof Error ? e.message : 'Failed to load facts';
			})
			.finally(() => {
				if (c) return;
				factsLoading = false;
			});
		return () => {
			c = true;
		};
	});

	$effect(() => {
		if (!browser) return;
		node.path;
		branchesRetry;
		let c = false;
		branchesLoading = true;
		branchesErr = null;
		fetchBranches(node.path)
			.then((r) => {
				if (!c) branches = r;
			})
			.catch((e: unknown) => {
				if (!c) branchesErr = e instanceof Error ? e.message : 'Failed to load branches';
			})
			.finally(() => {
				if (!c) branchesLoading = false;
			});
		return () => {
			c = true;
		};
	});

	$effect(() => {
		if (!browser) return;
		node.path;
		let c = false;
		graphLoaded = false;
		fetchRelationGraph(node.path)
			.then((g) => {
				if (c) return;
				graphEdges = g?.summary?.edge_count ?? 0;
				graphLoaded = true;
			})
			.catch(() => {
				if (c) return;
				graphEdges = 0;
				graphLoaded = true;
			});
		return () => {
			c = true;
		};
	});

	$effect(() => {
		if (!browser) return;
		node.path;
		let c = false;
		fetchFactsList(node.path)
			.then((rows) => {
				if (c) return;
				let best: { time: string; actor: string } | null = null;
				for (const r of rows) {
					const t = r.tx_time ?? r.valid_from;
					if (!t) continue;
					if (!best || t > best.time) {
						best = { time: t, actor: r.actor || '—' };
					}
				}
				listFactsMeta = best;
			})
			.catch(() => {
				if (c) return;
				listFactsMeta = null;
			});
		return () => {
			c = true;
		};
	});

	const statsTail = $derived.by(() => {
		const t = listFactsMeta?.time ?? node.last_tx ?? null;
		if (!t) return '';
		const rel = formatRelativeTime(t);
		const act = listFactsMeta?.actor;
		if (act && act !== '—') {
			return `last change ${rel} by ${act}`;
		}
		return `last change ${rel}`;
	});

	function isBranchLoneMain(list: BranchRow[]): boolean {
		if (list.length === 0) return true;
		if (list.length > 1) return false;
		return list[0].name === 'main' || list[0].is_current;
	}

	const kindLabel = $derived(
		node.exom_kind === 'project-main'
			? 'project-main'
			: node.exom_kind === 'session'
				? 'session'
				: node.exom_kind === 'bare'
					? 'bare'
					: node.exom_kind
	);

		function onUnarchive() {
		actorPrompt.run(async () => {
			unarchiveBusy = true;
			try {
				await unarchiveSessionExom(node.path);
				toast.success('Session unarchived');
				await invalidateAll();
			} catch (e) {
				toast.error(e instanceof Error ? e.message : 'Unarchive failed');
			} finally {
				unarchiveBusy = false;
			}
		});
	}

	let forkBusy = $state(false);

	const canFork = $derived.by(() => {
		const me = auth.user?.email;
		if (!me) return false;
		if (node.exom_kind === 'session') return false;
		if (!node.created_by) return false;
		if (node.created_by === me) return false;
		return true;
	});

	async function onFork() {
		if (!canFork || forkBusy) return;
		forkBusy = true;
		try {
			const r = await forkExom(node.path);
			toast.success(`Forked to ${r.target}`);
			await goto(`${base}/tree/${r.target}`);
		} catch (e) {
			if (e instanceof ApiActionError && e.code === 'fork_session_unsupported') {
				toast.error("Sessions can't be forked — fork the parent project instead.");
			} else {
				toast.error(e instanceof Error ? e.message : 'Fork failed');
			}
		} finally {
			forkBusy = false;
		}
	}

	const aclMode = $derived<AclMode>(node.acl_mode ?? 'solo-edit');

	const isCreator = $derived(
		auth.user?.email != null && node.created_by !== '' && auth.user.email === node.created_by
	);

	/** Mode-flip button visible only to creator on non-session exoms (Q4, Q7). */
	const canFlipMode = $derived(node.exom_kind !== 'session' && isCreator);

	let modeFlipBusy = $state(false);

	async function onFlipMode() {
		if (!canFlipMode || modeFlipBusy) return;
		const target: AclMode = aclMode === 'co-edit' ? 'solo-edit' : 'co-edit';
		const confirmMsg =
			target === 'co-edit'
				? 'Switch to co-edit?\n\nAnyone with access will be able to write the shared trunk.\nYou can switch back at any time.'
				: 'Switch to solo-edit?\n\nThe trunk re-claims to you. Other contributors keep read access (and any non-main branches they own) but lose write rights on main.';
		if (typeof window !== 'undefined' && !window.confirm(confirmMsg)) return;
		modeFlipBusy = true;
		try {
			const r = await setExomMode(node.path, target);
			toast.success(`Switched to ${r.mode}`);
			await invalidateAll();
		} catch (e) {
			toast.error(e instanceof Error ? e.message : 'Mode switch failed');
		} finally {
			modeFlipBusy = false;
		}
	}

	const modeStripBlurb = $derived.by(() => {
		if (aclMode === 'co-edit') {
			return isCreator
				? 'co-edit · anyone with access can write the shared trunk'
				: 'co-edit · you can write to the shared trunk';
		}
		if (isCreator) return 'solo-edit · only you write the trunk';
		const owner = node.created_by || 'the creator';
		return `solo-edit · only ${owner} writes the trunk`;
	});
</script>

<div
	class="flex flex-col gap-4"
	class:opacity-60={contentDimmed}
	data-read-only={readOnly ? 'true' : undefined}
>
	<header class="space-y-3">
		<div class="flex flex-wrap items-center justify-between gap-2">
			<div class="flex flex-wrap items-center gap-2 text-xs">
				<Badge variant="secondary" class="text-[10px] capitalize text-foreground">{kindLabel}</Badge>
				{#if aclMode === 'co-edit'}
					<Badge
						variant="outline"
						class="border-primary/40 bg-primary/10 text-[10px] uppercase tracking-wide text-primary"
						title="co-edit · anyone with access writes the shared trunk"
					>co-edit</Badge>
				{/if}
				{#if node.archived}
					<Badge variant="outline" class="border-primary/40 text-primary">archived</Badge>
				{/if}
				{#if node.closed}
					<Badge variant="outline" class="border-destructive/50 text-destructive">closed</Badge>
				{/if}
			</div>
			{#if canFork || showUnarchive || canFlipMode}
				<div class="flex items-center gap-2">
					{#if canFlipMode}
						<Button
							size="sm"
							variant="outline"
							disabled={modeFlipBusy}
							onclick={() => void onFlipMode()}
							title={aclMode === 'co-edit'
								? 'Switch to solo-edit (only you write main)'
								: 'Switch to co-edit (anyone with access writes main)'}
						>
							{#if modeFlipBusy}
								<Loader2 class="mr-1 size-3 animate-spin" />
							{/if}
							Switch to {aclMode === 'co-edit' ? 'solo-edit' : 'co-edit'}
						</Button>
					{/if}
					{#if canFork}
						<Button
							size="sm"
							variant="secondary"
							disabled={forkBusy}
							onclick={() => void onFork()}
							title="Fork into your namespace"
						>
							{#if forkBusy}
								<Loader2 class="mr-1 size-3 animate-spin" />
							{:else}
								<GitFork class="mr-1 size-3" />
							{/if}
							Fork
						</Button>
					{/if}
					{#if showUnarchive}
						<Button
							size="sm"
							variant="secondary"
							disabled={unarchiveBusy}
							onclick={() => void onUnarchive()}
						>
							{#if unarchiveBusy}
								<Loader2 class="mr-1 size-3 animate-spin" />
							{/if}
							Unarchive
						</Button>
					{/if}
				</div>
			{/if}
		</div>
		<p class="font-mono text-[11px] text-muted-foreground">
			{modeStripBlurb}
		</p>

		<div class="font-mono text-[11px] leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
			{node.path.split('/').filter(Boolean).join('/')}
		</div>
		<h1 class="font-serif text-3xl text-foreground">{titleText}</h1>
		{#if node.forked_from}
			<p class="font-mono text-[11px] text-muted-foreground">
				forked from
				<a
					href="{base}/tree/{node.forked_from.source_path}"
					class="underline-offset-2 hover:text-foreground hover:underline"
				>
					{node.forked_from.source_path}
				</a>
			</p>
		{/if}
		{#if headerBlurb}
			<p class="font-serif text-sm leading-relaxed text-muted-foreground">{headerBlurb}</p>
		{/if}
		<p class="font-mono text-[12px] text-muted-foreground">
			{node.fact_count} facts · {node.current_branch}
			{#if statsTail}· {statsTail}{/if}
		</p>
	</header>

	<main
		class="flex flex-col gap-6 md:grid md:grid-cols-[1fr_minmax(140px,160px)] md:items-start"
	>
		<article class="order-2 min-w-0 space-y-0 md:order-1 md:col-start-1 md:row-start-1">
			<NotebookSection id="facts" title="Facts">
				{#if sessionModes}
					<SessionFactsPanel exomPath={node.path} />
				{:else if factsLoading}
					<LoadingState message="Loading facts…" />
				{:else if factsErr}
					<ErrorState
						message={factsErr}
						onRetry={() => {
							factsErr = null;
							factsRetry++;
						}}
					/>
				{:else if bodyFacts.length === 0}
					<p class="font-serif text-sm text-muted-foreground">This exom has no facts yet.</p>
				{:else}
					<div class="space-y-4">
						{#each entityGroups as [ek, fgs] (ek)}
							<NotebookEntity entityKey={ek} facts={fgs} exomPath={node.path} />
						{/each}
					</div>
				{/if}
			</NotebookSection>

			<NotebookSection id="timeline" title="Timeline">
				<TimelinePanel exomPath={node.path} notebookMode />
			</NotebookSection>

			<NotebookSection id="branches" title="Branches">
				{#if branchesLoading}
					<LoadingState message="Loading branches…" />
				{:else if branchesErr}
					<ErrorState
						message={branchesErr}
						onRetry={() => {
							branchesErr = null;
							branchesRetry++;
						}}
					/>
				{:else if isBranchLoneMain(branches)}
					<p class="font-serif text-sm text-muted-foreground">
						Only <code class="font-mono text-foreground/90">main</code>. Branch from any fact id to
						explore alternatives.
					</p>
				{:else}
					<ul class="space-y-1.5 font-mono text-sm text-foreground/90">
						{#each branches as b (b.branch_id)}
							<li class="flex flex-wrap items-baseline gap-2 [overflow-wrap:anywhere]">
								<span
									class="inline-block w-2 shrink-0 text-center {b.is_current
										? 'text-branch-active'
										: 'text-transparent'}"
									aria-hidden="true"
									>●</span
								>
								<span>{b.name}</span>
								<span class="text-muted-foreground">
									{b.fact_count} facts{#if b.claimed_by} · {b.claimed_by}{/if}
								</span>
							</li>
						{/each}
					</ul>
				{/if}
			</NotebookSection>

			<NotebookSection id="rules" title="Rules">
				<RulesPanel exomPath={node.path} />
			</NotebookSection>

			<NotebookSection id="connections" title="Connections">
				{#if graphLoaded && graphEdges === 0}
					<p class="mb-3 font-serif text-sm text-muted-foreground">
						No outgoing relations yet — predicates like <code class="font-mono text-foreground/80"
							>operates_on</code
						>, <code class="font-mono text-foreground/80">lowers_to</code>, or any value pointing at
						another fact id will appear here.
					</p>
				{/if}
				<GraphPanel exomPath={node.path} />
			</NotebookSection>
		</article>

		<aside
			class="order-1 w-full self-start max-md:max-w-md md:order-2 md:col-start-2 md:row-start-1 md:sticky md:top-4"
		>
			<RightRailAnchors sections={railSections} />
		</aside>
	</main>
</div>
