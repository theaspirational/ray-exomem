<script lang="ts">
	import { browser } from '$app/environment';
	import { base } from '$app/paths';
	import { page } from '$app/state';
	import { ChevronRight, Key, LogOut, MoreHorizontal, Shield, User } from '@lucide/svelte';
	import { auth } from '$lib/auth.svelte';
	import { welcomeSheetState } from '$lib/Welcome/welcomeSheetState.svelte';

	let overflowOpen = $state(false);
	let userMenuOpen = $state(false);

	const userInitial = $derived(
		auth.user?.display_name?.charAt(0).toUpperCase() ?? auth.user?.email?.charAt(0).toUpperCase() ?? '?'
	);

	const crumbs = $derived.by(() => {
		let pathname = String(page.url.pathname);
		if (base && pathname.startsWith(base)) {
			pathname = pathname.slice(base.length) || '/';
		}
		if (!pathname.startsWith('/tree')) {
			return [{ label: 'tree', href: `${base}/tree/` }];
		}
		const rest = pathname.slice('/tree'.length).replace(/^\/+/, '');
		if (!rest) return [{ label: 'tree', href: `${base}/tree/` }];
		const segments = rest.split('/').filter(Boolean);
		return [
			{ label: 'tree', href: `${base}/tree/` },
			...segments.map((seg, i) => ({
				label: seg,
				href: `${base}/tree/${segments.slice(0, i + 1).join('/')}`
			}))
		];
	});

	$effect(() => {
		if (!overflowOpen || !browser) return;
		const handler = () => { overflowOpen = false; };
		setTimeout(() => document.addEventListener('click', handler, { once: true }), 0);
		return () => document.removeEventListener('click', handler);
	});

	$effect(() => {
		if (!userMenuOpen || !browser) return;
		const handler = () => { userMenuOpen = false; };
		setTimeout(() => document.addEventListener('click', handler, { once: true }), 0);
		return () => document.removeEventListener('click', handler);
	});
</script>

<header
	class="sticky top-0 z-20 flex h-11 shrink-0 items-center gap-2 border-b border-border bg-background px-3 font-sans text-foreground"
>
	<a
		href="{base}/"
		class="shrink-0 font-serif text-sm tracking-tight text-foreground hover:text-primary"
		title="ray-exomem · welcome"
	>ray-exomem</a>

	<div class="relative">
		<button
			type="button"
			class="flex size-6 items-center justify-center rounded text-muted-foreground hover:bg-secondary hover:text-foreground"
			onclick={(e) => { e.stopPropagation(); overflowOpen = !overflowOpen; }}
			aria-label="Navigate path"
		>
			<MoreHorizontal class="size-4" />
		</button>
		{#if overflowOpen}
			<div class="absolute left-0 top-full z-30 mt-1 min-w-[12rem] rounded-md border border-border bg-card py-1 shadow-lg">
				{#each crumbs as c (c.href)}
					<a
						href={c.href}
						class="block px-3 py-1.5 text-xs text-foreground/80 hover:bg-secondary hover:text-foreground"
						onclick={() => { overflowOpen = false; }}
					>{c.label}</a>
				{/each}
			</div>
		{/if}
	</div>

	<nav
		class="flex min-w-0 flex-1 items-center gap-1 overflow-hidden text-xs"
		aria-label="Path breadcrumb"
		title={crumbs.map(c => c.label).join(' / ')}
	>
		{#each crumbs as crumb, i (crumb.href)}
			{#if i > 0}
				<ChevronRight class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
			{/if}
			{@const isLast = i === crumbs.length - 1}
			<a
				href={crumb.href}
				class="min-w-0 shrink truncate font-mono text-[11px] underline-offset-2 hover:underline {isLast ? 'text-foreground' : 'text-foreground/60'}"
			>{crumb.label}</a>
		{/each}
	</nav>

	<div class="relative shrink-0">
		<button
			type="button"
			class="flex size-7 items-center justify-center rounded-full bg-secondary text-xs font-medium text-foreground hover:bg-muted"
			onclick={(e) => { e.stopPropagation(); userMenuOpen = !userMenuOpen; }}
			aria-label="User menu"
			title={auth.user?.display_name ?? auth.user?.email ?? 'User'}
		>
			{userInitial}
		</button>
		{#if userMenuOpen}
			<div class="absolute right-0 top-full z-30 mt-1 min-w-[13rem] rounded-md border border-border bg-card py-1 shadow-lg">
				{#if auth.user?.email}
					<div class="px-3 py-1.5 text-[11px] text-muted-foreground truncate" title={auth.user.email}>
						{auth.user.email}
					</div>
				{/if}
				<button
					type="button"
					class="flex w-full items-center gap-2 px-3 py-1.5 text-xs text-foreground/80 hover:bg-secondary hover:text-foreground"
					onclick={() => {
						userMenuOpen = false;
						welcomeSheetState.openSheet();
					}}
				>
					<Key class="size-3.5" />
					Connect an agent
				</button>
				<a
					href="{base}/profile"
					class="flex items-center gap-2 px-3 py-1.5 text-xs text-foreground/80 hover:bg-secondary hover:text-foreground"
					onclick={() => { userMenuOpen = false; }}
				>
					<User class="size-3.5" />
					Profile
				</a>
				{#if auth.isAdmin}
					<a
						href="{base}/admin"
						class="flex items-center gap-2 px-3 py-1.5 text-xs text-foreground/80 hover:bg-secondary hover:text-foreground"
						onclick={() => { userMenuOpen = false; }}
					>
						<Shield class="size-3.5" />
						Admin
					</a>
				{/if}
				<div class="my-1 border-t border-border"></div>
				<button
					type="button"
					class="flex w-full items-center gap-2 px-3 py-1.5 text-xs text-foreground/80 hover:bg-secondary hover:text-foreground"
					onclick={() => { userMenuOpen = false; auth.logout(); }}
				>
					<LogOut class="size-3.5" />
					Logout
				</button>
			</div>
		{/if}
	</div>
</header>
