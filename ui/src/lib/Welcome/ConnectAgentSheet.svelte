<script lang="ts">
	import { Copy, Key, Loader2 } from '@lucide/svelte';
	import { base } from '$app/paths';
	import { Button } from '$lib/components/ui/button/index.js';
	import {
		Sheet,
		SheetContent,
		SheetHeader,
		SheetTitle
	} from '$lib/components/ui/sheet/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { auth } from '$lib/auth.svelte';
	import { getExomemBaseUrl } from '$lib/exomem.svelte';
	import { welcomeSheetState } from '$lib/Welcome/welcomeSheetState.svelte';
	import { toast } from 'svelte-sonner';
	import { cn } from '$lib/utils.js';

	interface GeneratedKey {
		key_id: string;
		raw_key: string;
		mcp_config: Record<string, unknown> | null;
	}

	function authApiBase(): string {
		return getExomemBaseUrl();
	}

	function defaultKeyLabel(): string {
		const email = auth.user?.email?.trim();
		if (!email) return 'laptop';
		const local = email.split('@')[0] ?? 'user';
		const safe = local
			.replace(/[^a-zA-Z0-9._-]+/g, '-')
			.replace(/^-+|-+$/g, '');
		return `${(safe || 'user').toLowerCase()}-laptop`;
	}

	const VERIFY_CURL = $derived(`curl -H "Authorization: Bearer $KEY" \\
  ${authApiBase()}/api/status`);

	function mcpJson(cfg: Record<string, unknown> | null): string {
		if (cfg) return JSON.stringify(cfg, null, 2);
		const url = `${authApiBase()}/mcp`;
		return JSON.stringify(
			{
				mcpServers: {
					'ray-exomem': { url, headers: { Authorization: 'Bearer <YOUR_API_KEY>' } }
				}
			},
			null,
			2
		);
	}

	let preOpen = $state(false);
	let keyLabel = $state('');
	let generating = $state(false);
	let generatedKey = $state<GeneratedKey | null>(null);
	let snippetTab = $state<'mcp' | 'http'>('mcp');

	$effect(() => {
		const o = welcomeSheetState.open;
		if (o && !preOpen) {
			generatedKey = null;
			keyLabel = defaultKeyLabel();
			snippetTab = 'mcp';
		}
		preOpen = o;
	});

	async function generateKey() {
		if (!keyLabel.trim()) return;
		generating = true;
		try {
			const resp = await fetch(`${authApiBase()}/auth/api-keys`, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ label: keyLabel.trim() })
			});
			if (!resp.ok) {
				const body = await resp.json().catch(() => ({}));
				toast.error((body as { message?: string }).message || 'Failed to generate API key');
				return;
			}
			generatedKey = (await resp.json()) as GeneratedKey;
			toast.success('API key generated');
		} catch (e) {
			toast.error(e instanceof Error ? e.message : 'Failed to generate API key');
		} finally {
			generating = false;
		}
	}

	async function copyToClipboard(text: string, successLabel: string) {
		try {
			await navigator.clipboard.writeText(text);
			toast.success(`${successLabel} copied to clipboard`);
		} catch {
			toast.error('Failed to copy to clipboard');
		}
	}
</script>

<Sheet bind:open={welcomeSheetState.open}>
	<SheetContent
		side="right"
		showCloseButton
		class={cn(
			'!w-[min(100vw,28rem)] !max-w-[min(100vw,28rem)] sm:!max-w-[min(100vw,28rem)]',
			'border-l border-border bg-card p-0 shadow-sm'
		)}
	>
		<div class="flex max-h-svh min-h-0 flex-1 flex-col overflow-y-auto">
			<SheetHeader class="border-b border-border pr-10">
				<SheetTitle class="font-sans text-base text-foreground">Connect an agent</SheetTitle>
			</SheetHeader>

			<div class="space-y-5 p-4">
				{#if !generatedKey}
					<p class="font-sans text-sm leading-relaxed text-foreground/85">
						Your agent uses an API key to read and write this memory. Anything it writes shows up in
						this dashboard, attributed back to you.
					</p>
					<div class="space-y-2">
						<label for="connect-key-label" class="font-sans text-xs text-foreground/60"
							>Key label</label
						>
						<div class="flex flex-col gap-2 sm:flex-row sm:items-end">
							<Input
								id="connect-key-label"
								bind:value={keyLabel}
								disabled={generating}
								placeholder="you-laptop"
								class="border-border bg-background font-mono text-sm"
								autocomplete="off"
							/>
							<Button
								type="button"
								variant="default"
								size="default"
								class="w-full shrink-0 sm:w-auto"
								disabled={generating || !keyLabel.trim()}
								onclick={() => void generateKey()}
							>
								{#if generating}
									<Loader2 class="size-4 animate-spin" />
									Generating…
								{:else}
									<Key class="size-3.5" />
									Generate key
								{/if}
							</Button>
						</div>
					</div>
				{:else}
					<div class="space-y-4">
						<div class="space-y-1.5">
							<span class="font-sans text-xs text-foreground/70"
								>API key (copy now — won't be shown again)</span
							>
							<div class="relative">
								<pre
									class="overflow-x-auto break-all rounded-md border border-primary/30 bg-background p-3 pr-10 font-mono text-xs text-foreground"
								>{generatedKey.raw_key}</pre>
								<Button
									variant="ghost"
									size="icon-sm"
									class="absolute top-1.5 right-1.5"
									onclick={() => void copyToClipboard(generatedKey!.raw_key, 'API key')}
									title="Copy key"
								>
									<Copy class="size-3" />
								</Button>
							</div>
						</div>

						<div class="space-y-2">
							<div class="flex gap-1 rounded-md border border-border p-0.5 font-sans text-xs">
								<button
									type="button"
									class={cn(
										'flex-1 rounded px-2 py-1.5 transition',
										snippetTab === 'mcp' ? 'bg-primary text-primary-foreground' : 'text-foreground/70'
									)}
									onclick={() => (snippetTab = 'mcp')}
								>
									MCP config
								</button>
								<button
									type="button"
									class={cn(
										'flex-1 rounded px-2 py-1.5 transition',
										snippetTab === 'http' ? 'bg-primary text-primary-foreground' : 'text-foreground/70'
									)}
									onclick={() => (snippetTab = 'http')}
								>
									HTTP
								</button>
							</div>

							<div class="relative">
								{#if snippetTab === 'mcp'}
									<pre
										class="max-h-48 overflow-auto rounded-md border border-border bg-background p-3 pr-10 font-mono text-xs text-foreground/90"
									>{mcpJson(generatedKey.mcp_config)}</pre>
									<Button
										variant="ghost"
										size="icon-sm"
										class="absolute top-1.5 right-1.5"
										onclick={() =>
											void copyToClipboard(
												mcpJson(generatedKey!.mcp_config),
												'MCP config'
											)}
										title="Copy"
									>
										<Copy class="size-3" />
									</Button>
								{:else}
									<pre
										class="max-h-48 overflow-auto rounded-md border border-border bg-background p-3 pr-10 font-mono text-xs text-foreground/90"
									>{VERIFY_CURL}</pre>
									<Button
										variant="ghost"
										size="icon-sm"
										class="absolute top-1.5 right-1.5"
										onclick={() => void copyToClipboard(VERIFY_CURL, 'cURL example')}
										title="Copy"
									>
										<Copy class="size-3" />
									</Button>
								{/if}
							</div>
						</div>

						<div class="space-y-2">
							<p class="font-sans text-xs text-foreground/70">Verify it works:</p>
							<pre
								class="overflow-x-auto rounded-md border border-border bg-background/80 p-3 font-mono text-[11px] leading-relaxed text-foreground/90"
							>{VERIFY_CURL}</pre>
						</div>

						<p class="font-sans text-sm">
							<a
								href="{base}/guide"
								class="text-primary hover:underline"
								onclick={() => (welcomeSheetState.open = false)}
							>
								Want more? See the agent guide →
							</a>
						</p>
					</div>
				{/if}
			</div>
		</div>
	</SheetContent>
</Sheet>
