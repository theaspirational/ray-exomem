<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { toast } from 'svelte-sonner';
	import {
		Loader2,
		Trash2,
		UserX,
		UserCheck,
		Plus,
		ShieldCheck,
		ShieldOff
	} from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import Input from '$lib/components/ui/input/input.svelte';
	import * as Tabs from '$lib/components/ui/tabs/index.js';
	import { auth } from '$lib/auth.svelte';
	import { getExomemBaseUrl } from '$lib/exomem.svelte';

	function authApiBase(): string {
		return getExomemBaseUrl().replace('/ray-exomem', '');
	}

	// --- Tab state ---
	let activeTab = $state('users');
	const tabs: Array<{ id: string; label: string; topAdminOnly?: boolean }> = [
		{ id: 'users', label: 'Users' },
		{ id: 'sessions', label: 'Sessions' },
		{ id: 'api-keys', label: 'API Keys' },
		{ id: 'shares', label: 'Shares' },
		{ id: 'domains', label: 'Domains' },
		{ id: 'admins', label: 'Admins', topAdminOnly: true }
	];

	const visibleTabs = $derived(tabs.filter((t) => !t.topAdminOnly || auth.isTopAdmin));

	// --- Users ---
	interface AdminUser {
		email: string;
		role: string;
		status: string;
		last_login: string | null;
	}
	let users = $state<AdminUser[]>([]);
	let usersLoading = $state(false);
	let usersError = $state<string | null>(null);
	let userActioning = $state<string | null>(null);
	let userDeleting = $state<string | null>(null);

	async function fetchUsers() {
		usersLoading = true;
		usersError = null;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/users`, { credentials: 'include' });
			if (resp.ok) {
				const body = await resp.json();
				users = body.users ?? [];
			} else {
				usersError = 'Failed to load users';
			}
		} catch {
			usersError = 'Failed to load users';
		} finally {
			usersLoading = false;
		}
	}

	async function deactivateUser(email: string) {
		userActioning = email;
		try {
			const resp = await fetch(
				`${authApiBase()}/auth/admin/users/${encodeURIComponent(email)}/deactivate`,
				{
					method: 'POST',
					credentials: 'include'
				}
			);
			if (!resp.ok) {
				toast.error('Failed to deactivate user');
				return;
			}
			toast.success(`Deactivated ${email}`);
			await fetchUsers();
		} catch {
			toast.error('Failed to deactivate user');
		} finally {
			userActioning = null;
		}
	}

	async function deleteUser(email: string) {
		if (!confirm(`Delete ${email} and permanently remove their namespace data?`)) return;
		userDeleting = email;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/users/${encodeURIComponent(email)}`, {
				method: 'DELETE',
				credentials: 'include'
			});
			if (!resp.ok) {
				const body = await resp.json().catch(() => ({}));
				toast.error(body.message || 'Failed to delete user');
				return;
			}
			toast.success(`Deleted ${email}`);
			await fetchUsers();
			if (activeTab === 'sessions') {
				await fetchSessions();
			}
			if (activeTab === 'api-keys') {
				await fetchApiKeys();
			}
			if (activeTab === 'shares') {
				await fetchShares();
			}
		} catch {
			toast.error('Failed to delete user');
		} finally {
			userDeleting = null;
		}
	}

	async function activateUser(email: string) {
		userActioning = email;
		try {
			const resp = await fetch(
				`${authApiBase()}/auth/admin/users/${encodeURIComponent(email)}/activate`,
				{
					method: 'POST',
					credentials: 'include'
				}
			);
			if (!resp.ok) {
				toast.error('Failed to activate user');
				return;
			}
			toast.success(`Activated ${email}`);
			await fetchUsers();
		} catch {
			toast.error('Failed to activate user');
		} finally {
			userActioning = null;
		}
	}

	// --- Sessions ---
	interface AdminSession {
		session_id: string;
		email: string;
		created_at: string;
	}
	let sessions = $state<AdminSession[]>([]);
	let sessionsLoading = $state(false);
	let sessionsError = $state<string | null>(null);
	let sessionKilling = $state<string | null>(null);

	async function fetchSessions() {
		sessionsLoading = true;
		sessionsError = null;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/sessions`, {
				credentials: 'include'
			});
			if (resp.ok) {
				const body = await resp.json();
				sessions = body.sessions ?? [];
			} else {
				sessionsError = 'Failed to load sessions';
			}
		} catch {
			sessionsError = 'Failed to load sessions';
		} finally {
			sessionsLoading = false;
		}
	}

	async function killSession(id: string) {
		sessionKilling = id;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/sessions/${encodeURIComponent(id)}`, {
				method: 'DELETE',
				credentials: 'include'
			});
			if (!resp.ok) {
				toast.error('Failed to kill session');
				return;
			}
			sessions = sessions.filter((s) => s.session_id !== id);
			toast.success('Session killed');
		} catch {
			toast.error('Failed to kill session');
		} finally {
			sessionKilling = null;
		}
	}

	// --- API Keys ---
	interface AdminApiKey {
		key_id: string;
		email: string;
		label: string;
		created_at: string;
	}
	let apiKeys = $state<AdminApiKey[]>([]);
	let apiKeysLoading = $state(false);
	let apiKeysError = $state<string | null>(null);
	let keyRevoking = $state<string | null>(null);

	async function fetchApiKeys() {
		apiKeysLoading = true;
		apiKeysError = null;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/api-keys`, {
				credentials: 'include'
			});
			if (resp.ok) {
				const body = await resp.json();
				apiKeys = body.keys ?? [];
			} else {
				apiKeysError = 'Failed to load API keys';
			}
		} catch {
			apiKeysError = 'Failed to load API keys';
		} finally {
			apiKeysLoading = false;
		}
	}

	async function revokeApiKey(id: string) {
		keyRevoking = id;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/api-keys/${encodeURIComponent(id)}`, {
				method: 'DELETE',
				credentials: 'include'
			});
			if (!resp.ok) {
				toast.error('Failed to revoke API key');
				return;
			}
			apiKeys = apiKeys.filter((k) => k.key_id !== id);
			toast.success('API key revoked');
		} catch {
			toast.error('Failed to revoke API key');
		} finally {
			keyRevoking = null;
		}
	}

	// --- Shares ---
	interface AdminShare {
		owner_email: string;
		path: string;
		grantee_email: string;
		permission: string;
		created_at: string;
	}
	let shares = $state<AdminShare[]>([]);
	let sharesLoading = $state(false);
	let sharesError = $state<string | null>(null);

	async function fetchShares() {
		sharesLoading = true;
		sharesError = null;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/shares`, { credentials: 'include' });
			if (resp.ok) {
				const body = await resp.json();
				shares = body.shares ?? [];
			} else {
				sharesError = 'Failed to load shares';
			}
		} catch {
			sharesError = 'Failed to load shares';
		} finally {
			sharesLoading = false;
		}
	}

	// --- Domains ---
	let domains = $state<string[]>([]);
	let domainsLoading = $state(false);
	let domainsError = $state<string | null>(null);
	let newDomain = $state('');
	let domainAdding = $state(false);
	let domainRemoving = $state<string | null>(null);

	async function fetchDomains() {
		domainsLoading = true;
		domainsError = null;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/allowed-domains`, {
				credentials: 'include'
			});
			if (resp.ok) {
				const body = await resp.json();
				domains = body.domains ?? [];
			} else {
				domainsError = 'Failed to load domains';
			}
		} catch {
			domainsError = 'Failed to load domains';
		} finally {
			domainsLoading = false;
		}
	}

	async function addDomain() {
		if (!newDomain.trim()) return;
		domainAdding = true;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/allowed-domains`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ domain: newDomain.trim() })
			});
			if (!resp.ok) {
				toast.error('Failed to add domain');
				return;
			}
			toast.success(`Added ${newDomain.trim()}`);
			newDomain = '';
			await fetchDomains();
		} catch {
			toast.error('Failed to add domain');
		} finally {
			domainAdding = false;
		}
	}

	async function removeDomain(domain: string) {
		domainRemoving = domain;
		try {
			const resp = await fetch(
				`${authApiBase()}/auth/admin/allowed-domains/${encodeURIComponent(domain)}`,
				{
					method: 'DELETE',
					credentials: 'include'
				}
			);
			if (!resp.ok) {
				toast.error('Failed to remove domain');
				return;
			}
			domains = domains.filter((d) => d !== domain);
			toast.success(`Removed ${domain}`);
		} catch {
			toast.error('Failed to remove domain');
		} finally {
			domainRemoving = null;
		}
	}

	// --- Admins ---
	let adminEmail = $state('');
	let adminGranting = $state(false);
	let adminRevoking = $state<string | null>(null);

	// Admins are derived from the users list (role === 'admin' or 'top-admin')
	const adminUsers = $derived(users.filter((u) => u.role === 'admin' || u.role === 'top-admin'));

	async function grantAdmin() {
		if (!adminEmail.trim()) return;
		adminGranting = true;
		try {
			const resp = await fetch(`${authApiBase()}/auth/admin/admins`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ email: adminEmail.trim() })
			});
			if (!resp.ok) {
				const body = await resp.json().catch(() => ({}));
				toast.error(body.message || 'Failed to grant admin');
				return;
			}
			toast.success(`Granted admin to ${adminEmail.trim()}`);
			adminEmail = '';
			await fetchUsers();
		} catch {
			toast.error('Failed to grant admin');
		} finally {
			adminGranting = false;
		}
	}

	async function revokeAdmin(email: string) {
		adminRevoking = email;
		try {
			const resp = await fetch(
				`${authApiBase()}/auth/admin/admins/${encodeURIComponent(email)}`,
				{
					method: 'DELETE',
					credentials: 'include'
				}
			);
			if (!resp.ok) {
				toast.error('Failed to revoke admin');
				return;
			}
			toast.success(`Revoked admin from ${email}`);
			await fetchUsers();
		} catch {
			toast.error('Failed to revoke admin');
		} finally {
			adminRevoking = null;
		}
	}

	// --- Lifecycle ---
	onMount(() => {
		if (!auth.isAdmin) {
			goto(`${base}/`);
			return;
		}
		loadTab(activeTab);
	});

	function loadTab(tab: string) {
		switch (tab) {
			case 'users':
				fetchUsers();
				break;
			case 'sessions':
				fetchSessions();
				break;
			case 'api-keys':
				fetchApiKeys();
				break;
			case 'shares':
				fetchShares();
				break;
			case 'domains':
				fetchDomains();
				break;
			case 'admins':
				fetchUsers();
				break;
		}
	}

	$effect(() => {
		loadTab(activeTab);
	});

	function formatDate(iso: string | null): string {
		if (!iso) return '--';
		try {
			return new Date(iso).toLocaleString(undefined, {
				year: 'numeric',
				month: 'short',
				day: 'numeric',
				hour: '2-digit',
				minute: '2-digit'
			});
		} catch {
			return iso;
		}
	}
</script>

<svelte:head>
	<title>Admin - Ray Exomem</title>
</svelte:head>

<div class="mx-auto w-full max-w-5xl space-y-6 p-6">
	<h1 class="text-lg font-semibold text-zinc-100">Admin Panel</h1>

	<Tabs.Root bind:value={activeTab}>
		<Tabs.List class="border-b border-zinc-800 bg-transparent" variant="line">
			{#each visibleTabs as tab (tab.id)}
				<Tabs.Trigger value={tab.id}>{tab.label}</Tabs.Trigger>
			{/each}
		</Tabs.List>

		<!-- Users Tab -->
		<Tabs.Content value="users" class="pt-4">
			{#if usersLoading}
				<div class="flex items-center gap-2 py-8 text-sm text-zinc-500">
					<Loader2 class="size-4 animate-spin" />
					Loading users...
				</div>
			{:else if usersError}
				<p class="py-8 text-sm text-zinc-500">{usersError}</p>
			{:else if users.length === 0}
				<div class="rounded-md border border-dashed border-zinc-800 py-8 text-center">
					<p class="text-sm text-zinc-500">No users found</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-zinc-800">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-zinc-800 text-left text-xs text-zinc-500">
								<th class="px-3 py-2 font-medium">Email</th>
								<th class="px-3 py-2 font-medium">Role</th>
								<th class="px-3 py-2 font-medium">Status</th>
								<th class="px-3 py-2 font-medium">Last Login</th>
								<th class="px-3 py-2 font-medium">Actions</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-zinc-800">
							{#each users as user (user.email)}
								<tr class="text-zinc-300">
									<td class="px-3 py-2 font-mono text-xs">{user.email}</td>
									<td class="px-3 py-2">
										<Badge
											variant={user.role === 'top-admin'
												? 'default'
												: user.role === 'admin'
													? 'secondary'
													: 'outline'}
											class="text-xs"
										>
											{user.role}
										</Badge>
									</td>
									<td class="px-3 py-2">
										<Badge
											variant={user.status === 'active' ? 'secondary' : 'destructive'}
											class="text-xs"
										>
											{user.status || 'active'}
										</Badge>
									</td>
									<td class="px-3 py-2 text-xs text-zinc-500">
										{formatDate(user.last_login)}
									</td>
									<td class="px-3 py-2">
										<div class="flex items-center gap-2">
											{#if user.status === 'deactivated'}
												<Button
													variant="ghost"
													size="sm"
													disabled={userActioning === user.email ||
														userDeleting === user.email ||
														!auth.isTopAdmin}
													onclick={() => activateUser(user.email)}
												>
													{#if userActioning === user.email}
														<Loader2 class="size-3.5 animate-spin" />
													{:else}
														<UserCheck class="size-3.5 text-green-400" />
													{/if}
													Activate
												</Button>
											{:else}
												<Button
													variant="ghost"
													size="sm"
													disabled={userActioning === user.email ||
														userDeleting === user.email ||
														user.email === auth.user?.email ||
														!auth.isTopAdmin}
													onclick={() => deactivateUser(user.email)}
												>
													{#if userActioning === user.email}
														<Loader2 class="size-3.5 animate-spin" />
													{:else}
														<UserX class="size-3.5 text-red-400" />
													{/if}
													Deactivate
												</Button>
											{/if}
											{#if auth.isTopAdmin}
												<Button
													variant="ghost"
													size="sm"
													disabled={userDeleting === user.email ||
														userActioning === user.email ||
														user.email === auth.user?.email}
													onclick={() => deleteUser(user.email)}
												>
													{#if userDeleting === user.email}
														<Loader2 class="size-3.5 animate-spin" />
													{:else}
														<Trash2 class="size-3.5 text-red-400" />
													{/if}
													Delete
												</Button>
											{/if}
										</div>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</Tabs.Content>

		<!-- Sessions Tab -->
		<Tabs.Content value="sessions" class="pt-4">
			{#if sessionsLoading}
				<div class="flex items-center gap-2 py-8 text-sm text-zinc-500">
					<Loader2 class="size-4 animate-spin" />
					Loading sessions...
				</div>
			{:else if sessionsError}
				<p class="py-8 text-sm text-zinc-500">{sessionsError}</p>
			{:else if sessions.length === 0}
				<div class="rounded-md border border-dashed border-zinc-800 py-8 text-center">
					<p class="text-sm text-zinc-500">No active sessions</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-zinc-800">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-zinc-800 text-left text-xs text-zinc-500">
								<th class="px-3 py-2 font-medium">Session ID</th>
								<th class="px-3 py-2 font-medium">User</th>
								<th class="px-3 py-2 font-medium">Created</th>
								<th class="px-3 py-2 font-medium">Actions</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-zinc-800">
							{#each sessions as session (session.session_id)}
								<tr class="text-zinc-300">
									<td class="px-3 py-2 font-mono text-xs">{session.session_id}</td>
									<td class="px-3 py-2 text-xs">{session.email}</td>
									<td class="px-3 py-2 text-xs text-zinc-500">
										{formatDate(session.created_at)}
									</td>
									<td class="px-3 py-2">
										<Button
											variant="ghost"
											size="sm"
											disabled={sessionKilling === session.session_id}
											onclick={() => killSession(session.session_id)}
										>
											{#if sessionKilling === session.session_id}
												<Loader2 class="size-3.5 animate-spin" />
											{:else}
												<Trash2 class="size-3.5 text-red-400" />
											{/if}
											Kill
										</Button>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</Tabs.Content>

		<!-- API Keys Tab -->
		<Tabs.Content value="api-keys" class="pt-4">
			{#if apiKeysLoading}
				<div class="flex items-center gap-2 py-8 text-sm text-zinc-500">
					<Loader2 class="size-4 animate-spin" />
					Loading API keys...
				</div>
			{:else if apiKeysError}
				<p class="py-8 text-sm text-zinc-500">{apiKeysError}</p>
			{:else if apiKeys.length === 0}
				<div class="rounded-md border border-dashed border-zinc-800 py-8 text-center">
					<p class="text-sm text-zinc-500">No API keys found</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-zinc-800">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-zinc-800 text-left text-xs text-zinc-500">
								<th class="px-3 py-2 font-medium">User</th>
								<th class="px-3 py-2 font-medium">Label</th>
								<th class="px-3 py-2 font-medium">Key ID</th>
								<th class="px-3 py-2 font-medium">Created</th>
								<th class="px-3 py-2 font-medium">Actions</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-zinc-800">
							{#each apiKeys as key (key.key_id)}
								<tr class="text-zinc-300">
									<td class="px-3 py-2 text-xs">{key.email}</td>
									<td class="px-3 py-2 text-xs">{key.label}</td>
									<td class="px-3 py-2 font-mono text-xs">{key.key_id}</td>
									<td class="px-3 py-2 text-xs text-zinc-500">
										{formatDate(key.created_at)}
									</td>
									<td class="px-3 py-2">
										<Button
											variant="ghost"
											size="sm"
											disabled={keyRevoking === key.key_id}
											onclick={() => revokeApiKey(key.key_id)}
										>
											{#if keyRevoking === key.key_id}
												<Loader2 class="size-3.5 animate-spin" />
											{:else}
												<Trash2 class="size-3.5 text-red-400" />
											{/if}
											Revoke
										</Button>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</Tabs.Content>

		<!-- Shares Tab -->
		<Tabs.Content value="shares" class="pt-4">
			{#if sharesLoading}
				<div class="flex items-center gap-2 py-8 text-sm text-zinc-500">
					<Loader2 class="size-4 animate-spin" />
					Loading shares...
				</div>
			{:else if sharesError}
				<p class="py-8 text-sm text-zinc-500">{sharesError}</p>
			{:else if shares.length === 0}
				<div class="rounded-md border border-dashed border-zinc-800 py-8 text-center">
					<p class="text-sm text-zinc-500">No shares found</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-zinc-800">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-zinc-800 text-left text-xs text-zinc-500">
								<th class="px-3 py-2 font-medium">Owner</th>
								<th class="px-3 py-2 font-medium">Path</th>
								<th class="px-3 py-2 font-medium">Grantee</th>
								<th class="px-3 py-2 font-medium">Permission</th>
								<th class="px-3 py-2 font-medium">Created</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-zinc-800">
							{#each shares as share, i (i)}
								<tr class="text-zinc-300">
									<td class="px-3 py-2 text-xs">{share.owner_email}</td>
									<td class="px-3 py-2 font-mono text-xs">{share.path}</td>
									<td class="px-3 py-2 text-xs">{share.grantee_email}</td>
									<td class="px-3 py-2">
										<Badge variant="outline" class="text-xs">{share.permission}</Badge>
									</td>
									<td class="px-3 py-2 text-xs text-zinc-500">
										{formatDate(share.created_at)}
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</Tabs.Content>

		<!-- Domains Tab -->
		<Tabs.Content value="domains" class="pt-4">
			<div class="space-y-4">
				<form
					onsubmit={(e) => {
						e.preventDefault();
						addDomain();
					}}
					class="flex items-center gap-2"
				>
					<Input
						bind:value={newDomain}
						placeholder="example.com"
						disabled={domainAdding}
						class="max-w-xs border-zinc-700 bg-zinc-950 text-sm"
					/>
					<Button type="submit" size="sm" disabled={domainAdding || !newDomain.trim()}>
						{#if domainAdding}
							<Loader2 class="size-3.5 animate-spin" />
						{:else}
							<Plus class="size-3.5" />
						{/if}
						Add
					</Button>
				</form>

				{#if domainsLoading}
					<div class="flex items-center gap-2 py-8 text-sm text-zinc-500">
						<Loader2 class="size-4 animate-spin" />
						Loading domains...
					</div>
				{:else if domainsError}
					<p class="py-8 text-sm text-zinc-500">{domainsError}</p>
				{:else if domains.length === 0}
					<div class="rounded-md border border-dashed border-zinc-800 py-8 text-center">
						<p class="text-sm text-zinc-500">No allowed domains configured</p>
					</div>
				{:else}
					<div class="divide-y divide-zinc-800 rounded-md border border-zinc-800">
						{#each domains as domain (domain)}
							<div class="flex items-center justify-between px-3 py-2.5">
								<span class="font-mono text-sm text-zinc-300">{domain}</span>
								<Button
									variant="ghost"
									size="icon-sm"
									disabled={domainRemoving === domain}
									onclick={() => removeDomain(domain)}
									title="Remove domain"
								>
									{#if domainRemoving === domain}
										<Loader2 class="size-3.5 animate-spin" />
									{:else}
										<Trash2 class="size-3.5 text-zinc-500 hover:text-red-400" />
									{/if}
								</Button>
							</div>
						{/each}
					</div>
				{/if}
			</div>
		</Tabs.Content>

		<!-- Admins Tab (top-admin only) -->
		{#if auth.isTopAdmin}
			<Tabs.Content value="admins" class="pt-4">
				<div class="space-y-4">
					<form
						onsubmit={(e) => {
							e.preventDefault();
							grantAdmin();
						}}
						class="flex items-center gap-2"
					>
						<Input
							bind:value={adminEmail}
							placeholder="user@example.com"
							type="email"
							disabled={adminGranting}
							class="max-w-xs border-zinc-700 bg-zinc-950 text-sm"
						/>
						<Button type="submit" size="sm" disabled={adminGranting || !adminEmail.trim()}>
							{#if adminGranting}
								<Loader2 class="size-3.5 animate-spin" />
							{:else}
								<ShieldCheck class="size-3.5" />
							{/if}
							Grant Admin
						</Button>
					</form>

					{#if usersLoading}
						<div class="flex items-center gap-2 py-8 text-sm text-zinc-500">
							<Loader2 class="size-4 animate-spin" />
							Loading admins...
						</div>
					{:else if adminUsers.length === 0}
						<div class="rounded-md border border-dashed border-zinc-800 py-8 text-center">
							<p class="text-sm text-zinc-500">No admin users found</p>
						</div>
					{:else}
						<div class="divide-y divide-zinc-800 rounded-md border border-zinc-800">
							{#each adminUsers as user (user.email)}
								<div class="flex items-center justify-between px-3 py-2.5">
									<div class="flex items-center gap-2">
										<span class="font-mono text-sm text-zinc-300">{user.email}</span>
										<Badge
											variant={user.role === 'top-admin' ? 'default' : 'secondary'}
											class="text-xs"
										>
											{user.role}
										</Badge>
									</div>
									{#if user.role !== 'top-admin'}
										<Button
											variant="ghost"
											size="sm"
											disabled={adminRevoking === user.email}
											onclick={() => revokeAdmin(user.email)}
										>
											{#if adminRevoking === user.email}
												<Loader2 class="size-3.5 animate-spin" />
											{:else}
												<ShieldOff class="size-3.5 text-red-400" />
											{/if}
											Revoke
										</Button>
									{/if}
								</div>
							{/each}
						</div>
					{/if}
				</div>
			</Tabs.Content>
		{/if}
	</Tabs.Root>
</div>
