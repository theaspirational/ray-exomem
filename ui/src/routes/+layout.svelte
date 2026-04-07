<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { invalidateAll } from '$app/navigation';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import {
		Activity,
		BookOpen,
		Brain,
		Calendar,
		ChevronDown,
		Database,
		GitBranch,
		GitBranchPlus,
		Layers,
		Menu,
		Network,
		PanelLeftClose,
		PanelLeftOpen,
		Search,
		Share2,
		TreePine,
		X
	} from '@lucide/svelte';

	import { Badge } from '$lib/components/ui/badge';
	import { fetchBranches, switchBranch, type BranchRow } from '$lib/exomem.svelte';
	import { app } from '$lib/stores.svelte';

	let { children }: { children: Snippet } = $props();

	let collapsed = $state(false);
	let exomDropdownOpen = $state(false);
	let branchDropdownOpen = $state(false);
	let branches = $state<BranchRow[]>([]);
	let mobileSidebarOpen = $state(false);

	async function refreshBranches() {
		try {
			branches = await fetchBranches(app.selectedExom);
		} catch {
			branches = [];
		}
	}

	async function selectBranch(branchId: string) {
		try {
			await switchBranch(branchId, app.selectedExom);
			branchDropdownOpen = false;
			await invalidateAll();
		} catch {
			branchDropdownOpen = false;
		}
	}

	const navItems = [
		{ href: '/', label: 'Dashboard', icon: BookOpen },
		{ href: '/facts', label: 'Facts', icon: Database },
		{ href: '/branches', label: 'Branches', icon: GitBranchPlus },
		{ href: '/rules', label: 'Rules', icon: GitBranch },
		{ href: '/provenance', label: 'Provenance', icon: TreePine },
		{ href: '/graph', label: 'Graph View', icon: Share2 },
		{ href: '/exoms', label: 'Exoms', icon: Layers },
		{ href: '/timeline', label: 'Validity Timeline', icon: Calendar },
		{ href: '/query', label: 'Query Console', icon: Search }
	];

	function isActive(href: string): boolean {
		const p = page.url.pathname;
		if (href === '/') return p === base || p === `${base}/`;
		return p === `${base}${href}` || p.startsWith(`${base}${href}/`);
	}

	function closeMobileSidebar() {
		mobileSidebarOpen = false;
	}

	function formatUptime(seconds: number): string {
		if (seconds < 60) return `${seconds}s`;
		if (seconds < 3_600) return `${Math.floor(seconds / 60)}m`;
		const h = Math.floor(seconds / 3_600);
		const m = Math.floor((seconds % 3_600) / 60);
		return `${h}h ${m}m`;
	}

	onMount(() => {
		void app.refreshExoms();
		void refreshBranches();
		// Defer SSE so the first batch of JSON API calls (Memory Overview Promise.all) is not
		// competing for browser per-host connection slots with EventSource.
		const connectTimer = window.setTimeout(() => app.live.connect(), 75);
		const uptimeInterval = window.setInterval(() => void app.refreshServerUptime(), 15_000);
		return () => {
			clearTimeout(connectTimer);
			clearInterval(uptimeInterval);
			app.live.disconnect();
		};
	});

	$effect(() => {
		app.selectedExom;
		void app.refreshServerUptime();
		void refreshBranches();
	});
</script>

<svelte:head>
	<title>Ray Exomem</title>
</svelte:head>

<div class="dark flex h-dvh overflow-hidden bg-background text-foreground">
	{#if mobileSidebarOpen}
		<button
			class="fixed inset-0 z-30 bg-black/45 md:hidden"
			aria-label="Close navigation"
			onclick={closeMobileSidebar}
		></button>
	{/if}

	<!-- Sidebar -->
	<aside
		class={`fixed inset-y-0 left-0 z-40 flex h-dvh w-[min(85vw,18rem)] -translate-x-full flex-col border-r border-sidebar-border bg-sidebar transition-transform duration-200 md:static md:z-auto md:h-full md:translate-x-0 ${collapsed ? 'md:w-14' : 'md:w-56'} ${mobileSidebarOpen ? 'translate-x-0' : ''}`}
	>
		<!-- Logo area -->
		<div class="flex h-14 items-center gap-2.5 border-b border-sidebar-border px-3">
			{#if !collapsed}
				<div class="flex flex-1 items-center gap-2">
					<div
						class="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary"
						title={app.serverUptimeSec != null ? `${formatUptime(app.serverUptimeSec)} up` : undefined}
					>
						<Network class="size-4" />
					</div>
					<div class="flex min-w-0 flex-col gap-0.5">
						<span class="truncate text-sm font-semibold leading-none tracking-tight text-sidebar-foreground">Exomem</span>
						<span class="truncate text-[0.65rem] leading-none text-muted-foreground">Bitemporal knowledge store</span>
						{#if app.serverUptimeSec != null}
							<span class="flex min-w-0 items-center gap-1 text-[0.65rem] leading-none text-muted-foreground">
								<Activity class="size-3 shrink-0 text-primary" aria-hidden="true" />
								<span class="truncate tabular-nums">{formatUptime(app.serverUptimeSec)} up</span>
							</span>
						{/if}
					</div>
				</div>
			{:else}
				<div
					class="mx-auto flex size-7 items-center justify-center rounded-md bg-primary/15 text-primary"
					title={app.serverUptimeSec != null ? `${formatUptime(app.serverUptimeSec)} up` : undefined}
				>
					<Network class="size-4" />
				</div>
			{/if}
			<button
				class="ml-auto rounded-md p-1 text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground md:hidden"
				onclick={closeMobileSidebar}
				aria-label="Close sidebar"
			>
				<X class="size-4" />
			</button>
		</div>

		<!-- Exom selector -->
		{#if !collapsed}
			<div class="border-b border-sidebar-border px-3 py-2.5">
				<div class="relative">
					<button
						class="flex w-full items-center gap-2 rounded-md border border-sidebar-border bg-sidebar-accent/50 px-2.5 py-1.5 text-left text-sm transition-colors hover:bg-sidebar-accent"
						onclick={() => (exomDropdownOpen = !exomDropdownOpen)}
					>
						<Brain class="size-3.5 shrink-0 text-muted-foreground" />
						<span class="flex-1 truncate text-sm font-medium">{app.selectedExom}</span>
						<ChevronDown class="size-3 shrink-0 text-muted-foreground" />
					</button>
					{#if exomDropdownOpen}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="absolute left-0 top-full z-30 mt-1 w-full overflow-hidden rounded-lg border border-border bg-popover shadow-lg"
							onmouseleave={() => (exomDropdownOpen = false)}
						>
							{#each app.activeExoms as exom (exom.name)}
								<button
									class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm transition-colors hover:bg-muted/60 {app.selectedExom === exom.name ? 'bg-muted/40 font-medium' : ''}"
									onclick={() => {
										app.switchExom(exom.name);
										exomDropdownOpen = false;
										closeMobileSidebar();
									}}
								>
									<span class="flex-1 truncate">{exom.name}</span>
									{#if app.selectedExom === exom.name}
										<span class="size-1.5 rounded-full bg-primary"></span>
									{/if}
								</button>
							{/each}
						</div>
					{/if}
				</div>
			</div>
		{/if}

		<!-- Branch selector -->
		{#if !collapsed}
			<div class="border-b border-sidebar-border px-3 py-2.5">
				<div class="relative">
					<button
						type="button"
						class="flex w-full items-center gap-2 rounded-md border border-sidebar-border bg-sidebar-accent/50 px-2.5 py-1.5 text-left text-sm transition-colors hover:bg-sidebar-accent"
						onclick={() => (branchDropdownOpen = !branchDropdownOpen)}
					>
						<GitBranch class="size-3.5 shrink-0 text-muted-foreground" />
						<span class="flex-1 truncate font-mono text-xs font-medium">
							{branches.find((b) => b.is_current)?.branch_id ?? 'main'}
						</span>
						<ChevronDown class="size-3 shrink-0 text-muted-foreground" />
					</button>
					{#if branchDropdownOpen}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="absolute left-0 top-full z-30 mt-1 max-h-56 w-full overflow-y-auto rounded-lg border border-border bg-popover shadow-lg"
							onmouseleave={() => (branchDropdownOpen = false)}
						>
							{#each branches.filter((b) => !b.archived) as br (br.branch_id)}
								<button
									type="button"
									class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm transition-colors hover:bg-muted/60 {br.is_current
										? 'bg-muted/40 font-medium'
										: ''}"
									onclick={() => void selectBranch(br.branch_id)}
								>
									<span class="flex-1 truncate font-mono text-xs">{br.branch_id}</span>
									{#if br.is_current}
										<span class="size-1.5 shrink-0 rounded-full bg-primary"></span>
									{/if}
								</button>
							{/each}
						</div>
					{/if}
				</div>
			</div>
		{/if}

		<!-- Navigation -->
		<nav class="flex flex-1 flex-col gap-0.5 overflow-y-auto px-2 py-3 no-scrollbar">
			{#each navItems as item (item.href)}
				{@const active = isActive(item.href)}
				{@const Icon = item.icon}
				<a
					href={`${base}${item.href}`}
					class="group flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm font-medium transition-colors
						{active
							? 'bg-sidebar-accent text-sidebar-foreground'
							: 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
					title={collapsed ? item.label : undefined}
					onclick={closeMobileSidebar}
				>
					<Icon class="size-4 shrink-0 {active ? 'text-primary' : 'text-muted-foreground group-hover:text-sidebar-foreground'}" />
					{#if !collapsed}
						<span>{item.label}</span>
					{/if}
				</a>
			{/each}
		</nav>

		<!-- Connection status + collapse -->
		<div class="flex items-center justify-between border-t border-sidebar-border px-3 py-2.5">
			{#if !collapsed}
				<Badge
					variant={app.live.status === 'open' ? 'default' : app.live.status === 'connecting' ? 'secondary' : 'outline'}
					class="hidden text-[0.65rem] md:inline-flex"
				>
					<span class="mr-1 size-1.5 rounded-full {app.live.status === 'open' ? 'bg-green-400' : app.live.status === 'connecting' ? 'bg-yellow-400 animate-pulse' : 'bg-muted-foreground'}"></span>
					{app.live.status === 'open' ? 'live' : app.live.status === 'connecting' ? 'connecting' : 'offline'}
				</Badge>
			{/if}
			<div class="ml-auto flex items-center gap-1">
				<button
					class="rounded-md p-1 text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground md:hidden"
					onclick={closeMobileSidebar}
					title="Close sidebar"
				>
					<X class="size-4" />
				</button>
				<button
					class="hidden rounded-md p-1 text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground md:inline-flex"
					onclick={() => (collapsed = !collapsed)}
					title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
				>
					{#if collapsed}
						<PanelLeftOpen class="size-4" />
					{:else}
						<PanelLeftClose class="size-4" />
					{/if}
				</button>
			</div>
		</div>
	</aside>

	<!-- Main content -->
	<main class="min-w-0 flex-1 overflow-y-auto">
		<div class="sticky top-0 z-20 flex items-center gap-3 border-b border-border bg-background/95 px-4 py-3 backdrop-blur md:hidden">
			<button
				class="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
				onclick={() => (mobileSidebarOpen = true)}
				aria-label="Open navigation"
			>
				<Menu class="size-5" />
			</button>
			<div class="min-w-0 flex-1">
				<p class="truncate text-sm font-semibold leading-tight">Exomem</p>
				<p class="truncate text-xs text-muted-foreground">Bitemporal knowledge store</p>
				{#if app.serverUptimeSec != null}
					<p class="mt-0.5 flex items-center gap-1 truncate text-[0.65rem] text-muted-foreground">
						<Activity class="size-3 shrink-0 text-primary" aria-hidden="true" />
						<span class="tabular-nums">{formatUptime(app.serverUptimeSec)} up</span>
					</p>
				{/if}
			</div>
			<Badge variant="secondary" class="max-w-[40vw] truncate text-[0.65rem]">{app.selectedExom}</Badge>
		</div>
		{@render children()}
	</main>
</div>
