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
		ShieldOff,
		AlertTriangle,
		Check,
		Copy
	} from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import Input from '$lib/components/ui/input/input.svelte';
	import * as Tabs from '$lib/components/ui/tabs/index.js';
	import { auth } from '$lib/auth.svelte';
	import { builtinViewQuery } from '$lib/builtinViewQueries';
	import { getExomemBaseUrl, factoryReset, fetchExomemSchema } from '$lib/exomem.svelte';
	import type { ExomemSchemaResponse } from '$lib/types';

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
		{ id: 'admins', label: 'Admins', topAdminOnly: true },
		{ id: 'developer', label: 'Developer', topAdminOnly: true },
		{ id: 'system', label: 'System', topAdminOnly: true }
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

	// --- Developer (builtin views) ---
	let devExomPath = $state('main');
	let devSchemaLoading = $state(false);
	let devSchema = $state<ExomemSchemaResponse | null>(null);
	let devSchemaError = $state<string | null>(null);
	let devCopied = $state<string | null>(null);
	let devCopyTimer: ReturnType<typeof setTimeout> | null = null;

	async function loadDeveloperSchema() {
		devSchemaLoading = true;
		devSchemaError = null;
		try {
			const p = devExomPath.trim() || 'main';
			devSchema = await fetchExomemSchema(p);
		} catch (e) {
			devSchema = null;
			devSchemaError = e instanceof Error ? e.message : 'Failed to load schema';
		} finally {
			devSchemaLoading = false;
		}
	}

	function copyDevSnippet(key: string, text: string) {
		void navigator.clipboard.writeText(text);
		devCopied = key;
		if (devCopyTimer) clearTimeout(devCopyTimer);
		devCopyTimer = setTimeout(() => (devCopied = null), 1600);
	}

	const devBuiltinViews = $derived(devSchema?.ontology?.builtin_views ?? []);

	// --- System / factory-reset ---
	const FACTORY_RESET_PHRASE = 'reset';
	let factoryResetArmed = $state(false);
	let factoryResetConfirm = $state('');
	let factoryResetRunning = $state(false);
	let factoryResetError = $state<string | null>(null);
	let factoryResetResult = $state<{ removed_exoms: string[] } | null>(null);

	const factoryResetReady = $derived(
		factoryResetConfirm.trim().toLowerCase() === FACTORY_RESET_PHRASE
	);

	function armFactoryReset() {
		factoryResetArmed = true;
		factoryResetConfirm = '';
		factoryResetError = null;
		factoryResetResult = null;
	}

	function cancelFactoryReset() {
		factoryResetArmed = false;
		factoryResetConfirm = '';
		factoryResetError = null;
	}

	async function runFactoryReset() {
		if (!factoryResetReady || factoryResetRunning) return;
		factoryResetRunning = true;
		factoryResetError = null;
		try {
			const res = await factoryReset();
			factoryResetResult = { removed_exoms: res.removed_exoms ?? [] };
			factoryResetArmed = false;
			factoryResetConfirm = '';
			toast.success(
				`Factory reset complete (${res.removed_exoms?.length ?? 0} exoms removed). Signing out…`
			);
			// The server cleared the session cookie and truncated auth state.
			// Drop our in-memory user record and redirect to the login page so
			// the next session starts cleanly. Any cached SPA state (selected
			// exom, share lists) gets dropped naturally on reload after login.
			auth.user = null;
			void goto(`${base}/login`, { replaceState: true });
		} catch (e) {
			const msg = e instanceof Error ? e.message : String(e);
			factoryResetError = msg;
			toast.error(msg);
		} finally {
			factoryResetRunning = false;
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
			case 'developer':
				if (auth.isTopAdmin) void loadDeveloperSchema();
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
	<h1 class="text-lg font-semibold text-foreground">Admin Panel</h1>

	<Tabs.Root bind:value={activeTab}>
		<Tabs.List class="border-b border-border/60 bg-transparent" variant="line">
			{#each visibleTabs as tab (tab.id)}
				<Tabs.Trigger value={tab.id}>{tab.label}</Tabs.Trigger>
			{/each}
		</Tabs.List>

		<!-- Users Tab -->
		<Tabs.Content value="users" class="pt-4">
			{#if usersLoading}
				<div class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
					<Loader2 class="size-4 animate-spin" />
					Loading users...
				</div>
			{:else if usersError}
				<p class="py-8 text-sm text-muted-foreground">{usersError}</p>
			{:else if users.length === 0}
				<div class="rounded-md border border-dashed border-border/60 py-8 text-center">
					<p class="text-sm text-muted-foreground">No users found</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-border/60">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-border/60 text-left text-xs text-muted-foreground">
								<th class="px-3 py-2 font-medium">Email</th>
								<th class="px-3 py-2 font-medium">Role</th>
								<th class="px-3 py-2 font-medium">Status</th>
								<th class="px-3 py-2 font-medium">Last Login</th>
								<th class="px-3 py-2 font-medium">Actions</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-border/60">
							{#each users as user (user.email)}
								<tr class="text-foreground/80">
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
									<td class="px-3 py-2 text-xs text-muted-foreground">
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
														<UserCheck class="size-3.5 text-branch-active" />
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
														<UserX class="size-3.5 text-destructive" />
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
														<Trash2 class="size-3.5 text-destructive" />
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
				<div class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
					<Loader2 class="size-4 animate-spin" />
					Loading sessions...
				</div>
			{:else if sessionsError}
				<p class="py-8 text-sm text-muted-foreground">{sessionsError}</p>
			{:else if sessions.length === 0}
				<div class="rounded-md border border-dashed border-border/60 py-8 text-center">
					<p class="text-sm text-muted-foreground">No active sessions</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-border/60">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-border/60 text-left text-xs text-muted-foreground">
								<th class="px-3 py-2 font-medium">Session ID</th>
								<th class="px-3 py-2 font-medium">User</th>
								<th class="px-3 py-2 font-medium">Created</th>
								<th class="px-3 py-2 font-medium">Actions</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-border/60">
							{#each sessions as session (session.session_id)}
								<tr class="text-foreground/80">
									<td class="px-3 py-2 font-mono text-xs">{session.session_id}</td>
									<td class="px-3 py-2 text-xs">{session.email}</td>
									<td class="px-3 py-2 text-xs text-muted-foreground">
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
												<Trash2 class="size-3.5 text-destructive" />
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
				<div class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
					<Loader2 class="size-4 animate-spin" />
					Loading API keys...
				</div>
			{:else if apiKeysError}
				<p class="py-8 text-sm text-muted-foreground">{apiKeysError}</p>
			{:else if apiKeys.length === 0}
				<div class="rounded-md border border-dashed border-border/60 py-8 text-center">
					<p class="text-sm text-muted-foreground">No API keys found</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-border/60">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-border/60 text-left text-xs text-muted-foreground">
								<th class="px-3 py-2 font-medium">User</th>
								<th class="px-3 py-2 font-medium">Label</th>
								<th class="px-3 py-2 font-medium">Key ID</th>
								<th class="px-3 py-2 font-medium">Created</th>
								<th class="px-3 py-2 font-medium">Actions</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-border/60">
							{#each apiKeys as key (key.key_id)}
								<tr class="text-foreground/80">
									<td class="px-3 py-2 text-xs">{key.email}</td>
									<td class="px-3 py-2 text-xs">{key.label}</td>
									<td class="px-3 py-2 font-mono text-xs">{key.key_id}</td>
									<td class="px-3 py-2 text-xs text-muted-foreground">
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
												<Trash2 class="size-3.5 text-destructive" />
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
				<div class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
					<Loader2 class="size-4 animate-spin" />
					Loading shares...
				</div>
			{:else if sharesError}
				<p class="py-8 text-sm text-muted-foreground">{sharesError}</p>
			{:else if shares.length === 0}
				<div class="rounded-md border border-dashed border-border/60 py-8 text-center">
					<p class="text-sm text-muted-foreground">No shares found</p>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-md border border-border/60">
					<table class="w-full text-sm">
						<thead>
							<tr class="border-b border-border/60 text-left text-xs text-muted-foreground">
								<th class="px-3 py-2 font-medium">Owner</th>
								<th class="px-3 py-2 font-medium">Path</th>
								<th class="px-3 py-2 font-medium">Grantee</th>
								<th class="px-3 py-2 font-medium">Permission</th>
								<th class="px-3 py-2 font-medium">Created</th>
							</tr>
						</thead>
						<tbody class="divide-y divide-border/60">
							{#each shares as share, i (i)}
								<tr class="text-foreground/80">
									<td class="px-3 py-2 text-xs">{share.owner_email}</td>
									<td class="px-3 py-2 font-mono text-xs">{share.path}</td>
									<td class="px-3 py-2 text-xs">{share.grantee_email}</td>
									<td class="px-3 py-2">
										<Badge variant="outline" class="text-xs">{share.permission}</Badge>
									</td>
									<td class="px-3 py-2 text-xs text-muted-foreground">
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
						class="max-w-xs border-border bg-background text-sm"
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
					<div class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
						<Loader2 class="size-4 animate-spin" />
						Loading domains...
					</div>
				{:else if domainsError}
					<p class="py-8 text-sm text-muted-foreground">{domainsError}</p>
				{:else if domains.length === 0}
					<div class="rounded-md border border-dashed border-border/60 py-8 text-center">
						<p class="text-sm text-muted-foreground">No allowed domains configured</p>
					</div>
				{:else}
					<div class="divide-y divide-border/60 rounded-md border border-border/60">
						{#each domains as domain (domain)}
							<div class="flex items-center justify-between px-3 py-2.5">
								<span class="font-mono text-sm text-foreground/80">{domain}</span>
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
										<Trash2 class="size-3.5 text-muted-foreground hover:text-destructive" />
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
							class="max-w-xs border-border bg-background text-sm"
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
						<div class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
							<Loader2 class="size-4 animate-spin" />
							Loading admins...
						</div>
					{:else if adminUsers.length === 0}
						<div class="rounded-md border border-dashed border-border/60 py-8 text-center">
							<p class="text-sm text-muted-foreground">No admin users found</p>
						</div>
					{:else}
						<div class="divide-y divide-border/60 rounded-md border border-border/60">
							{#each adminUsers as user (user.email)}
								<div class="flex items-center justify-between px-3 py-2.5">
									<div class="flex items-center gap-2">
										<span class="font-mono text-sm text-foreground/80">{user.email}</span>
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
												<ShieldOff class="size-3.5 text-destructive" />
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

		<!-- System Tab (top-admin only) -->
		{#if auth.isTopAdmin}
			<Tabs.Content value="developer" class="pt-4">
				<div class="space-y-4">
					<div class="flex flex-wrap items-end gap-2">
						<div class="min-w-0 grow">
							<p class="mb-1 text-xs text-muted-foreground">Exom (slash path, e.g. main)</p>
							<Input
								bind:value={devExomPath}
								placeholder="main"
								class="max-w-md border-border bg-background text-sm font-mono"
							/>
						</div>
						<Button
							size="sm"
							variant="secondary"
							disabled={devSchemaLoading}
							onclick={() => void loadDeveloperSchema()}
						>
							{#if devSchemaLoading}
								<Loader2 class="mr-1 size-3.5 animate-spin" />
							{/if}
							Load
						</Button>
					</div>

					{#if devSchemaLoading}
						<div class="flex items-center gap-2 py-8 text-sm text-muted-foreground">
							<Loader2 class="size-4 animate-spin" />
							Loading schema…
						</div>
					{:else if devSchemaError}
						<p class="py-4 text-sm text-destructive">{devSchemaError}</p>
					{:else if devBuiltinViews.length === 0}
						<p class="text-sm text-muted-foreground">No built-in views for this exom.</p>
					{:else}
						<div class="space-y-3">
							{#each devBuiltinViews as view, vi (`${view.name}-${vi}`)}
								<div
									class="rounded-md border border-border/60 bg-background/50 p-3"
								>
									<div class="flex items-start justify-between gap-2">
										<div class="min-w-0">
											<div class="flex flex-wrap items-center gap-2">
												<span class="font-mono text-sm text-foreground">{view.name}</span>
												<Badge variant="secondary" class="text-[10px]">arity {view.arity}</Badge>
											</div>
											<p class="mt-1 text-xs text-muted-foreground">{view.description}</p>
										</div>
										<Button
											variant="ghost"
											size="sm"
											class="shrink-0"
											onclick={() =>
												copyDevSnippet(
													`view:${view.name}`,
													builtinViewQuery(devExomPath.trim() || 'main', view)
												)}
										>
											{#if devCopied === `view:${view.name}`}
												<Check class="size-3.5" />
											{:else}
												<Copy class="size-3.5" />
											{/if}
										</Button>
									</div>
									<div class="mt-2 text-[0.7rem] text-muted-foreground">Rule</div>
									<pre
										class="mt-0.5 overflow-x-auto rounded border border-border/60 bg-background p-2 font-mono text-[11px] leading-relaxed text-foreground/60"
									>{view.rule}</pre>
									<div class="mt-2 text-[0.7rem] text-muted-foreground">Query</div>
									<pre
										class="mt-0.5 overflow-x-auto rounded border border-border/60 bg-background p-2 font-mono text-[11px] leading-relaxed text-foreground/60"
									>{builtinViewQuery(devExomPath.trim() || 'main', view)}</pre>
								</div>
							{/each}
						</div>
					{/if}
				</div>
			</Tabs.Content>

			<Tabs.Content value="system" class="pt-4">
				<div class="space-y-6">
					<div class="space-y-3 rounded-md border border-destructive/40 bg-destructive/10 p-4">
						<div class="flex items-center gap-2">
							<AlertTriangle class="size-4 text-destructive" />
							<h2 class="text-sm font-semibold text-foreground">Factory reset</h2>
						</div>
						<p class="text-sm text-foreground/60">
							Wipes every exom, fact, rule, and transaction across the entire server. Only the
							default <span class="font-mono">main</span> exom remains, empty. This cannot be undone.
						</p>

						{#if factoryResetResult}
							<div
								class="rounded-md border border-border/60 bg-background p-3 text-sm text-foreground/80"
							>
								<p>
									Factory reset complete. Removed
									<span class="font-mono text-foreground"
										>{factoryResetResult.removed_exoms.length}</span
									>
									exom{factoryResetResult.removed_exoms.length === 1 ? '' : 's'}.
								</p>
								<p class="mt-1 text-xs text-muted-foreground">Reload the page to refresh UI state.</p>
							</div>
						{/if}

						{#if factoryResetError}
							<p class="text-sm text-destructive">{factoryResetError}</p>
						{/if}

						{#if !factoryResetArmed}
							<div>
								<Button variant="destructive" size="sm" onclick={armFactoryReset}>
									<AlertTriangle class="size-3.5" />
									Factory reset
								</Button>
							</div>
						{:else}
							<div class="space-y-3">
								<p class="text-xs text-foreground/60">
									Type <span class="font-mono text-foreground">{FACTORY_RESET_PHRASE}</span> to
									confirm.
								</p>
								<div class="flex items-center gap-2">
									<Input
										bind:value={factoryResetConfirm}
										placeholder={FACTORY_RESET_PHRASE}
										disabled={factoryResetRunning}
										autocomplete="off"
										class="max-w-xs border-border bg-background text-sm"
									/>
									<Button
										variant="destructive"
										size="sm"
										disabled={!factoryResetReady || factoryResetRunning}
										onclick={runFactoryReset}
									>
										{#if factoryResetRunning}
											<Loader2 class="size-3.5 animate-spin" />
										{:else}
											<Trash2 class="size-3.5" />
										{/if}
										Yes, wipe everything
									</Button>
									<Button
										variant="ghost"
										size="sm"
										disabled={factoryResetRunning}
										onclick={cancelFactoryReset}
									>
										Cancel
									</Button>
								</div>
							</div>
						{/if}
					</div>
				</div>
			</Tabs.Content>
		{/if}
	</Tabs.Root>
</div>
