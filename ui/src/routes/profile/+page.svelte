<script lang="ts">
	import { onMount } from 'svelte';
	import { toast } from 'svelte-sonner';
	import { Copy, Key, LogOut, Plus, Trash2, Loader2 } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Card } from '$lib/components/ui/card/index.js';
	import { Separator } from '$lib/components/ui/separator/index.js';
	import * as Dialog from '$lib/components/ui/dialog/index.js';
	import Input from '$lib/components/ui/input/input.svelte';
	import { auth } from '$lib/auth.svelte';
	import { getExomemBaseUrl } from '$lib/exomem.svelte';

	interface ApiKey {
		key_id: string;
		label: string;
		created_at: string;
	}

	interface GeneratedKey {
		key_id: string;
		raw_key: string;
		mcp_config: Record<string, unknown>;
	}

	function authApiBase(): string {
		return getExomemBaseUrl().replace('/ray-exomem', '');
	}

	let keys = $state<ApiKey[]>([]);
	let keysLoading = $state(true);
	let keysError = $state<string | null>(null);

	let generateDialogOpen = $state(false);
	let newKeyLabel = $state('');
	let generating = $state(false);
	let generatedKey = $state<GeneratedKey | null>(null);

	let revoking = $state<string | null>(null);

	onMount(() => {
		fetchKeys();
	});

	async function fetchKeys() {
		keysLoading = true;
		keysError = null;
		try {
			const resp = await fetch(`${authApiBase()}/auth/api-keys`, {
				credentials: 'include'
			});
			if (resp.ok) {
				const body = await resp.json();
				keys = body.keys ?? [];
			} else {
				keysError = 'Failed to load API keys';
			}
		} catch {
			keysError = 'Failed to load API keys';
		} finally {
			keysLoading = false;
		}
	}

	async function generateKey() {
		if (!newKeyLabel.trim()) return;
		generating = true;
		try {
			const resp = await fetch(`${authApiBase()}/auth/api-keys`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ label: newKeyLabel.trim() })
			});
			if (!resp.ok) {
				const body = await resp.json().catch(() => ({}));
				toast.error(body.message || 'Failed to generate API key');
				return;
			}
			const result: GeneratedKey = await resp.json();
			generatedKey = result;
			await fetchKeys();
			toast.success('API key generated');
		} catch (e) {
			toast.error(e instanceof Error ? e.message : 'Failed to generate API key');
		} finally {
			generating = false;
		}
	}

	async function revokeKey(keyId: string) {
		revoking = keyId;
		try {
			const resp = await fetch(`${authApiBase()}/auth/api-keys/${keyId}`, {
				method: 'DELETE',
				credentials: 'include'
			});
			if (!resp.ok) {
				toast.error('Failed to revoke API key');
				return;
			}
			keys = keys.filter((k) => k.key_id !== keyId);
			toast.success('API key revoked');
		} catch {
			toast.error('Failed to revoke API key');
		} finally {
			revoking = null;
		}
	}

	async function copyToClipboard(text: string, label: string) {
		try {
			await navigator.clipboard.writeText(text);
			toast.success(`${label} copied to clipboard`);
		} catch {
			toast.error('Failed to copy to clipboard');
		}
	}

	function openGenerateDialog() {
		newKeyLabel = '';
		generatedKey = null;
		generateDialogOpen = true;
	}

	function closeGenerateDialog() {
		generateDialogOpen = false;
		generatedKey = null;
		newKeyLabel = '';
	}

	function mcpConfigSnippet(baseUrl?: string): string {
		const url = baseUrl || `${authApiBase()}/mcp`;
		return JSON.stringify(
			{
				mcpServers: {
					'ray-exomem': {
						url,
						headers: {
							Authorization: 'Bearer <YOUR_API_KEY>'
						}
					}
				}
			},
			null,
			2
		);
	}

	function formatDate(iso: string): string {
		try {
			return new Date(iso).toLocaleDateString(undefined, {
				year: 'numeric',
				month: 'short',
				day: 'numeric'
			});
		} catch {
			return iso;
		}
	}
</script>

<svelte:head>
	<title>Profile - Ray Exomem</title>
</svelte:head>

<div class="mx-auto w-full max-w-2xl space-y-6 p-6">
	<h1 class="text-lg font-semibold text-zinc-100">Profile</h1>

	<!-- User Info -->
	<Card class="border-zinc-800 bg-zinc-900/80">
		<div class="space-y-3">
			<h2 class="text-sm font-medium text-zinc-400">User Info</h2>
			<div class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
				<span class="text-zinc-500">Email</span>
				<span class="text-zinc-100">{auth.user?.email ?? '--'}</span>

				<span class="text-zinc-500">Display Name</span>
				<span class="text-zinc-100">{auth.user?.display_name ?? '--'}</span>

				<span class="text-zinc-500">Provider</span>
				<span class="text-zinc-100">{auth.user?.provider ?? '--'}</span>

				<span class="text-zinc-500">Role</span>
				<span class="text-zinc-100">{auth.user?.role ?? '--'}</span>
			</div>
			<Separator class="bg-zinc-800" />
			<Button variant="destructive" size="sm" onclick={() => auth.logout()}>
				<LogOut class="size-3.5" />
				Logout
			</Button>
		</div>
	</Card>

	<!-- API Keys -->
	<Card class="border-zinc-800 bg-zinc-900/80">
		<div class="space-y-4">
			<div class="flex items-center justify-between">
				<h2 class="text-sm font-medium text-zinc-400">API Keys</h2>
				<Button variant="outline" size="sm" onclick={openGenerateDialog}>
					<Plus class="size-3.5" />
					Generate New Key
				</Button>
			</div>

			{#if keysLoading}
				<div class="flex items-center gap-2 py-4 text-sm text-zinc-500">
					<Loader2 class="size-4 animate-spin" />
					Loading keys...
				</div>
			{:else if keysError}
				<p class="py-4 text-sm text-zinc-500">{keysError}</p>
			{:else if keys.length === 0}
				<div class="rounded-md border border-dashed border-zinc-800 py-6 text-center">
					<Key class="mx-auto size-6 text-zinc-600" />
					<p class="mt-2 text-sm text-zinc-500">No API keys yet</p>
					<p class="mt-1 text-xs text-zinc-600">
						Generate one to use with MCP clients or the API.
					</p>
				</div>
			{:else}
				<div class="divide-y divide-zinc-800 rounded-md border border-zinc-800">
					{#each keys as key (key.key_id)}
						<div class="flex items-center justify-between px-3 py-2.5">
							<div class="min-w-0 flex-1">
								<p class="truncate text-sm text-zinc-100">{key.label}</p>
								<p class="text-xs text-zinc-500">
									{key.key_id} &middot; Created {formatDate(key.created_at)}
								</p>
							</div>
							<Button
								variant="ghost"
								size="icon-sm"
								disabled={revoking === key.key_id}
								onclick={() => revokeKey(key.key_id)}
								title="Revoke key"
							>
								{#if revoking === key.key_id}
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
	</Card>

	<!-- MCP Configuration -->
	<Card class="border-zinc-800 bg-zinc-900/80">
		<div class="space-y-3">
			<h2 class="text-sm font-medium text-zinc-400">MCP Configuration</h2>
			<p class="text-xs text-zinc-500">
				Use this snippet in your MCP client configuration. Replace the API key placeholder with a
				generated key.
			</p>
			<div class="relative">
				<pre
					class="overflow-x-auto rounded-md border border-zinc-800 bg-zinc-950 p-3 text-xs text-zinc-300"
				>{mcpConfigSnippet()}</pre>
				<Button
					variant="ghost"
					size="icon-xs"
					class="absolute top-2 right-2"
					onclick={() => copyToClipboard(mcpConfigSnippet(), 'MCP config')}
					title="Copy config"
				>
					<Copy class="size-3" />
				</Button>
			</div>
		</div>
	</Card>
</div>

<!-- Generate Key Dialog -->
<Dialog.Root bind:open={generateDialogOpen}>
	<Dialog.Content class="border-zinc-800 bg-zinc-900 sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>
				{#if generatedKey}
					Key Generated
				{:else}
					Generate API Key
				{/if}
			</Dialog.Title>
			<Dialog.Description>
				{#if generatedKey}
					Copy this key now -- it will not be shown again.
				{:else}
					Enter a label to identify this key.
				{/if}
			</Dialog.Description>
		</Dialog.Header>

		{#if generatedKey}
			<div class="space-y-4">
				<!-- Raw key -->
				<div class="space-y-1.5">
					<span class="text-xs font-medium text-zinc-400">API Key</span>
					<div class="relative">
						<pre
							class="overflow-x-auto rounded-md border border-amber-900/50 bg-amber-950/20 p-3 pr-10 text-xs text-amber-200 break-all whitespace-pre-wrap"
						>{generatedKey.raw_key}</pre>
						<Button
							variant="ghost"
							size="icon-xs"
							class="absolute top-2 right-2"
							onclick={() => copyToClipboard(generatedKey!.raw_key, 'API key')}
							title="Copy key"
						>
							<Copy class="size-3" />
						</Button>
					</div>
					<p class="text-xs text-amber-500/80">This key is shown only once.</p>
				</div>

				<!-- MCP Config -->
				{#if generatedKey.mcp_config}
					<div class="space-y-1.5">
						<span class="text-xs font-medium text-zinc-400">MCP Config</span>
						<div class="relative">
							<pre
								class="overflow-x-auto rounded-md border border-zinc-800 bg-zinc-950 p-3 pr-10 text-xs text-zinc-300 break-all whitespace-pre-wrap"
							>{JSON.stringify(generatedKey.mcp_config, null, 2)}</pre>
							<Button
								variant="ghost"
								size="icon-xs"
								class="absolute top-2 right-2"
								onclick={() =>
									copyToClipboard(
										JSON.stringify(generatedKey!.mcp_config, null, 2),
										'MCP config'
									)}
								title="Copy config"
							>
								<Copy class="size-3" />
							</Button>
						</div>
					</div>
				{/if}
			</div>

			<Dialog.Footer>
				<Button variant="outline" onclick={closeGenerateDialog}>Done</Button>
			</Dialog.Footer>
		{:else}
			<form
				onsubmit={(e) => {
					e.preventDefault();
					generateKey();
				}}
				class="space-y-4"
			>
				<div class="space-y-1.5">
					<label for="key-label" class="text-xs font-medium text-zinc-400">Label</label>
					<Input
						id="key-label"
						bind:value={newKeyLabel}
						placeholder="e.g. claude-desktop, cursor, dev-testing"
						disabled={generating}
					/>
				</div>
				<Dialog.Footer>
					<Button variant="outline" onclick={closeGenerateDialog} disabled={generating}>
						Cancel
					</Button>
					<Button type="submit" disabled={generating || !newKeyLabel.trim()}>
						{#if generating}
							<Loader2 class="size-3.5 animate-spin" />
							Generating...
						{:else}
							<Key class="size-3.5" />
							Generate
						{/if}
					</Button>
				</Dialog.Footer>
			</form>
		{/if}
	</Dialog.Content>
</Dialog.Root>
